//! End-to-end contract tests of Web Push in `soos-remote::server::serve` (ADR 2026-10-06
//! "Web Push Notifications for Failed-Password Alerts Through a Separate Sender Unit",
//! architect spec `AI/architect_spec_remote_web_push.md` §5–§7, tests 23 and 25–36, 54;
//! matrix RMC60, RMC61, RMC65–RMC71), plus test 61 of the alerts round 3 (owner request
//! 2026-10-06 "clear acknowledged entries", matrix RMC75).
//!
//! Same deterministic harness as `alerts_server_tests.rs` (frozen paused clock, scripted
//! logind, raw HTTP/1.1 over the Unix socket in a `TempDir`, passkey store fixture for
//! Funnel sessions, [`ScriptedJournal`] injected through `ServerState::with_password_alerts`)
//! plus a [`FakeTransport`] injected through `ServerState::with_push`: no real journal, no
//! sender process, no network, no push service.
//!
//! Plan-evaluator round-2 finding encoded here: F-12 (the "new summary replaces a pending
//! retry" case is driven through a `Retry-After: 120` retry that is due later than the next
//! summary; tests 29 and 54).
//! Contract choices where the spec is silent: `ServerState::with_push(PushSettings,
//! Arc<dyn PushTransport>)` is ignored unless `config.push.enabled` (like
//! `with_password_alerts`); the disabled view reports `sender: "unknown"`; test 23 is
//! end-to-end because the spec defines the subscribe body only through route answers.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

#[path = "common/passkey.rs"]
mod passkey;

#[path = "common/harness.rs"]
mod harness;

#[path = "common/journal.rs"]
mod journal;

#[path = "common/push.rs"]
mod push;

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::oneshot;
use tokio::task::yield_now;

use harness::*;
use journal::*;
use passkey::*;
use push::*;
use soos_push_protocol::{
    encode_reply, encode_request, DeliveryReply, DeliveryRequest, Outcome, PushEndpoint, Urgency,
};
use soos_remote::alerts::AlertSettings;
use soos_remote::auth::{RandomError, RandomSource};
use soos_remote::config::{
    AlertsConfig, AuthConfig, PushConfig, PushPreviews, RemoteConfig, TailscaleLogin,
};
use soos_remote::journal::OwnerLogin;
use soos_remote::push::{
    PushSettings, PushStore, PushTransport, TransportError, UnixPushTransport,
};
use soos_remote::server::{serve, ServerState};
use soos_remote::{
    ALERTS_ACK_FILE_NAME, CREDENTIALS_FILE_NAME, DEFAULT_POLL_INTERVAL_MS, JOURNAL_IDLE_TICK_MS,
    MAX_PUSH_SUBSCRIBE_BODY_BYTES, MAX_PUSH_SUBSCRIPTIONS, PUSH_COALESCE_MS,
    PUSH_CONNECT_UNIX_TIMEOUT_MS, PUSH_EXCHANGE_TIMEOUT_MS, PUSH_MAX_PER_HOUR,
    PUSH_MIN_INTERVAL_MS, PUSH_RETRY_DELAYS_MS, PUSH_ROUTE_MIN_INTERVAL_MS, PUSH_STORE_FILE_NAME,
    PUSH_TEST_MIN_INTERVAL_MS, PUSH_TEST_TOPIC, PUSH_TOPIC, PUSH_TTL_S,
};

const IP_A: &str = "203.0.113.10";
const HOUR_US: u64 = 3_600 * SECOND_US;
const HOUR_MS: u64 = 3_600_000;
const SUBJECT: &str = "https://pc.tail1234.ts.net";

// ---------------------------------------------------------------------------------------
// Push harness
// ---------------------------------------------------------------------------------------

/// What persists across service restarts of one test.
struct Env {
    dir: TempDir,
    clock: Arc<TestClock>,
    random: RandomSource,
}

impl Env {
    fn new() -> Self {
        Self::with_random(counter_random())
    }

    fn with_random(random: RandomSource) -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            clock: TestClock::new(),
            random,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn push_store(&self) -> PathBuf {
        self.path(PUSH_STORE_FILE_NAME)
    }

    /// A regular file standing for the lock-screen program.
    fn locker(&self) -> String {
        let path = self.path("swaylock-plugin");
        if !path.exists() {
            fs::write(&path, b"#!/bin/false\n").unwrap();
        }
        path.to_string_lossy().into_owned()
    }
}

fn counter_random() -> RandomSource {
    let counter = Arc::new(AtomicU64::new(1));
    Arc::new(move |buf: &mut [u8]| -> Result<(), RandomError> {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let seed = sha256(&n.to_be_bytes());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = seed[i % 32] ^ (i / 32) as u8;
        }
        Ok(())
    })
}

fn failing_random() -> RandomSource {
    Arc::new(|_: &mut [u8]| -> Result<(), RandomError> { Err(RandomError::Failed) })
}

#[derive(Clone)]
struct PushOptions {
    push_enabled: bool,
    alerts_enabled: bool,
    previews: PushPreviews,
}

impl PushOptions {
    fn enabled() -> Self {
        Self {
            push_enabled: true,
            alerts_enabled: true,
            previews: PushPreviews::Detailed,
        }
    }
}

/// Starts `serve` with alerts (the env's locker configured) and push wired to `transport`;
/// Funnel and passkeys configured (the owner's passkey stored); unlock off.
async fn start_push(
    env: &Env,
    journal: &ScriptedJournal,
    transport: &FakeTransport,
    options: PushOptions,
) -> Harness {
    let frozen = FrozenClock::hold();
    let dir = env.dir.path().to_path_buf();
    let path = dir.join("remote.sock");
    let _ = fs::remove_file(&path);
    let store_path = dir.join(CREDENTIALS_FILE_NAME);
    let owner = Authenticator::owner();
    if !store_path.exists() {
        write_store(&store_path, &[StoredPasskey::of(&owner)]);
    }
    let alerts = AlertsConfig {
        enabled: options.alerts_enabled,
        lock_screen_programs: vec![env.locker()],
    };
    let push = PushConfig {
        enabled: options.push_enabled,
        vapid_subject: None,
        socket_path: None,
        previews: options.previews,
    };
    let config = RemoteConfig {
        allowed_logins: vec![TailscaleLogin::parse(LOGIN).unwrap()],
        socket_path: path.clone(),
        poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
        allowed_hosts: Vec::new(),
        allow_unlock: false,
        auth: AuthConfig {
            rp_id: Some(HOST.to_string()),
            allow_funnel: true,
            credentials_path: Some(store_path.clone()),
        },
        alerts: alerts.clone(),
        push,
    };
    let alert_settings = AlertSettings {
        owner_login: Some(OwnerLogin::parse(OWNER).unwrap()),
        lock_screen_programs: alerts.lock_screen_programs.clone(),
        ack_path: Some(env.path(ALERTS_ACK_FILE_NAME)),
    };
    let push_settings = PushSettings {
        store_path: env.push_store(),
        subject: SUBJECT.to_string(),
        previews: options.previews,
        rp_id: HOST.to_string(),
    };
    let source = MockSource::unlocked();
    let clock_fn = Arc::clone(&env.clock);
    let state = ServerState::new(config, UID, source.clone())
        .with_unix_clock(Arc::new(move || clock_fn.now_ms()))
        .with_credentials_path(store_path.clone())
        .with_file_owner_uid(own_uid())
        .with_random(env.random.clone())
        .with_password_alerts(alert_settings, Arc::new(journal.clone()))
        .with_push(push_settings, transport.transport());
    let listener = UnixListener::bind(&path).unwrap();
    let (tx, rx) = oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, Arc::new(state), async move {
        let _ = rx.await;
    }));
    settle().await;
    Harness {
        _dir: tempfile::tempdir().unwrap(),
        dir,
        path,
        store_path,
        owner,
        source,
        clock: Arc::clone(&env.clock),
        shutdown: Some(tx),
        server: Some(server),
        _frozen: frozen,
        secrets: Mutex::new(Vec::new()),
    }
}

/// Lets the server run many scheduler rounds without moving the clock.
async fn pump() {
    for _ in 0..POLL_ROUNDS {
        yield_now().await;
    }
}

fn now_us(h: &Harness) -> u64 {
    h.clock.now_ms() * 1000
}

fn assert_result(r: &HttpResponse, status: u16, result: &str) {
    assert_eq!(
        (r.status, r.result_or_body()),
        (status, result.to_string()),
        "expected {status} {result}"
    );
    r.assert_mandatory_headers();
}

const VIEW_KEYS: [&str; 7] = [
    "state",
    "reason",
    "public_key",
    "subscriptions",
    "devices",
    "last_delivery",
    "sender",
];

fn assert_push_view_shape(v: &Value) {
    let keys: BTreeSet<&str> = v
        .as_object()
        .unwrap_or_else(|| panic!("view is an object: {v}"))
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, VIEW_KEYS.into_iter().collect(), "{v}");
    assert_eq!(v["state"] == "unavailable", !v["reason"].is_null(), "{v}");
    assert_eq!(v["state"] == "active", !v["public_key"].is_null(), "{v}");
    for d in v["devices"].as_array().expect("devices array") {
        let keys: BTreeSet<&str> = d.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["created_unix_s", "service"].into_iter().collect(),
            "{d}"
        );
    }
    assert!(v["devices"].as_array().unwrap().len() <= MAX_PUSH_SUBSCRIPTIONS);
}

fn disabled_view() -> Value {
    json!({
        "state": "disabled",
        "reason": null,
        "public_key": null,
        "subscriptions": 0,
        "devices": [],
        "last_delivery": null,
        "sender": "unknown",
    })
}

async fn push_view_via(h: &Harness, via: &Via) -> Value {
    let r = h.get_via(via, "/api/push").await;
    assert_eq!(r.status, 200, "GET /api/push: {}", r.result_or_body());
    r.assert_mandatory_headers();
    r.assert_json_body();
    let json = r.json();
    assert_push_view_shape(&json);
    json
}

async fn push_view(h: &Harness) -> Value {
    pump().await;
    push_view_via(h, &Via::Tailnet).await
}

async fn subscribe_via(h: &Harness, via: &Via, endpoint: &str, ua: &UaFixture) -> HttpResponse {
    post_body(
        h,
        via,
        "/api/push/subscribe",
        "push-subscribe",
        &subscribe_body(endpoint, ua),
    )
    .await
}

async fn subscribe(h: &Harness, endpoint: &str, ua: &UaFixture) -> HttpResponse {
    subscribe_via(h, &Via::Tailnet, endpoint, ua).await
}

async fn unsubscribe(h: &Harness, endpoint: &str) -> HttpResponse {
    post_body(
        h,
        &Via::Tailnet,
        "/api/push/unsubscribe",
        "push-unsubscribe",
        &unsubscribe_body(endpoint),
    )
    .await
}

async fn post_body(h: &Harness, via: &Via, path: &str, action: &str, body: &str) -> HttpResponse {
    h.send(
        via,
        "POST",
        path,
        &[("X-Soos-Action", action), ("Origin", ORIGIN)],
        Some(body),
    )
    .await
}

async fn push_test_via(h: &Harness, via: &Via) -> HttpResponse {
    h.send(
        via,
        "POST",
        "/api/push/test",
        &[("X-Soos-Action", "push-test"), ("Origin", ORIGIN)],
        None,
    )
    .await
}

async fn push_test(h: &Harness) -> HttpResponse {
    push_test_via(h, &Via::Tailnet).await
}

/// Waits out the shared subscribe/unsubscribe gate.
async fn gap(h: &Harness) {
    h.advance_ms(PUSH_ROUTE_MIN_INTERVAL_MS + 1).await;
}

/// Subscribes `endpoints` (one fixture key per endpoint, seeds 1..), waiting out the gate.
async fn subscribe_all(h: &Harness, endpoints: &[String]) -> Vec<UaFixture> {
    let mut out = Vec::new();
    for (i, endpoint) in endpoints.iter().enumerate() {
        let ua = UaFixture::new(i as u8 + 1);
        let r = subscribe(h, endpoint, &ua).await;
        assert_result(&r, 200, "subscribed");
        gap(h).await;
        out.push(ua);
    }
    out
}

/// A trusted lock-screen failure of the owner: the helper check and the locker's
/// `pam_unix` line at the same journal time (one attempt, at `at_us`).
fn lock_screen_failure(env: &Env, at_us: u64) -> Vec<JLine> {
    let locker = env.locker();
    vec![
        chkpwd_line(OWNER_UID, OWNER, at_us),
        pam_unix_line("swaylock", OWNER_UID, Some(&locker), OWNER, at_us),
    ]
}

/// A trusted root-side `sudo` failure of the owner (one immediate attempt).
fn sudo_failure(at_us: u64) -> JLine {
    pam_unix_line("sudo", OWNER_UID, Some(SUDO_EXE), OWNER, at_us)
}

/// Pushes `n` live `sudo` failures now (journal times 1 ms apart, multiples of 1000 µs).
async fn sudo_burst(h: &Harness, j: &ScriptedJournal, n: u64) {
    let base = (now_us(h) / 1000) * 1000;
    for i in 0..n {
        j.push(&sudo_failure(base + i * 1000));
    }
    pump().await;
}

async fn alerts_view(h: &Harness) -> Value {
    pump().await;
    let r = h.get("/api/alerts").await;
    assert_eq!(r.status, 200);
    r.json()
}

/// Starts, waits for the follower and for alerts `active`.
async fn until_active(h: &Harness, j: &ScriptedJournal) {
    let mut elapsed = 0;
    loop {
        pump().await;
        if j.follows() >= 1 && alerts_view(h).await["state"] == "active" {
            return;
        }
        assert!(
            elapsed < 3 * JOURNAL_IDLE_TICK_MS + 1000,
            "alerts not active"
        );
        h.advance_ms(250).await;
        elapsed += 250;
    }
}

/// Moves the clock in `step` ms steps until the transport saw `n` calls; returns the
/// virtual time consumed; panics after `max_ms`.
async fn wait_calls(h: &Harness, t: &FakeTransport, n: usize, max_ms: u64, step: u64) -> u64 {
    let mut elapsed = 0;
    loop {
        pump().await;
        if t.count() >= n {
            return elapsed;
        }
        assert!(
            elapsed < max_ms,
            "{n} calls not reached within {max_ms} ms ({} so far)",
            t.count()
        );
        h.advance_ms(step).await;
        elapsed += step;
    }
}

/// Advances `total` ms in `step` ms steps.
async fn idle(h: &Harness, total: u64, step: u64) {
    h.step_ms(step, total).await;
    pump().await;
}

fn count_lines(text: &str, needle: &str) -> usize {
    text.lines().filter(|l| l.contains(needle)).count()
}

fn assert_audit_lines(text: &str, message: &str, level: &str) {
    for line in text.lines().filter(|l| l.contains(message)) {
        assert!(line.contains(level), "{line}");
        assert!(
            line.trim_end().ends_with(message),
            "an audit line carries no field: {line}"
        );
    }
}

fn capture_logs() -> (LogCapture, tracing::subscriber::DefaultGuard) {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (capture, guard)
}

/// The decrypted payload of `call` for the subscription keys `ua`.
fn payload_of(call: &Call, ua: &UaFixture) -> Value {
    decrypt_json(&call.request.body, ua)
}

fn wrong_of(call: &Call, ua: &UaFixture) -> u64 {
    payload_of(call, ua)["soos"]["wrong_password"]
        .as_u64()
        .unwrap()
}

fn snapshot(path: &Path) -> (Vec<u8>, u64, std::time::SystemTime) {
    let meta = fs::symlink_metadata(path).unwrap();
    (
        fs::read(path).unwrap(),
        meta.ino(),
        meta.modified().unwrap(),
    )
}

/// The detailed alert payload the phone must receive.
fn detailed_json(body: &str, w: u64, m: u64, source: &str, account: &str, last_ms: u64) -> Value {
    json!({
        "web_push": 8030,
        "notification": {
            "title": "Failed password on your PC",
            "body": body,
            "navigate": "https://pc.tail1234.ts.net/",
            "lang": "en",
        },
        "soos": {
            "v": 1,
            "kind": "alerts",
            "wrong_password": w,
            "locked_out": m,
            "source": source,
            "account": account,
            "last_unix_ms": last_ms,
        },
    })
}

fn test_json() -> Value {
    json!({
        "web_push": 8030,
        "notification": {
            "title": "soos test notification",
            "body": "Notifications from your PC work",
            "navigate": "https://pc.tail1234.ts.net/",
            "lang": "en",
        },
        "soos": {
            "v": 1,
            "kind": "test",
            "wrong_password": 0,
            "locked_out": 0,
            "source": null,
            "account": null,
            "last_unix_ms": null,
        },
    })
}

/// Starts a push-enabled service with the given subscriptions, alerts active.
async fn setup(
    endpoints: &[String],
) -> (Env, ScriptedJournal, FakeTransport, Harness, Vec<UaFixture>) {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t, PushOptions::enabled()).await;
    until_active(&h, &j).await;
    let uas = subscribe_all(&h, endpoints).await;
    (env, j, t, h, uas)
}

/// Spec §3.1 constants used by this suite.
#[test]
fn test_rwp_runtime_constants_match_the_spec() {
    assert_eq!(MAX_PUSH_SUBSCRIBE_BODY_BYTES, 2048);
    assert_eq!(PUSH_RETRY_DELAYS_MS, [5_000, 30_000]);
    assert_eq!(PUSH_TEST_MIN_INTERVAL_MS, 10_000);
    assert_eq!(PUSH_ROUTE_MIN_INTERVAL_MS, 1000);
    assert_eq!(PUSH_EXCHANGE_TIMEOUT_MS, 18_000);
    assert_eq!(PUSH_CONNECT_UNIX_TIMEOUT_MS, 1000);
    assert_eq!(PUSH_TOPIC, "soosalerts");
    assert_eq!(PUSH_TEST_TOPIC, "soostest");
}

// ---------------------------------------------------------------------------------------
// Test 25 — opt-in
// ---------------------------------------------------------------------------------------

/// Test 25 (RMC60, W-2): disabled by default (shared harness without push wiring, and with
/// a transport wired but `push_notifications` off): the disabled view, `403 push_disabled`
/// on every POST before any body byte, no store file, the transport never called, alerts
/// unaffected.
#[tokio::test(start_paused = true)]
async fn test_rwp_push_disabled_by_default() {
    assert!(!PushConfig::default().enabled);
    assert_eq!(PushConfig::default().previews, PushPreviews::Detailed);
    {
        let h = Harness::start().await;
        assert_eq!(push_view(&h).await, disabled_view());
        for (path, action) in [
            ("/api/push/subscribe", "push-subscribe"),
            ("/api/push/unsubscribe", "push-unsubscribe"),
            ("/api/push/test", "push-test"),
        ] {
            let r = h.request("POST", path, &[("X-Soos-Action", action)]).await;
            assert_result(&r, 403, "push_disabled");
        }
    }

    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(
        &env,
        &j,
        &t,
        PushOptions {
            push_enabled: false,
            ..PushOptions::enabled()
        },
    )
    .await;
    until_active(&h, &j).await;
    assert_eq!(push_view(&h).await, disabled_view());
    let head = h.request("HEAD", "/api/push", &[]).await;
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    // Answered before the body is read (a declared body is never sent).
    for (path, action) in [
        ("/api/push/subscribe", "push-subscribe"),
        ("/api/push/unsubscribe", "push-unsubscribe"),
    ] {
        let mut held = h
            .hold(
                &Via::Tailnet,
                "POST",
                path,
                &[("X-Soos-Action", action), ("Origin", ORIGIN)],
                Some(200),
            )
            .await;
        let r = held
            .response_now()
            .await
            .expect("answered without the body");
        assert_result(&r, 403, "push_disabled");
    }
    assert_result(&push_test(&h).await, 403, "push_disabled");
    // Live attempts are alerted, never pushed.
    sudo_burst(&h, &j, 3).await;
    idle(&h, 120_000, 1000).await;
    assert_eq!(alerts_view(&h).await["unacknowledged_wrong_password"], 3);
    assert_eq!(t.count(), 0, "the transport is never called");
    assert!(
        fs::symlink_metadata(env.push_store()).is_err(),
        "no store file"
    );
    assert_eq!(push_view(&h).await, disabled_view());
}

// ---------------------------------------------------------------------------------------
// Test 26 — authentication
// ---------------------------------------------------------------------------------------

/// Test 26 (RMC66, RMC73, O-4): tailnet identity or Funnel session only; a foreign identity
/// is `403 forbidden`, anonymous Funnel `403 login_required` on the four push API routes
/// (a subscribe body is never read); `/sw.js` is a public asset with the unchanged headers.
#[tokio::test(start_paused = true)]
async fn test_rwp_push_routes_require_authentication() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t, PushOptions::enabled()).await;
    let ua = ua_keys();

    // Tailnet identity.
    let v = push_view(&h).await;
    assert_eq!(v["state"], "active");
    // A foreign identity.
    for (method, path, action) in [
        ("GET", "/api/push", ""),
        ("POST", "/api/push/subscribe", "push-subscribe"),
        ("POST", "/api/push/unsubscribe", "push-unsubscribe"),
        ("POST", "/api/push/test", "push-test"),
    ] {
        let mut headers = vec![
            ("Host", HOST),
            ("Tailscale-User-Login", "intruder@example.com"),
        ];
        if !action.is_empty() {
            headers.push(("X-Soos-Action", action));
        }
        let r = h.raw(&raw_request(method, path, &headers)).await;
        assert_result(&r, 403, "forbidden");
    }
    // Anonymous Funnel.
    let anonymous = Via::funnel(IP_A);
    assert_result(
        &h.get_via(&anonymous, "/api/push").await,
        403,
        "login_required",
    );
    for (path, action) in [
        ("/api/push/subscribe", "push-subscribe"),
        ("/api/push/unsubscribe", "push-unsubscribe"),
    ] {
        let mut held = h
            .hold(
                &anonymous,
                "POST",
                path,
                &[("X-Soos-Action", action), ("Origin", ORIGIN)],
                Some(300),
            )
            .await;
        let r = held
            .response_now()
            .await
            .expect("answered before the body is sent");
        assert_result(&r, 403, "login_required");
    }
    assert_result(&push_test_via(&h, &anonymous).await, 403, "login_required");
    assert_eq!(t.count(), 0);
    assert!(
        PushStore::new(env.push_store(), own_uid())
            .load()
            .unwrap()
            .unwrap()
            .subscriptions
            .is_empty(),
        "nothing was stored"
    );

    // A Funnel session.
    let s = h.session(IP_A).await;
    let v = push_view_via(&h, &s).await;
    assert_eq!(v["state"], "active");
    assert_result(
        &subscribe_via(&h, &s, &apple_endpoint(1), &ua).await,
        200,
        "subscribed",
    );
    assert_eq!(push_view_via(&h, &s).await["subscriptions"], 1);

    // The service worker: public asset, mandatory headers, unchanged CSP.
    for via in [anonymous.clone(), Via::Tailnet] {
        let r = h.get_via(&via, "/sw.js").await;
        assert_eq!(r.status, 200);
        assert_eq!(
            r.header("content-type"),
            Some("text/javascript; charset=utf-8")
        );
        r.assert_mandatory_headers();
        assert_eq!(r.header("content-security-policy"), Some(CSP));
        let on_disk = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sw.js")).unwrap();
        assert_eq!(r.body, on_disk);
    }
    let r = h.send(&anonymous, "HEAD", "/sw.js", &[], None).await;
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty());
    let r = h.send(&Via::Tailnet, "POST", "/sw.js", &[], None).await;
    assert_eq!(r.status, 405);
    assert_eq!(r.header("allow"), Some("GET, HEAD"));
}

// ---------------------------------------------------------------------------------------
// Test 23 — subscribe body shapes (end to end)
// ---------------------------------------------------------------------------------------

/// One subscribe POST with `body`, then the gate is waited out.
async fn post_subscribe(h: &Harness, body: String) -> HttpResponse {
    let r = post_body(
        h,
        &Via::Tailnet,
        "/api/push/subscribe",
        "push-subscribe",
        &body,
    )
    .await;
    gap(h).await;
    r
}

/// Test 23 (RMC61, RMC66, §6.2 steps 4–7): the `toJSON` shapes (`expirationTime` null or a
/// number) are accepted; unknown members at either level, missing `keys`/`auth`/`endpoint`,
/// a non-string endpoint and a body over 2 KiB are refused with the specified answers.
#[tokio::test(start_paused = true)]
async fn test_rwp_subscribe_body_parsing() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t, PushOptions::enabled()).await;
    let ua = ua_keys();
    let (p, a) = (ua.p256dh_b64(), ua.auth_b64());
    let ep = apple_endpoint(1);

    let accepted = [
        format!("{{\"endpoint\":\"{ep}\",\"expirationTime\":null,\"keys\":{{\"p256dh\":\"{p}\",\"auth\":\"{a}\"}}}}"),
        format!("{{\"endpoint\":\"{ep}\",\"expirationTime\":1759786400000,\"keys\":{{\"p256dh\":\"{p}\",\"auth\":\"{a}\"}}}}"),
        format!("{{\"endpoint\":\"{ep}\",\"keys\":{{\"auth\":\"{a}\",\"p256dh\":\"{p}\"}}}}"),
    ];
    for body in accepted {
        assert_result(&post_subscribe(&h, body.clone()).await, 200, "subscribed");
    }
    // Exactly MAX_PUSH_SUBSCRIBE_BODY_BYTES (padded with JSON whitespace) is accepted.
    let base = subscribe_body(&ep, &ua);
    let mut at_bound = base.clone();
    at_bound.push_str(&" ".repeat(MAX_PUSH_SUBSCRIBE_BODY_BYTES - base.len()));
    assert_eq!(at_bound.len(), 2048);
    assert_result(&post_subscribe(&h, at_bound).await, 200, "subscribed");
    let mut over = base.clone();
    over.push_str(&" ".repeat(MAX_PUSH_SUBSCRIBE_BODY_BYTES + 1 - base.len()));
    assert_result(&post_subscribe(&h, over).await, 413, "body_too_large");

    let refused = [
        format!("{{\"endpoint\":\"{ep}\",\"expirationTime\":null,\"keys\":{{\"p256dh\":\"{p}\",\"auth\":\"{a}\"}},\"extra\":1}}"),
        format!("{{\"endpoint\":\"{ep}\",\"keys\":{{\"p256dh\":\"{p}\",\"auth\":\"{a}\",\"extra\":\"x\"}}}}"),
        format!("{{\"endpoint\":\"{ep}\",\"expirationTime\":null}}"),
        format!("{{\"endpoint\":\"{ep}\",\"keys\":{{\"p256dh\":\"{p}\"}}}}"),
        format!("{{\"keys\":{{\"p256dh\":\"{p}\",\"auth\":\"{a}\"}}}}"),
        format!("{{\"endpoint\":42,\"keys\":{{\"p256dh\":\"{p}\",\"auth\":\"{a}\"}}}}"),
        "[]".to_string(),
        "not json".to_string(),
        String::new(),
    ];
    for body in refused {
        assert_result(&post_subscribe(&h, body.clone()).await, 400, "bad_request");
    }
    assert_eq!(
        PushStore::new(env.push_store(), own_uid())
            .load()
            .unwrap()
            .unwrap()
            .subscriptions
            .len(),
        1,
        "only the accepted endpoint is stored"
    );
}

// ---------------------------------------------------------------------------------------
// Test 27 — subscribe gates and validation
// ---------------------------------------------------------------------------------------

/// Test 27 (RMC61, RMC65, RMC66, §6.2): CSRF refusals before the body; the shared 1 s gate;
/// body bound; `unsupported_push_service`; bad keys; the 5th endpoint; replacement; `0600`
/// store; one `push subscription added` audit line per added subscription only.
#[tokio::test(start_paused = true)]
async fn test_rwp_subscribe_gates_and_validation() {
    let (capture, _guard) = capture_logs();
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t, PushOptions::enabled()).await;
    let ua = ua_keys();

    // CSRF refusals are answered before any body byte.
    let csrf_cases: Vec<Vec<(&str, &str)>> = vec![
        vec![("Origin", ORIGIN)],
        vec![("X-Soos-Action", "push-test"), ("Origin", ORIGIN)],
        vec![("X-Soos-Action", "lock"), ("Origin", ORIGIN)],
        vec![("X-Soos-Action", "Push-Subscribe"), ("Origin", ORIGIN)],
        vec![
            ("X-Soos-Action", "push-subscribe"),
            ("Sec-Fetch-Site", "cross-site"),
        ],
        vec![
            ("X-Soos-Action", "push-subscribe"),
            ("Origin", "https://evil.example"),
        ],
    ];
    for headers in csrf_cases {
        let mut held = h
            .hold(
                &Via::Tailnet,
                "POST",
                "/api/push/subscribe",
                &headers,
                Some(400),
            )
            .await;
        let r = held.response_now().await.expect("refused before the body");
        assert_result(&r, 403, "forbidden");
    }

    // Accepted, then the gate.
    assert_result(
        &subscribe(&h, &apple_endpoint(1), &ua).await,
        200,
        "subscribed",
    );
    let mut held = h
        .hold(
            &Via::Tailnet,
            "POST",
            "/api/push/subscribe",
            &[("X-Soos-Action", "push-subscribe"), ("Origin", ORIGIN)],
            Some(400),
        )
        .await;
    let r = held.response_now().await.expect("gated before the body");
    assert_result(&r, 429, "rate_limited");
    gap(&h).await;

    // Body bound (after the shared 8 KiB read bound).
    let base = subscribe_body(&apple_endpoint(2), &ua);
    let over = format!(
        "{base}{}",
        " ".repeat(MAX_PUSH_SUBSCRIBE_BODY_BYTES + 1 - base.len())
    );
    let r = post_body(
        &h,
        &Via::Tailnet,
        "/api/push/subscribe",
        "push-subscribe",
        &over,
    )
    .await;
    assert_result(&r, 413, "body_too_large");
    gap(&h).await;

    // Unsupported push service and other endpoint errors.
    for (endpoint, result) in [
        ("https://push.example.com/abc", "unsupported_push_service"),
        (
            "https://wns2-by3p.notify.windows.com/w/abc",
            "unsupported_push_service",
        ),
        ("http://web.push.apple.com/abc", "bad_request"),
        ("https://web.push.apple.com:443/abc", "bad_request"),
        ("https://web.push.apple.com/a?b", "bad_request"),
    ] {
        let r = subscribe(&h, endpoint, &ua).await;
        assert_result(&r, 400, result);
        gap(&h).await;
    }
    // Bad keys.
    let bad_keys = subscribe_body(&apple_endpoint(2), &ua).replace(&ua.auth_b64(), "AAAA");
    let r = post_body(
        &h,
        &Via::Tailnet,
        "/api/push/subscribe",
        "push-subscribe",
        &bad_keys,
    )
    .await;
    assert_result(&r, 400, "bad_request");
    gap(&h).await;

    // Fill to MAX_PUSH_SUBSCRIPTIONS, then the 5th distinct endpoint.
    for endpoint in [mozilla_endpoint(2), fcm_endpoint(3), apple_endpoint(4)] {
        assert_result(&subscribe(&h, &endpoint, &ua).await, 200, "subscribed");
        gap(&h).await;
    }
    let before = fs::read(env.push_store()).unwrap();
    assert_result(
        &subscribe(&h, &apple_endpoint(5), &ua).await,
        409,
        "too_many_subscriptions",
    );
    assert_eq!(fs::read(env.push_store()).unwrap(), before);
    gap(&h).await;
    // The same endpoint again: replaced.
    let other = UaFixture::new(9);
    assert_result(
        &subscribe(&h, &apple_endpoint(1), &other).await,
        200,
        "subscribed",
    );
    let stored = PushStore::new(env.push_store(), own_uid())
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(stored.subscriptions.len(), MAX_PUSH_SUBSCRIPTIONS);
    assert!(stored.subscriptions[0].keys.p256dh == other.ua_keys().p256dh);
    assert_eq!(
        fs::metadata(env.push_store()).unwrap().permissions().mode() & 0o7777,
        0o600
    );
    assert_eq!(push_view(&h).await["subscriptions"], 4);

    let text = capture.text();
    assert_eq!(count_lines(&text, "push subscription added"), 4, "{text}");
    assert_audit_lines(&text, "push subscription added", "INFO");
    assert_eq!(count_lines(&text, "push subscription removed"), 0);
    assert_eq!(t.count(), 0, "subscribing sends nothing");
}

// ---------------------------------------------------------------------------------------
// Test 28 — live attempts
// ---------------------------------------------------------------------------------------

/// Test 28 (RMC67, RMC69, W-1, W-11, §5.6): replayed attempts are never pushed; three live
/// lock-screen failures within 1 s give exactly one notification per subscription after the
/// coalescing delay, with the exact headers, a VAPID JWT for the endpoint's origin signed
/// by the store's key, and the exact encrypted payload; `push_previews = "generic"` gives
/// the generic payload.
#[tokio::test(start_paused = true)]
async fn test_rwp_live_attempt_sends_one_coalesced_notification() {
    let endpoints = vec![apple_endpoint(1), mozilla_endpoint(2)];
    let (env, j, t, h, uas) = setup(&endpoints).await;
    let public_key = push_view(&h).await["public_key"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(public_key.len(), 87);

    // Replay: journal times before the start (and 1 h old) are alerted, never pushed.
    let start_us = now_us(&h);
    for line in lock_screen_failure(&env, start_us - HOUR_US) {
        j.push(&line);
    }
    j.push(&sudo_failure(start_us - 60 * SECOND_US - 3_600 * SECOND_US));
    idle(&h, 60_000, 500).await;
    assert_eq!(t.count(), 0, "replayed attempts are never pushed");

    // Three live lock-screen failures within 1 s.
    let base = (now_us(&h) / 1000) * 1000;
    let times = [base, base + 400_000, base + 800_000];
    for (i, at) in times.iter().enumerate() {
        for line in lock_screen_failure(&env, *at) {
            j.push(&line);
        }
        pump().await;
        if i < 2 {
            h.advance_ms(400).await;
        }
    }
    // Not before the coalescing delay (counted from the first push, the earliest note).
    h.advance_ms(PUSH_COALESCE_MS - 800 - 1).await;
    pump().await;
    assert_eq!(t.count(), 0, "nothing before {PUSH_COALESCE_MS} ms");
    wait_calls(&h, &t, 2, JOURNAL_IDLE_TICK_MS + 2000, 100).await;
    let send_s = h.now_unix_s();
    idle(&h, 120_000, 1000).await;
    let calls = t.calls();
    assert_eq!(calls.len(), 2, "exactly one per subscription");
    assert_eq!(
        alerts_view(&h).await["unacknowledged_wrong_password"],
        5,
        "two replayed attempts and three live ones are all alerted"
    );
    for (endpoint, ua) in endpoints.iter().zip(&uas) {
        let call = t.calls_to(endpoint);
        assert_eq!(call.len(), 1, "{endpoint}");
        let req = &call[0].request;
        assert_eq!(req.endpoint.as_str(), endpoint);
        assert_eq!(req.ttl_s, PUSH_TTL_S);
        assert_eq!(req.ttl_s, 43_200);
        assert_eq!(req.urgency, Urgency::High);
        assert_eq!(req.topic.as_deref(), Some("soosalerts"));
        let vapid = verify_vapid(&req.authorization, Some(&public_key));
        assert_eq!(vapid.header, "{\"typ\":\"JWT\",\"alg\":\"ES256\"}");
        let origin = PushEndpoint::parse(endpoint).unwrap().origin();
        let claims: Value = serde_json::from_str(&vapid.claims).unwrap();
        assert_eq!(claims["aud"], origin.as_str());
        assert_eq!(claims["sub"], SUBJECT);
        let exp = claims["exp"].as_u64().unwrap();
        assert!(
            (send_s - 1..=send_s).contains(&(exp - 43_200)),
            "exp = send time + 12 h: {exp} vs {send_s}"
        );
        assert_eq!(
            payload_of(&call[0], ua),
            detailed_json(
                "3 wrong passwords \u{2014} lock screen, your account",
                3,
                0,
                "lock_screen",
                "owner",
                times[2] / 1000
            )
        );
    }
    drop(h);

    // Generic previews.
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(
        &env,
        &j,
        &t,
        PushOptions {
            previews: PushPreviews::Generic,
            ..PushOptions::enabled()
        },
    )
    .await;
    until_active(&h, &j).await;
    let uas = subscribe_all(&h, &[apple_endpoint(1)]).await;
    let base = (now_us(&h) / 1000) * 1000;
    for line in lock_screen_failure(&env, base) {
        j.push(&line);
    }
    wait_calls(
        &h,
        &t,
        1,
        PUSH_COALESCE_MS + JOURNAL_IDLE_TICK_MS + 2000,
        100,
    )
    .await;
    let payload = payload_of(&t.calls()[0], &uas[0]);
    assert_eq!(
        payload,
        json!({
            "web_push": 8030,
            "notification": {
                "title": "Security alert on your PC",
                "body": "Open soos for details",
                "navigate": "https://pc.tail1234.ts.net/",
                "lang": "en",
            },
            "soos": {
                "v": 1,
                "kind": "alerts",
                "wrong_password": 1,
                "locked_out": 0,
                "source": null,
                "account": null,
                "last_unix_ms": base / 1000,
            },
        })
    );
}

// ---------------------------------------------------------------------------------------
// Test 29 — delivery outcomes
// ---------------------------------------------------------------------------------------

/// Test 29 (RMC68, W-13, §5.6, F-12): `gone` (404, 410) removes the subscription and audits
/// it; `rejected` and `refused` keep it without retry; `retry` is retried at +5 s and +30 s
/// then `failed`; `Retry-After` is honoured; an unavailable sender is reported once per
/// transition; a newer summary replaces a pending retry that is due later.
#[tokio::test(start_paused = true)]
async fn test_rwp_delivery_outcomes() {
    // Gone (404 and 410).
    for status in [404u16, 410] {
        let (capture, _guard) = capture_logs();
        let (_env, j, t, h, _uas) = setup(&[apple_endpoint(1), mozilla_endpoint(2)]).await;
        t.queue(&[reply(Outcome::Gone, Some(status), None)]);
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 2, 10_000, 100).await;
        idle(&h, 60_000, 1000).await;
        assert_eq!(t.count(), 2, "no retry of a gone subscription");
        let gone = t.calls()[0].endpoint().to_string();
        let v = push_view(&h).await;
        assert_eq!(v["subscriptions"], 1, "{v}");
        let stored = PushStore::new(h.dir.join(PUSH_STORE_FILE_NAME), own_uid())
            .load()
            .unwrap()
            .unwrap();
        assert_eq!(stored.subscriptions.len(), 1);
        assert_ne!(stored.subscriptions[0].endpoint.as_str(), gone);
        let text = capture.text();
        assert_eq!(count_lines(&text, "push subscription removed"), 1, "{text}");
        assert_audit_lines(&text, "push subscription removed", "INFO");
    }
    {
        let (_env, j, t, h, _uas) = setup(&[apple_endpoint(1)]).await;
        t.set_default(reply(Outcome::Gone, Some(410), None));
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 1, 10_000, 100).await;
        pump().await;
        let v = push_view(&h).await;
        assert_eq!(
            (v["subscriptions"].clone(), v["last_delivery"].clone()),
            (json!(0), json!("gone"))
        );
    }

    // Rejected and Refused: kept, no retry.
    for scripted in [
        reply(Outcome::Rejected, Some(403), None),
        reply(Outcome::Rejected, Some(301), None),
        reply(Outcome::Refused, None, None),
    ] {
        let (capture, _guard) = capture_logs();
        let (_env, j, t, h, _uas) = setup(&[apple_endpoint(1)]).await;
        t.set_default(scripted);
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 1, 10_000, 100).await;
        idle(&h, 120_000, 1000).await;
        assert_eq!(t.count(), 1, "{scripted:?}: no retry");
        let v = push_view(&h).await;
        assert_eq!(v["subscriptions"], 1, "kept");
        assert_eq!(v["last_delivery"], "rejected");
        assert_eq!(count_lines(&capture.text(), "push delivery failed"), 1);
        assert_audit_lines(&capture.text(), "push delivery failed", "WARN");
    }

    // Retry: +5 s, then +30 s, then failed (3 calls).
    {
        let (capture, _guard) = capture_logs();
        let (_env, j, t, h, _uas) = setup(&[apple_endpoint(1)]).await;
        t.set_default(reply(Outcome::Retry, Some(503), None));
        sudo_burst(&h, &j, 2).await;
        wait_calls(&h, &t, 1, 10_000, 100).await;
        let first = t.calls()[0].at_ms;
        h.advance_ms(PUSH_RETRY_DELAYS_MS[0] - 1).await;
        pump().await;
        assert_eq!(t.count(), 1, "not before +5 s");
        h.advance_ms(1).await;
        pump().await;
        assert_eq!(t.count(), 2);
        assert_eq!(t.calls()[1].at_ms, first + 5_000);
        h.advance_ms(PUSH_RETRY_DELAYS_MS[1] - 1).await;
        pump().await;
        assert_eq!(t.count(), 2, "not before +30 s");
        h.advance_ms(1).await;
        pump().await;
        assert_eq!(t.count(), 3);
        assert_eq!(t.calls()[2].at_ms, first + 35_000);
        idle(&h, 300_000, 1000).await;
        assert_eq!(t.count(), 3, "2 retries at most");
        assert_eq!(push_view(&h).await["last_delivery"], "failed");
        assert_eq!(count_lines(&capture.text(), "push delivery failed"), 1);
    }

    // Retry-After 120 s is honoured.
    {
        let (_env, j, t, h, _uas) = setup(&[apple_endpoint(1)]).await;
        t.queue(&[reply(Outcome::Retry, Some(429), Some(120))]);
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 1, 10_000, 100).await;
        let first = t.calls()[0].at_ms;
        h.step_ms(1000, 119_000).await;
        h.advance_ms(999).await;
        pump().await;
        assert_eq!(t.count(), 1, "not before Retry-After");
        h.advance_ms(1).await;
        pump().await;
        assert_eq!(t.count(), 2);
        assert_eq!(t.calls()[1].at_ms, first + 120_000);
        assert_eq!(push_view(&h).await["last_delivery"], "delivered");
    }

    // Sender unavailable: reported once per transition, then reachable again.
    {
        let (capture, _guard) = capture_logs();
        let (_env, j, t, h, _uas) = setup(&[apple_endpoint(1)]).await;
        t.set_default(Scripted::Fail(TransportError::Unavailable));
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 3, 60_000, 500).await;
        let v = push_view(&h).await;
        assert_eq!(v["sender"], "unavailable");
        assert_eq!(v["last_delivery"], "failed");
        let text = capture.text();
        assert_eq!(count_lines(&text, "push sender unavailable"), 1, "{text}");
        assert_audit_lines(&text, "push sender unavailable", "WARN");
        t.set_default(Scripted::Reply(DELIVERED));
        h.advance_ms(PUSH_MIN_INTERVAL_MS).await;
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 4, 60_000, 500).await;
        let v = push_view(&h).await;
        assert_eq!(v["sender"], "reachable");
        assert_eq!(v["last_delivery"], "delivered");
        // Unavailable again: a second transition, a second line.
        t.set_default(Scripted::Fail(TransportError::Unavailable));
        h.advance_ms(PUSH_MIN_INTERVAL_MS).await;
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 5, 60_000, 500).await;
        assert_eq!(count_lines(&capture.text(), "push sender unavailable"), 2);
    }
    // Timeout and Protocol errors are retried like Retry.
    for error in [TransportError::Timeout, TransportError::Protocol] {
        let (_env, j, t, h, _uas) = setup(&[apple_endpoint(1)]).await;
        t.set_default(Scripted::Fail(error));
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 3, 60_000, 500).await;
        idle(&h, 300_000, 1000).await;
        assert_eq!(t.count(), 3, "{error:?}");
        assert_eq!(push_view(&h).await["last_delivery"], "failed");
    }

    // F-12: a newer summary replaces a pending retry due later (Retry-After 120 s > 30 s).
    {
        let (_env, j, t, h, uas) = setup(&[apple_endpoint(1)]).await;
        t.queue(&[reply(Outcome::Retry, Some(429), Some(120))]);
        sudo_burst(&h, &j, 3).await;
        wait_calls(&h, &t, 1, 10_000, 100).await;
        let first = t.calls()[0].at_ms;
        h.advance_ms(1000).await;
        sudo_burst(&h, &j, 2).await;
        wait_calls(&h, &t, 2, PUSH_MIN_INTERVAL_MS + 1000, 100).await;
        let second = &t.calls()[1];
        assert!(second.at_ms < first + 120_000);
        assert!(second.at_ms >= first + PUSH_MIN_INTERVAL_MS);
        assert_eq!(
            wrong_of(second, &uas[0]),
            5,
            "the pending retry is folded in"
        );
        idle(&h, 300_000, 1000).await;
        assert_eq!(t.count(), 2, "the replaced retry is never sent");
    }
}

// ---------------------------------------------------------------------------------------
// Test 30 — rate bound
// ---------------------------------------------------------------------------------------

/// Test 30 (RMC67, W-13, O-3): 200 live attempts over 2 h: per subscription at most 20
/// alert notifications in any rolling hour, at least 30 s apart, and the decrypted counts
/// add up to 200.
#[tokio::test(start_paused = true)]
async fn test_rwp_notification_rate_is_bounded() {
    let endpoints = vec![apple_endpoint(1), mozilla_endpoint(2)];
    let (_env, j, t, h, uas) = setup(&endpoints).await;
    for _ in 0..200 {
        j.push(&sudo_failure(now_us(&h)));
        pump().await;
        h.step_ms(1000, 36_000).await;
    }
    // Let the last summaries out (at most one hour of backlog); calls are decrypted once.
    let mut waited = 0;
    let mut decoded = 0;
    let mut total = 0u64;
    loop {
        let calls = t.calls();
        for call in &calls[decoded..] {
            let ua = &uas[endpoints.iter().position(|e| e == call.endpoint()).unwrap()];
            total += wrong_of(call, ua);
        }
        decoded = calls.len();
        if total == 400 {
            break;
        }
        assert!(
            waited <= HOUR_MS + 60_000,
            "counts never reached 200 per phone: {total}"
        );
        h.advance_ms(1000).await;
        waited += 1000;
    }
    for (endpoint, ua) in endpoints.iter().zip(&uas) {
        let calls = t.calls_to(endpoint);
        assert!(calls.iter().all(|c| c.topic() == Some(PUSH_TOPIC)));
        let times: Vec<u64> = calls.iter().map(|c| c.at_ms).collect();
        for pair in times.windows(2) {
            assert!(pair[1] - pair[0] >= PUSH_MIN_INTERVAL_MS, "{times:?}");
        }
        for (i, start) in times.iter().enumerate() {
            let in_hour = times[i..]
                .iter()
                .take_while(|t| **t < start + HOUR_MS)
                .count();
            assert!(
                in_hour <= PUSH_MAX_PER_HOUR as usize,
                "{in_hour} in one hour: {times:?}"
            );
        }
        let sum: u64 = calls.iter().map(|c| wrong_of(c, ua)).sum();
        assert_eq!(sum, 200, "{endpoint}");
    }
}

// ---------------------------------------------------------------------------------------
// Test 31 — test notification
// ---------------------------------------------------------------------------------------

/// Test 31 (RMC68, §6.4, F-5): no subscription is `409`; with two, `202 test_queued` and
/// one delivery each with the test payload and `Topic: soostest`; the test gate; a body is
/// refused; a test sent while an alert retry is pending changes neither the retry nor its
/// counts.
#[tokio::test(start_paused = true)]
async fn test_rwp_test_notification() {
    let (_env, _j, t, h, _) = setup(&[]).await;
    assert_result(&push_test(&h).await, 409, "no_subscriptions");
    drop(h);
    drop(t);

    let endpoints = vec![apple_endpoint(1), mozilla_endpoint(2)];
    let (_env, _j, t, h, uas) = setup(&endpoints).await;
    let r = push_test(&h).await;
    assert_eq!(r.status, 202, "{}", r.result_or_body());
    assert_eq!(r.result(), "test_queued");
    r.assert_mandatory_headers();
    wait_calls(&h, &t, 2, 1000, 100).await;
    idle(&h, 5_000, 500).await;
    assert_eq!(t.count(), 2);
    for (endpoint, ua) in endpoints.iter().zip(&uas) {
        let calls = t.calls_to(endpoint);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].topic(), Some("soostest"));
        assert_eq!(calls[0].request.ttl_s, PUSH_TTL_S);
        assert_eq!(payload_of(&calls[0], ua), test_json());
    }
    assert_eq!(push_view(&h).await["last_delivery"], "delivered");
    // The gate.
    assert_result(&push_test(&h).await, 429, "rate_limited");
    // A body is refused by the parser.
    h.advance_ms(PUSH_TEST_MIN_INTERVAL_MS).await;
    let r = h
        .send(
            &Via::Tailnet,
            "POST",
            "/api/push/test",
            &[("X-Soos-Action", "push-test"), ("Origin", ORIGIN)],
            Some("{}"),
        )
        .await;
    assert_result(&r, 413, "body_not_allowed");
    // CSRF.
    let r = h
        .send(
            &Via::Tailnet,
            "POST",
            "/api/push/test",
            &[("X-Soos-Action", "push-subscribe")],
            None,
        )
        .await;
    assert_result(&r, 403, "forbidden");
    drop(h);

    // A test while an alert retry is pending.
    let (_env, j, t, h, uas) = setup(&[apple_endpoint(1)]).await;
    t.queue(&[reply(Outcome::Retry, Some(503), None)]);
    sudo_burst(&h, &j, 2).await;
    wait_calls(&h, &t, 1, 10_000, 100).await;
    let first = t.calls()[0].at_ms;
    h.advance_ms(1000).await;
    assert_eq!(push_test(&h).await.status, 202);
    let waited = wait_calls(&h, &t, 2, 1000, 100).await;
    assert_eq!(t.calls()[1].topic(), Some(PUSH_TEST_TOPIC));
    h.advance_ms(PUSH_RETRY_DELAYS_MS[0] - 1000 - waited - 1)
        .await;
    pump().await;
    assert_eq!(t.count(), 2);
    h.advance_ms(1).await;
    pump().await;
    assert_eq!(t.count(), 3, "the alert retry is still due at +5 s");
    let retry = &t.calls()[2];
    assert_eq!(retry.at_ms, first + 5_000);
    assert_eq!(retry.topic(), Some(PUSH_TOPIC));
    assert_eq!(wrong_of(retry, &uas[0]), 2, "counts unchanged by the test");
    idle(&h, 120_000, 1000).await;
    assert_eq!(t.count(), 3);
}

// ---------------------------------------------------------------------------------------
// Test 32 — unsubscribe
// ---------------------------------------------------------------------------------------

/// Test 32 (RMC66, §6.3): removes and audits; an absent endpoint is also `200 unsubscribed`
/// (no oracle) without an audit line; an invalid endpoint or body is `400`; the gate is
/// shared with subscribe.
#[tokio::test(start_paused = true)]
async fn test_rwp_unsubscribe() {
    let (capture, _guard) = capture_logs();
    let endpoints = vec![apple_endpoint(1), mozilla_endpoint(2)];
    let (env, _j, _t, h, _uas) = setup(&endpoints).await;
    // The gate is shared: an unsubscribe right after a subscribe is limited.
    assert_result(
        &subscribe(&h, &fcm_endpoint(3), &ua_keys()).await,
        200,
        "subscribed",
    );
    assert_result(
        &unsubscribe(&h, &fcm_endpoint(3)).await,
        429,
        "rate_limited",
    );
    gap(&h).await;

    assert_result(&unsubscribe(&h, &endpoints[0]).await, 200, "unsubscribed");
    gap(&h).await;
    let stored: Vec<String> = PushStore::new(env.push_store(), own_uid())
        .load()
        .unwrap()
        .unwrap()
        .subscriptions
        .iter()
        .map(|s| s.endpoint.as_str().to_string())
        .collect();
    assert_eq!(stored, vec![endpoints[1].clone(), fcm_endpoint(3)]);
    assert_eq!(count_lines(&capture.text(), "push subscription removed"), 1);
    assert_result(&unsubscribe(&h, &endpoints[0]).await, 200, "unsubscribed");
    gap(&h).await;
    assert_eq!(
        count_lines(&capture.text(), "push subscription removed"),
        1,
        "no audit line for an absent endpoint"
    );
    for body in [
        unsubscribe_body("http://web.push.apple.com/abc"),
        unsubscribe_body("https://evil.example/abc"),
        format!("{{\"endpoint\":\"{}\",\"x\":1}}", endpoints[1]),
        "{}".to_string(),
        "nope".to_string(),
    ] {
        let r = post_body(
            &h,
            &Via::Tailnet,
            "/api/push/unsubscribe",
            "push-unsubscribe",
            &body,
        )
        .await;
        assert_result(&r, 400, "bad_request");
        gap(&h).await;
    }
    let r = post_body(
        &h,
        &Via::Tailnet,
        "/api/push/unsubscribe",
        "push-subscribe",
        &unsubscribe_body(&endpoints[1]),
    )
    .await;
    assert_result(&r, 403, "forbidden");
    assert_eq!(push_view(&h).await["subscriptions"], 2);
}

// ---------------------------------------------------------------------------------------
// Test 33 — secrets
// ---------------------------------------------------------------------------------------

/// Reads whatever the stream has buffered without moving the clock.
async fn drain(c: &mut SseClient, into: &mut Vec<u8>) {
    into.append(&mut c.buf);
    for _ in 0..POLL_ROUNDS {
        let mut chunk = [0u8; 4096];
        match c.stream.try_read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => into.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => yield_now().await,
            Err(_) => return,
        }
    }
}

/// Test 33 (RMC69, O-2, W-14): no endpoint token, subscription key, private key, JWT,
/// ciphertext, owner login or journal user name appears in any response, SSE byte or log
/// line; the VAPID public key appears only in `GET /api/push`.
#[tokio::test(start_paused = true)]
async fn test_rwp_push_never_exposes_secrets() {
    let (capture, _guard) = capture_logs();
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t, PushOptions::enabled()).await;
    until_active(&h, &j).await;
    let mut stream = h.open_stream().await;
    let mut sse = Vec::new();
    let mut bodies: Vec<(String, Vec<u8>)> = Vec::new();
    let endpoints = [apple_endpoint(1), mozilla_endpoint(2)];
    let mut uas = Vec::new();
    for (i, endpoint) in endpoints.iter().enumerate() {
        let ua = UaFixture::new(i as u8 + 1);
        let r = subscribe(&h, endpoint, &ua).await;
        assert_eq!(r.status, 200);
        bodies.push(("subscribe".into(), r.body.clone()));
        gap(&h).await;
        uas.push(ua);
    }
    let password_like = "Hunter2-Secret!";
    j.push(&pam_unix_line(
        "sudo",
        OWNER_UID,
        Some(SUDO_EXE),
        password_like,
        now_us(&h),
    ));
    sudo_burst(&h, &j, 2).await;
    wait_calls(&h, &t, 2, 10_000, 100).await;
    drain(&mut stream, &mut sse).await;
    h.advance_ms(PUSH_TEST_MIN_INTERVAL_MS).await;
    let r = push_test(&h).await;
    bodies.push(("test".into(), r.body.clone()));
    wait_calls(&h, &t, 4, 2000, 100).await;
    let r = unsubscribe(&h, &endpoints[0]).await;
    bodies.push(("unsubscribe".into(), r.body.clone()));
    gap(&h).await;
    for target in ["/api/alerts", "/api/status"] {
        let r = h.get(target).await;
        bodies.push((target.into(), r.body.clone()));
    }
    let push_body = h.get("/api/push").await.body;
    idle(&h, 5_000, 500).await;
    drain(&mut stream, &mut sse).await;

    let store_text = fs::read_to_string(env.push_store()).unwrap();
    let store: Value = serde_json::from_str(&store_text).unwrap();
    let private = store["vapid_private_key"].as_str().unwrap().to_string();
    let public_key = push_view(&h).await["public_key"]
        .as_str()
        .unwrap()
        .to_string();
    let mut needles: Vec<String> = vec![
        private,
        LOGIN.to_string(),
        OWNER.to_string(),
        password_like.to_string(),
    ];
    for (endpoint, ua) in endpoints.iter().zip(&uas) {
        needles.push(endpoint_token(endpoint));
        needles.push(ua.p256dh_b64());
        needles.push(ua.auth_b64());
    }
    for call in t.calls() {
        let vapid = verify_vapid(&call.request.authorization, Some(&public_key));
        needles.push(vapid.jwt.clone());
        needles.push(vapid.jwt.split('.').nth(2).unwrap().to_string());
        needles.push(b64url(&call.request.body));
        needles.push(to_hex(&call.request.body[86..118]));
    }
    let logs = capture.text();
    let sse_text = String::from_utf8_lossy(&sse).into_owned();
    let push_text = String::from_utf8_lossy(&push_body).into_owned();
    for needle in &needles {
        for (what, body) in &bodies {
            assert!(
                !String::from_utf8_lossy(body).contains(needle.as_str()),
                "{what} leaks {needle}"
            );
        }
        assert!(
            !push_text.contains(needle.as_str()),
            "GET /api/push leaks {needle}"
        );
        assert!(!sse_text.contains(needle.as_str()), "SSE leaks {needle}");
        assert!(
            !logs.contains(needle.as_str()),
            "logs leak {needle}: {logs}"
        );
    }
    assert!(
        push_text.contains(&public_key),
        "the public key is served by GET /api/push"
    );
    for (what, body) in &bodies {
        assert!(
            !String::from_utf8_lossy(body).contains(&public_key),
            "{what} carries the public key"
        );
    }
    assert!(!sse_text.contains(&public_key));
    assert!(!logs.contains(&public_key), "the public key is not logged");
    for word in [
        "web.push.apple.com",
        "mozilla",
        "vapid",
        "aes128gcm",
        "Authorization",
    ] {
        assert!(!logs.contains(word), "logs mention {word}: {logs}");
    }
}

// ---------------------------------------------------------------------------------------
// Test 34 — isolation
// ---------------------------------------------------------------------------------------

/// Test 34 (RMC70, §7): a transport that never answers, or fails again and again, changes
/// nothing for status, lock, alerts and their events; `serve` keeps running.
#[tokio::test(start_paused = true)]
async fn test_rwp_push_failure_never_affects_other_features() {
    for mode in ["hold", "fail"] {
        let (_env, j, t, h, _uas) = setup(&[apple_endpoint(1)]).await;
        match mode {
            "hold" => t.set_default(Scripted::Hold),
            _ => {
                t.set_default(Scripted::Fail(TransportError::Protocol));
                t.queue(&[
                    Scripted::Fail(TransportError::Unavailable),
                    Scripted::Fail(TransportError::Timeout),
                    Scripted::Fail(TransportError::Unavailable),
                ]);
            }
        }
        let mut stream = h.open_stream().await;
        let mut sse = Vec::new();
        for round in 0..4u64 {
            sudo_burst(&h, &j, 1).await;
            idle(&h, 40_000, 1000).await;
            assert_eq!(
                alerts_view(&h).await["unacknowledged_wrong_password"],
                round + 1,
                "{mode}: the follower keeps recording"
            );
        }
        assert!(t.count() >= 1, "{mode}");
        let r = h.get("/api/status").await;
        assert_eq!(r.status, 200);
        assert_status_shape(&r.json());
        let r = h.lock(&[]).await;
        assert_eq!(r.status, 202, "{mode}");
        assert_eq!(r.result(), "lock_requested");
        h.source.set_locked();
        idle(&h, 5_000, 500).await;
        drain(&mut stream, &mut sse).await;
        let text = String::from_utf8_lossy(&sse);
        assert!(text.contains("event: status"), "{mode}: status events flow");
        assert!(text.contains("event: alerts"), "{mode}: alert events flow");
        assert!(text.contains("\"locked\""), "{mode}: the lock is reported");
        assert!(
            !h.server.as_ref().unwrap().is_finished(),
            "{mode}: serve keeps running"
        );
        assert_eq!(push_view(&h).await["state"], "active");
    }
}

// ---------------------------------------------------------------------------------------
// Test 35 — store failures
// ---------------------------------------------------------------------------------------

/// Test 35 (RMC65, RMC70, §3.3, §5.7, F-7, F-9): an insecure store at start is
/// `unavailable/store_failed` (routes `503 unavailable` before the body, file untouched,
/// one audit line); a failing CSPRNG without a file is `rng_failed`; a store deleted while
/// running is `store_missing`, routes answer `503 store_unavailable` and nothing recreates
/// it until `PushStore::reset`; `devices` lists service and creation time only.
#[tokio::test(start_paused = true)]
async fn test_rwp_store_failure_is_unavailable() {
    // Insecure store at start.
    {
        let (capture, _guard) = capture_logs();
        let env = Env::new();
        let store = env.push_store();
        let fresh = tempfile::tempdir().unwrap();
        let tmp = PushStore::new(fresh.path().join("x.json"), own_uid());
        tmp.load_or_create(&counter_random()).unwrap();
        let bytes = fs::read(fresh.path().join("x.json")).unwrap();
        write_file_mode(&store, &bytes, 0o644);
        let before = snapshot(&store);
        let j = ScriptedJournal::new();
        let t = FakeTransport::new();
        let h = start_push(&env, &j, &t, PushOptions::enabled()).await;
        until_active(&h, &j).await;
        let v = push_view(&h).await;
        assert_eq!(
            v,
            json!({
                "state": "unavailable",
                "reason": "store_failed",
                "public_key": null,
                "subscriptions": 0,
                "devices": [],
                "last_delivery": null,
                "sender": "unknown",
            })
        );
        let mut held = h
            .hold(
                &Via::Tailnet,
                "POST",
                "/api/push/subscribe",
                &[("X-Soos-Action", "push-subscribe"), ("Origin", ORIGIN)],
                Some(300),
            )
            .await;
        let r = held.response_now().await.expect("refused before the body");
        assert_result(&r, 503, "unavailable");
        assert_result(&push_test(&h).await, 503, "unavailable");
        sudo_burst(&h, &j, 2).await;
        idle(&h, 60_000, 1000).await;
        assert_eq!(t.count(), 0);
        assert_eq!(snapshot(&store), before, "never rewritten");
        let text = capture.text();
        assert_eq!(
            count_lines(&text, "push notifications unavailable"),
            1,
            "{text}"
        );
        assert_audit_lines(&text, "push notifications unavailable", "WARN");
    }
    // Failing CSPRNG, no file.
    {
        let env = Env::with_random(failing_random());
        let j = ScriptedJournal::new();
        let t = FakeTransport::new();
        let h = start_push(&env, &j, &t, PushOptions::enabled()).await;
        pump().await;
        let v = push_view(&h).await;
        assert_eq!(
            (v["state"].clone(), v["reason"].clone()),
            (json!("unavailable"), json!("rng_failed"))
        );
        assert!(fs::symlink_metadata(env.push_store()).is_err());
    }
    // Deleted while running.
    {
        let (env, j, t, h, _uas) = setup(&[apple_endpoint(1)]).await;
        h.advance_ms(5_000).await;
        let created_2 = h.now_unix_s();
        assert_result(
            &subscribe(&h, &mozilla_endpoint(2), &UaFixture::new(2)).await,
            200,
            "subscribed",
        );
        gap(&h).await;
        let v = push_view(&h).await;
        let devices = v["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0]["service"], "apple");
        assert_eq!(
            devices[1],
            json!({"service": "mozilla", "created_unix_s": created_2})
        );
        assert!(devices[0]["created_unix_s"].as_u64().unwrap() < created_2);
        let old_key = v["public_key"].as_str().unwrap().to_string();

        fs::remove_file(env.push_store()).unwrap();
        let v = push_view(&h).await;
        assert_eq!(
            v,
            json!({
                "state": "unavailable",
                "reason": "store_missing",
                "public_key": null,
                "subscriptions": 0,
                "devices": [],
                "last_delivery": null,
                "sender": "unknown",
            })
        );
        assert_result(
            &subscribe(&h, &fcm_endpoint(3), &ua_keys()).await,
            503,
            "store_unavailable",
        );
        gap(&h).await;
        assert_result(
            &unsubscribe(&h, &apple_endpoint(1)).await,
            503,
            "store_unavailable",
        );
        gap(&h).await;
        assert_result(&push_test(&h).await, 503, "store_unavailable");
        sudo_burst(&h, &j, 2).await;
        idle(&h, 60_000, 1000).await;
        assert_eq!(t.count(), 0, "nothing sent without a store");
        assert!(
            fs::symlink_metadata(env.push_store()).is_err(),
            "the service never recreates the store"
        );
        assert_eq!(push_view(&h).await["last_delivery"], "failed");
        // The owner's `soos-remote push reset`.
        let reset = PushStore::new(env.push_store(), own_uid())
            .reset(&counter_random())
            .unwrap();
        let v = push_view(&h).await;
        assert_eq!(v["state"], "active");
        assert_eq!(v["reason"], Value::Null);
        assert_eq!(v["public_key"], reset.key.public_key_b64());
        assert_ne!(v["public_key"], old_key.as_str());
        assert_eq!(v["subscriptions"], 0);
        assert_eq!(v["devices"], json!([]));
    }
}

// ---------------------------------------------------------------------------------------
// Test 36 — Unix transport
// ---------------------------------------------------------------------------------------

fn sample_request() -> DeliveryRequest {
    DeliveryRequest {
        endpoint: PushEndpoint::parse(&apple_endpoint(1)).unwrap(),
        authorization: zeroize::Zeroizing::new("vapid t=a.b.c, k=BAAA".to_string()),
        ttl_s: PUSH_TTL_S,
        urgency: Urgency::High,
        topic: Some(PUSH_TOPIC.to_string()),
        body: zeroize::Zeroizing::new(vec![0x42; 200]),
    }
}

/// An in-test sender: accepts one connection, reads one frame (returned), waits `delay_ms`
/// (virtual), then writes `answer` (raw bytes) unless `None`, and holds the connection.
fn fake_sender(
    listener: UnixListener,
    delay_ms: u64,
    answer: Option<Vec<u8>>,
) -> tokio::task::JoinHandle<Vec<u8>> {
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut prefix = [0u8; 4];
        stream.read_exact(&mut prefix).await.unwrap();
        let mut payload = vec![0u8; u32::from_be_bytes(prefix) as usize];
        stream.read_exact(&mut payload).await.unwrap();
        let mut frame = prefix.to_vec();
        frame.extend_from_slice(&payload);
        tokio::time::sleep(ms(delay_ms)).await;
        if let Some(answer) = answer {
            stream.write_all(&answer).await.unwrap();
            tokio::time::sleep(ms(3_600_000)).await;
        } else {
            tokio::time::sleep(ms(3_600_000)).await;
        }
        drop(stream);
        frame
    })
}

/// Test 36 (RMC68, RMC71, §5.5, F-4): exact request bytes and decoded reply; no answer is
/// `Timeout` at exactly `PUSH_EXCHANGE_TIMEOUT_MS`; an answer after 17 s is used; a missing
/// socket, a symlink, a regular file or a socket of another uid is `Unavailable` without
/// connecting; a malformed reply is `Protocol`.
#[tokio::test(start_paused = true)]
async fn test_rwp_unix_transport_round_trip() {
    let _frozen = FrozenClock::hold();
    let dir = tempfile::tempdir().unwrap();
    let uid = own_uid();
    let path = dir.path().join("push.sock");
    let request = sample_request();
    let expected_frame = encode_request(&request).unwrap().to_vec();
    let answer = DeliveryReply {
        outcome: Outcome::Retry,
        status: Some(429),
        retry_after_s: Some(120),
    };

    // Round trip.
    let sender = fake_sender(
        UnixListener::bind(&path).unwrap(),
        0,
        Some(encode_reply(&answer).unwrap()),
    );
    let transport = UnixPushTransport::new(path.clone(), uid);
    let reply = transport.deliver(request.clone()).await;
    assert_eq!(reply, Ok(answer));
    sender.abort();

    // Request bytes (checked through a sender that returns them).
    fs::remove_file(&path).unwrap();
    let listener = UnixListener::bind(&path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = Arc::clone(&seen);
    let reply_bytes = encode_reply(&answer).unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut prefix = [0u8; 4];
        stream.read_exact(&mut prefix).await.unwrap();
        let mut payload = vec![0u8; u32::from_be_bytes(prefix) as usize];
        stream.read_exact(&mut payload).await.unwrap();
        let mut frame = prefix.to_vec();
        frame.extend_from_slice(&payload);
        *seen2.lock().unwrap() = frame;
        stream.write_all(&reply_bytes).await.unwrap();
    });
    let transport = UnixPushTransport::new(path.clone(), uid);
    assert_eq!(transport.deliver(request.clone()).await, Ok(answer));
    task.await.unwrap();
    assert_eq!(*seen.lock().unwrap(), expected_frame, "exact request frame");

    // Never answering: Timeout at exactly PUSH_EXCHANGE_TIMEOUT_MS.
    fs::remove_file(&path).unwrap();
    let sender = fake_sender(UnixListener::bind(&path).unwrap(), 0, None);
    let transport = Arc::new(UnixPushTransport::new(path.clone(), uid));
    let t2 = Arc::clone(&transport);
    let req = request.clone();
    let pending = tokio::spawn(async move { t2.deliver(req).await });
    settle().await;
    tokio::time::advance(ms(PUSH_EXCHANGE_TIMEOUT_MS - 1)).await;
    settle().await;
    assert!(
        !finished_soon(&pending).await,
        "not before {PUSH_EXCHANGE_TIMEOUT_MS} ms"
    );
    tokio::time::advance(ms(1)).await;
    settle().await;
    assert!(finished_soon(&pending).await);
    assert_eq!(pending.await.unwrap(), Err(TransportError::Timeout));
    sender.abort();

    // An answer after 17 s (inside the bound) is used.
    fs::remove_file(&path).unwrap();
    let sender = fake_sender(
        UnixListener::bind(&path).unwrap(),
        17_000,
        Some(encode_reply(&answer).unwrap()),
    );
    let t3 = Arc::new(UnixPushTransport::new(path.clone(), uid));
    let t4 = Arc::clone(&t3);
    let req = request.clone();
    let pending = tokio::spawn(async move { t4.deliver(req).await });
    settle().await;
    for _ in 0..17 {
        tokio::time::advance(ms(1000)).await;
        settle().await;
    }
    assert!(finished_soon(&pending).await, "answered at 17 s");
    assert_eq!(pending.await.unwrap(), Ok(answer));
    sender.abort();

    // Malformed reply.
    fs::remove_file(&path).unwrap();
    let mut bad = (8u32).to_be_bytes().to_vec();
    bad.extend_from_slice(b"{nope!!}");
    let sender = fake_sender(UnixListener::bind(&path).unwrap(), 0, Some(bad));
    let transport = UnixPushTransport::new(path.clone(), uid);
    assert_eq!(
        transport.deliver(request.clone()).await,
        Err(TransportError::Protocol)
    );
    sender.abort();
    // Oversized reply prefix.
    fs::remove_file(&path).unwrap();
    let sender = fake_sender(
        UnixListener::bind(&path).unwrap(),
        0,
        Some(u32::MAX.to_be_bytes().to_vec()),
    );
    let transport = UnixPushTransport::new(path.clone(), uid);
    assert_eq!(
        transport.deliver(request.clone()).await,
        Err(TransportError::Protocol)
    );
    sender.abort();

    // Missing socket.
    fs::remove_file(&path).unwrap();
    let transport = UnixPushTransport::new(path.clone(), uid);
    assert_eq!(
        transport.deliver(request.clone()).await,
        Err(TransportError::Unavailable)
    );

    // A symlink to a live socket, a regular file, a socket not owned by the expected uid:
    // never connected.
    let real = dir.path().join("real.sock");
    let std_listener = std::os::unix::net::UnixListener::bind(&real).unwrap();
    std_listener.set_nonblocking(true).unwrap();
    std::os::unix::fs::symlink(&real, &path).unwrap();
    let transport = UnixPushTransport::new(path.clone(), uid);
    assert_eq!(
        transport.deliver(request.clone()).await,
        Err(TransportError::Unavailable)
    );
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"plain file").unwrap();
    assert_eq!(
        UnixPushTransport::new(path.clone(), uid)
            .deliver(request.clone())
            .await,
        Err(TransportError::Unavailable)
    );
    assert_eq!(
        UnixPushTransport::new(real.clone(), uid.wrapping_add(1))
            .deliver(request.clone())
            .await,
        Err(TransportError::Unavailable)
    );
    assert!(
        std_listener.accept().is_err(),
        "no connection reached the live socket"
    );
}

// ---------------------------------------------------------------------------------------
// Test 54 — undelivered counts carry forward
// ---------------------------------------------------------------------------------------

fn delivered_sum(t: &FakeTransport, endpoint: &str, ua: &UaFixture, outcomes: &[bool]) -> u64 {
    t.calls_to(endpoint)
        .iter()
        .zip(outcomes)
        .filter(|(_, delivered)| **delivered)
        .map(|(c, _)| wrong_of(c, ua))
        .sum()
}

/// Test 54 (RMC67, W-13, §5.6, F-10, F-12): counts not delivered to a phone (retries
/// exhausted, a retry replaced by a newer summary, a rejection) are folded into its next
/// notification, so the delivered counts add up to the live attempts.
#[tokio::test(start_paused = true)]
async fn test_rwp_undelivered_counts_carry_forward() {
    let ep = apple_endpoint(1);
    // Case 1: retries exhausted, then 4 more attempts → 7.
    {
        let (_env, j, t, h, uas) = setup(std::slice::from_ref(&ep)).await;
        let retry = reply(Outcome::Retry, Some(503), None);
        t.queue(&[retry, retry, retry]);
        sudo_burst(&h, &j, 3).await;
        wait_calls(&h, &t, 3, 60_000, 500).await;
        idle(&h, 60_000, 1000).await;
        assert_eq!(t.count(), 3);
        assert_eq!(push_view(&h).await["last_delivery"], "failed");
        sudo_burst(&h, &j, 4).await;
        wait_calls(&h, &t, 4, 60_000, 500).await;
        idle(&h, 120_000, 1000).await;
        assert_eq!(t.count(), 4);
        assert_eq!(wrong_of(&t.calls()[3], &uas[0]), 7);
        assert_eq!(
            delivered_sum(&t, &ep, &uas[0], &[false, false, false, true]),
            7
        );
        assert_eq!(push_view(&h).await["last_delivery"], "delivered");
    }
    // Case 2 (F-12): a Retry-After 120 s retry replaced by the next summary (due at +30 s).
    {
        let (_env, j, t, h, uas) = setup(std::slice::from_ref(&ep)).await;
        t.queue(&[reply(Outcome::Retry, Some(429), Some(120))]);
        sudo_burst(&h, &j, 3).await;
        wait_calls(&h, &t, 1, 10_000, 100).await;
        h.advance_ms(2000).await;
        sudo_burst(&h, &j, 2).await;
        wait_calls(&h, &t, 2, PUSH_MIN_INTERVAL_MS + 1000, 100).await;
        idle(&h, 300_000, 1000).await;
        assert_eq!(t.count(), 2, "the replaced retry is never sent");
        assert_eq!(wrong_of(&t.calls()[1], &uas[0]), 5);
        assert_eq!(delivered_sum(&t, &ep, &uas[0], &[false, true]), 5);
    }
    // Case 3: Rejected, then a later summary of 1 → 3.
    {
        let (_env, j, t, h, uas) = setup(std::slice::from_ref(&ep)).await;
        t.queue(&[reply(Outcome::Rejected, Some(400), None)]);
        sudo_burst(&h, &j, 2).await;
        wait_calls(&h, &t, 1, 10_000, 100).await;
        idle(&h, 60_000, 1000).await;
        assert_eq!(t.count(), 1);
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 2, 60_000, 500).await;
        idle(&h, 60_000, 1000).await;
        assert_eq!(t.count(), 2);
        assert_eq!(wrong_of(&t.calls()[1], &uas[0]), 3);
        assert_eq!(delivered_sum(&t, &ep, &uas[0], &[false, true]), 3);
    }
    // Two subscriptions: one phone's failure never adds counts to the other's.
    {
        let endpoints = vec![apple_endpoint(1), mozilla_endpoint(2)];
        let (_env, j, t, h, uas) = setup(&endpoints).await;
        // The first call (to whichever is first in store order) is rejected.
        t.queue(&[reply(Outcome::Rejected, Some(400), None)]);
        sudo_burst(&h, &j, 2).await;
        wait_calls(&h, &t, 2, 10_000, 100).await;
        idle(&h, 60_000, 1000).await;
        sudo_burst(&h, &j, 1).await;
        wait_calls(&h, &t, 4, 60_000, 500).await;
        idle(&h, 60_000, 1000).await;
        let a = t.calls_to(&endpoints[0]);
        let b = t.calls_to(&endpoints[1]);
        assert_eq!((a.len(), b.len()), (2, 2));
        assert_eq!(wrong_of(&a[1], &uas[0]), 3, "rejected phone gets the carry");
        assert_eq!(wrong_of(&b[0], &uas[1]), 2);
        assert_eq!(wrong_of(&b[1], &uas[1]), 1, "the other phone has no carry");
    }
}

// ---------------------------------------------------------------------------------------
// Test 61 — round 3: acknowledging never changes push (owner request 2026-10-06)
// ---------------------------------------------------------------------------------------

/// `POST /api/alerts/ack` for the displayed `epoch` and `through`.
async fn alerts_ack(h: &Harness, epoch: &str, through: u64) -> HttpResponse {
    let through = through.to_string();
    h.send(
        &Via::Tailnet,
        "POST",
        "/api/alerts/ack",
        &[
            ("X-Soos-Action", "alerts-ack"),
            ("X-Soos-Alerts-Epoch", epoch),
            ("X-Soos-Alerts-Through", &through),
        ],
        None,
    )
    .await
}

/// Test 61 (RMC75, spec R3.4, alerts round 3, owner request 2026-10-06 "clear acknowledged
/// entries"): removing acknowledged entries never cancels, reduces or sends a notification:
/// an acknowledgement before the coalesced summary is due still delivers it with its
/// count; a later live attempt gets its own notification (no carry, no loss); an
/// acknowledgement with nothing pending sends nothing; replayed attempts (covered by the
/// marker or not) are never pushed after a restart.
#[tokio::test(start_paused = true)]
async fn test_rwp_acknowledge_never_changes_push() {
    let ep = apple_endpoint(1);
    let (env, j, t, h, uas) = setup(std::slice::from_ref(&ep)).await;

    // A live attempt, acknowledged at once (before the 3 s summary is due).
    let a1 = (now_us(&h) / 1000) * 1000;
    j.push(&sudo_failure(a1));
    pump().await;
    let v = alerts_view(&h).await;
    assert_eq!(v["through"], 1, "{v}");
    let epoch = v["epoch"].as_str().unwrap().to_string();
    let r = alerts_ack(&h, &epoch, 1).await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    assert_eq!(r.json()["history"], json!([]), "the entry is removed");
    assert_eq!(t.count(), 0, "the acknowledgement sends nothing by itself");
    wait_calls(&h, &t, 1, PUSH_COALESCE_MS + 2000, 100).await;
    idle(&h, 10_000, 500).await;
    assert_eq!(t.count(), 1, "exactly one notification");
    assert_eq!(
        wrong_of(&t.calls()[0], &uas[0]),
        1,
        "its count is unchanged"
    );

    // A second live attempt after the acknowledgement: its own notification, count 1.
    idle(&h, PUSH_MIN_INTERVAL_MS, 1000).await;
    let b1 = (now_us(&h) / 1000) * 1000;
    j.push(&sudo_failure(b1));
    pump().await;
    wait_calls(
        &h,
        &t,
        2,
        PUSH_MIN_INTERVAL_MS + PUSH_COALESCE_MS + 2000,
        250,
    )
    .await;
    idle(&h, 10_000, 500).await;
    assert_eq!(t.count(), 2);
    assert_eq!(
        wrong_of(&t.calls()[1], &uas[0]),
        1,
        "no carry from the acknowledged attempt, no loss"
    );

    // Acknowledging with nothing pending sends nothing.
    let v = alerts_view(&h).await;
    assert_eq!(v["through"], 2, "{v}");
    let r = alerts_ack(&h, &epoch, 2).await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    assert_eq!(r.json()["history"], json!([]));
    idle(&h, 2 * PUSH_MIN_INTERVAL_MS, 1000).await;
    assert_eq!(t.count(), 2, "nothing sent by an acknowledgement");
    h.shutdown().await.unwrap();
    tokio::time::advance(ms(10_000)).await;

    // Restart: covered replays are discarded, an uncovered replay is shown; none is pushed.
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t, PushOptions::enabled()).await;
    until_active(&h, &j).await;
    assert_eq!(push_view(&h).await["subscriptions"], 1);
    j.push(&sudo_failure(a1));
    j.push(&sudo_failure(b1));
    let c1 = b1 + 5 * SECOND_US;
    j.push(&sudo_failure(c1));
    idle(&h, 60_000, 500).await;
    assert_eq!(t.count(), 0, "replayed attempts are never pushed");
    let v = alerts_view(&h).await;
    let history = v["history"].as_array().unwrap();
    assert_eq!(history.len(), 1, "only the uncovered replay is shown: {v}");
    assert_eq!(history[0]["first_unix_ms"], c1 / 1000);
    assert_eq!(v["unacknowledged_wrong_password"], 1);
}

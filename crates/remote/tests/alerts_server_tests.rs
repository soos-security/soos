//! End-to-end contract tests of the failed-password alerts in `soos-remote::server::serve`
//! (ADR 2026-10-06 "Failed-Password Alerts in `soos-remote` From the System Journal",
//! architect spec `AI/architect_spec_remote_auth_alerts.md` §5, §6, §7, §9, tests 23–36 and
//! 53–55; matrix RMC45, RMC48–RMC55), plus round 3 (owner request 2026-10-06 "clear
//! acknowledged entries", spec R3.3/R3.4, test 60, matrix RMC75; the helpers `counts`/`key`,
//! `RECORD_KEYS` and tests 30, 31, 54 carry recorded contract migrations,
//! `AI/tester_contract_alerts.md`).
//!
//! Same deterministic harness as `server_tests.rs` / `auth_server_tests.rs` (frozen paused
//! clock, scripted logind, raw HTTP/1.1 over the Unix socket in a `TempDir`, passkey store
//! fixture for Funnel sessions), plus a [`ScriptedJournal`] injected through
//! `ServerState::with_password_alerts`: no real journal, no `journalctl`, no network, no
//! logind, no `~/.config`.
//!
//! Plan-evaluator round-2 findings encoded here (the approved spec had not folded them):
//! G-1 the loaded marker survives two restarts without a new acknowledgement (test 31);
//! G-2 a disabled service reports `disabled` even with a failing CSPRNG (test 23);
//! G-9 backoff reset after a stable run (test 26), lock-screen coverage re-evaluated at each
//! follower start (test 27), a per-call varying CSPRNG for the epoch (tests 31, 53).
//! Contract choices where the spec is silent: the ack file owner is checked against
//! `ServerState::file_owner_uid` (the store's seam); the CSRF refusal of the ack route is
//! `403 forbidden` like the lock route.

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

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::UnixListener;
use tokio::sync::oneshot;
use tokio::task::yield_now;

use harness::*;
use journal::*;
use passkey::*;
use soos_remote::alerts::AlertSettings;
use soos_remote::auth::{RandomError, RandomSource};
use soos_remote::config::{AlertsConfig, AuthConfig, RemoteConfig, TailscaleLogin};
use soos_remote::journal::{FollowStart, JournalError, OwnerLogin};
use soos_remote::server::{serve, ServerState};
use soos_remote::{
    ALERTS_ACK_FILE_NAME, ALERT_EVENT_MIN_INTERVAL_MS, CREDENTIALS_FILE_NAME,
    DEFAULT_POLL_INTERVAL_MS, HISTORY_REBUILD_WINDOW_S, JOURNAL_BATCH_PAUSE_MS,
    JOURNAL_IDLE_TICK_MS, JOURNAL_LINES_PER_BATCH, JOURNAL_RESTART_MAX_MS, JOURNAL_RESTART_MIN_MS,
    JOURNAL_STABLE_RUN_MS, MIN_ALERT_ACK_INTERVAL_MS, SSE_KEEPALIVE_MS,
};

const IP_A: &str = "203.0.113.10";
const IP_B: &str = "203.0.113.20";
const IP_C: &str = "203.0.113.30";
const HOUR_US: u64 = 3_600 * SECOND_US;

// ---------------------------------------------------------------------------------------
// Alert harness
// ---------------------------------------------------------------------------------------

/// What persists across service restarts of one test: the directory (socket, store, ack
/// file), the Unix clock and the CSPRNG.
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

    fn ack_path(&self) -> PathBuf {
        self.path(ALERTS_ACK_FILE_NAME)
    }

    /// A regular file standing for a lock-screen program.
    fn locker(&self) -> String {
        let path = self.path("swaylock-plugin");
        if !path.exists() {
            fs::write(&path, b"#!/bin/false\n").unwrap();
        }
        path.to_string_lossy().into_owned()
    }
}

/// A CSPRNG whose output differs on every call (SHA-256 of a counter).
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

/// A CSPRNG that always fills `0xab` (no decimal digit in any hex or base64 rendering).
fn fixed_random() -> RandomSource {
    Arc::new(|buf: &mut [u8]| -> Result<(), RandomError> {
        buf.fill(0xab);
        Ok(())
    })
}

fn failing_random() -> RandomSource {
    Arc::new(|_: &mut [u8]| -> Result<(), RandomError> { Err(RandomError::Failed) })
}

#[derive(Clone)]
enum AckPath {
    /// `<dir>/remote-alerts.json`.
    Default,
    /// No ack file (in memory only).
    Absent,
    Custom(PathBuf),
}

#[derive(Clone)]
struct AlertOptions {
    enabled: bool,
    owner_login: Option<&'static str>,
    /// `None`: the built-in defaults.
    lock_screen_programs: Option<Vec<String>>,
    ack_path: AckPath,
    socket: &'static str,
}

impl AlertOptions {
    fn enabled() -> Self {
        Self {
            enabled: true,
            owner_login: Some(OWNER),
            lock_screen_programs: None,
            ack_path: AckPath::Default,
            socket: "remote.sock",
        }
    }

    fn with_locker(env: &Env) -> Self {
        Self {
            lock_screen_programs: Some(vec![env.locker()]),
            ..Self::enabled()
        }
    }

    fn socket(self, socket: &'static str) -> Self {
        Self { socket, ..self }
    }
}

/// Starts `serve` with alerts wired to `journal`; Funnel and passkeys are configured (the
/// owner's passkey stored) so Funnel sessions can be opened; unlock stays off.
async fn start_alerts(env: &Env, journal: &ScriptedJournal, options: AlertOptions) -> Harness {
    let frozen = FrozenClock::hold();
    let dir = env.dir.path().to_path_buf();
    let path = dir.join(options.socket);
    let store_path = dir.join(CREDENTIALS_FILE_NAME);
    let owner = Authenticator::owner();
    if !store_path.exists() {
        write_store(&store_path, &[StoredPasskey::of(&owner)]);
    }
    let alerts = AlertsConfig {
        enabled: options.enabled,
        lock_screen_programs: options
            .lock_screen_programs
            .clone()
            .unwrap_or_else(|| AlertsConfig::default().lock_screen_programs),
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
        push: soos_remote::config::PushConfig::default(),
        camera: soos_remote::config::CameraConfig::default(),
    };
    let settings = AlertSettings {
        owner_login: options
            .owner_login
            .map(|login| OwnerLogin::parse(login).unwrap()),
        lock_screen_programs: alerts.lock_screen_programs.clone(),
        ack_path: match &options.ack_path {
            AckPath::Default => Some(env.ack_path()),
            AckPath::Absent => None,
            AckPath::Custom(path) => Some(path.clone()),
        },
    };
    let source = MockSource::unlocked();
    let clock_fn = Arc::clone(&env.clock);
    let state = ServerState::new(config, UID, source.clone())
        .with_unix_clock(Arc::new(move || clock_fn.now_ms()))
        .with_credentials_path(store_path.clone())
        .with_file_owner_uid(own_uid())
        .with_random(env.random.clone())
        .with_password_alerts(settings, Arc::new(journal.clone()));
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

async fn view_via(h: &Harness, via: &Via) -> Value {
    let r = h.get_via(via, "/api/alerts").await;
    assert_eq!(r.status, 200, "GET /api/alerts: {}", r.result_or_body());
    r.assert_mandatory_headers();
    r.assert_json_body();
    let json = r.json();
    assert_view_shape(&json);
    json
}

async fn view(h: &Harness) -> Value {
    view_via(h, &Via::Tailnet).await
}

/// The view after the server had the chance to process everything already handed to it.
async fn view_settled(h: &Harness) -> Value {
    pump().await;
    view(h).await
}

/// Moves the clock in `step` ms steps until `pred(view)` holds; panics after `max_ms`.
async fn view_within(
    h: &Harness,
    max_ms: u64,
    step: u64,
    pred: impl Fn(&Value) -> bool,
) -> (Value, u64) {
    let mut elapsed = 0;
    loop {
        let v = view_settled(h).await;
        if pred(&v) {
            return (v, elapsed);
        }
        assert!(
            elapsed < max_ms,
            "condition not reached within {max_ms} ms: {v}"
        );
        h.advance_ms(step).await;
        elapsed += step;
    }
}

async fn wait_follows(h: &Harness, journal: &ScriptedJournal, n: usize, max_ms: u64) -> u64 {
    let mut elapsed = 0;
    loop {
        pump().await;
        if journal.follows() >= n {
            return elapsed;
        }
        assert!(
            elapsed < max_ms,
            "follower {n} not started within {max_ms} ms ({} so far)",
            journal.follows()
        );
        h.advance_ms(50).await;
        elapsed += 50;
    }
}

/// Starts, waits for the first follower and for `active` (catch-up by idle tick).
async fn until_active(h: &Harness, journal: &ScriptedJournal) -> Value {
    wait_follows(h, journal, 1, 1000).await;
    view_within(h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| v["state"] == "active")
        .await
        .0
}

fn state_of(v: &Value) -> (String, Value) {
    (
        v["state"].as_str().unwrap().to_string(),
        v["reason"].clone(),
    )
}

fn epoch_of(v: &Value) -> String {
    v["epoch"].as_str().expect("epoch").to_string()
}

fn through_of(v: &Value) -> u64 {
    v["through"].as_u64().expect("through")
}

fn wrong_count(v: &Value) -> u64 {
    v["unacknowledged_wrong_password"].as_u64().unwrap()
}

fn locked_count(v: &Value) -> u64 {
    v["unacknowledged_locked_out"].as_u64().unwrap()
}

/// Sum of `count` per (`source`, `kind`) over the history. Contract migration (owner
/// request 2026-10-06, "clear acknowledged entries"): records carry no `acknowledged` key
/// any more (acknowledged records are removed), which this helper asserts.
fn counts(v: &Value) -> BTreeMap<(String, String), u64> {
    let mut out = BTreeMap::new();
    for r in v["history"].as_array().unwrap() {
        assert!(r.get("acknowledged").is_none(), "no acknowledged key: {r}");
        let key = (
            r["source"].as_str().unwrap().to_string(),
            r["kind"].as_str().unwrap().to_string(),
        );
        *out.entry(key).or_insert(0) += r["count"].as_u64().unwrap();
    }
    out
}

fn key(source: &str, kind: &str) -> (String, String) {
    (source.to_string(), kind.to_string())
}

const VIEW_KEYS: [&str; 10] = [
    "state",
    "reason",
    "epoch",
    "lock_screen",
    "unacknowledged_wrong_password",
    "unacknowledged_locked_out",
    "last_unix_ms",
    "last_source",
    "through",
    "history",
];
const RECORD_KEYS: [&str; 7] = [
    "id",
    "first_unix_ms",
    "last_unix_ms",
    "source",
    "account",
    "kind",
    "count",
];

/// Exact key sets of the view and of every record (§4.4); `reason` iff `unavailable`.
fn assert_view_shape(v: &Value) {
    let keys: BTreeSet<&str> = v
        .as_object()
        .unwrap_or_else(|| panic!("view is an object: {v}"))
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, VIEW_KEYS.into_iter().collect(), "{v}");
    for r in v["history"].as_array().expect("history array") {
        let keys: BTreeSet<&str> = r.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, RECORD_KEYS.into_iter().collect(), "{r}");
    }
    assert_eq!(
        v["state"] == "unavailable",
        !v["reason"].is_null(),
        "reason iff unavailable: {v}"
    );
}

fn disabled_view() -> Value {
    json!({
        "state": "disabled",
        "reason": null,
        "epoch": null,
        "lock_screen": null,
        "unacknowledged_wrong_password": 0,
        "unacknowledged_locked_out": 0,
        "last_unix_ms": null,
        "last_source": null,
        "through": 0,
        "history": [],
    })
}

/// `POST /api/alerts/ack` with the action header and the two snapshot headers.
async fn ack_via(h: &Harness, via: &Via, epoch: &str, through: &str) -> HttpResponse {
    h.send(
        via,
        "POST",
        "/api/alerts/ack",
        &[
            ("X-Soos-Action", "alerts-ack"),
            ("X-Soos-Alerts-Epoch", epoch),
            ("X-Soos-Alerts-Through", through),
        ],
        None,
    )
    .await
}

async fn ack(h: &Harness, epoch: &str, through: &str) -> HttpResponse {
    ack_via(h, &Via::Tailnet, epoch, through).await
}

/// Another well-formed epoch (last hex digit changed).
fn other_epoch(epoch: &str) -> String {
    let mut chars: Vec<char> = epoch.chars().collect();
    let last = chars.last_mut().unwrap();
    *last = if *last == 'a' { 'b' } else { 'a' };
    chars.into_iter().collect()
}

/// A trusted root-side `sudo` failure of the owner (one immediate attempt).
fn sudo_failure(at_us: u64) -> JLine {
    pam_unix_line("sudo", OWNER_UID, Some(SUDO_EXE), OWNER, at_us)
}

/// A trusted root-side `polkit-1` failure (class `other`).
fn polkit_failure(at_us: u64) -> JLine {
    pam_unix_line("polkit-1", 0, Some(POLKIT_HELPER_EXE), OWNER, at_us)
}

/// A trusted root-side `gdm-password` failure (class `login`).
fn gdm_failure(at_us: u64) -> JLine {
    pam_unix_line("gdm-password", 0, Some(GDM_WORKER_EXE), OWNER, at_us)
}

// ---------------------------------------------------------------------------------------
// SSE frames of any event name
// ---------------------------------------------------------------------------------------

enum Frame {
    Event {
        name: String,
        data: Value,
        raw: String,
    },
    Eof,
    Pending,
}

fn poll_frame(c: &mut SseClient) -> Frame {
    loop {
        if let Some(pos) = c.buf.windows(2).position(|w| w == b"\n\n") {
            let frame: Vec<u8> = c.buf.drain(..pos + 2).collect();
            let raw = String::from_utf8(frame).expect("UTF-8 frame");
            let mut name = None;
            let mut data = None;
            for line in raw.lines() {
                if let Some(rest) = line.strip_prefix("event: ") {
                    name = Some(rest.to_string());
                } else if let Some(rest) = line.strip_prefix("data: ") {
                    assert!(data.is_none(), "one data line per event: {raw:?}");
                    data = Some(rest.to_string());
                } else if !line.is_empty() && !line.starts_with(':') {
                    panic!("unexpected SSE line {line:?}");
                }
            }
            let name = name.unwrap_or_else(|| panic!("event without name: {raw:?}"));
            assert!(
                name == "status" || name == "alerts",
                "unknown event name {name:?}"
            );
            let data = data.unwrap_or_else(|| panic!("event without data: {raw:?}"));
            let data: Value = serde_json::from_str(&data).expect("event data is JSON");
            if name == "alerts" {
                assert_view_shape(&data);
            }
            return Frame::Event { name, data, raw };
        }
        let mut chunk = [0u8; 4096];
        match c.stream.try_read(&mut chunk) {
            Ok(0) => return Frame::Eof,
            Ok(n) => c.buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Frame::Pending,
            Err(_) => return Frame::Eof,
        }
    }
}

/// The next frame that needs no time to pass.
async fn frame_now(c: &mut SseClient) -> (String, Value) {
    for _ in 0..POLL_ROUNDS {
        match poll_frame(c) {
            Frame::Event { name, data, .. } => return (name, data),
            Frame::Eof => panic!("stream ended while an event was expected"),
            Frame::Pending => yield_now().await,
        }
    }
    panic!("no event arrived without time passing");
}

/// Every frame received while the clock moves by `total_ms` in `step` ms steps, with its
/// arrival time (virtual ms since the call); stops early at EOF (`true`).
async fn frames_during(
    c: &mut SseClient,
    h: &Harness,
    total_ms: u64,
    step: u64,
) -> (Vec<(u64, String, Value, String)>, bool) {
    let mut out = Vec::new();
    let mut elapsed = 0;
    loop {
        for _ in 0..SETTLE_ROUNDS * 4 {
            match poll_frame(c) {
                Frame::Event { name, data, raw } => out.push((elapsed, name, data, raw)),
                Frame::Eof => return (out, true),
                Frame::Pending => yield_now().await,
            }
        }
        if elapsed >= total_ms {
            return (out, false);
        }
        h.advance_ms(step).await;
        elapsed += step;
    }
}

// ---------------------------------------------------------------------------------------
// Test 23 — opt-in
// ---------------------------------------------------------------------------------------

/// Test 23 (RMC45, A-2, G-2): by default (and with `password_alerts = false`) nothing is
/// probed or spawned, `GET /api/alerts` is the disabled view, the acknowledgement is `403
/// alerts_disabled` before any header check, and no `alerts` event is ever sent — even with
/// a failing CSPRNG (a disabled feature never reports `rng_failed`).
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_disabled_by_default() {
    // The configuration default.
    let default = AlertsConfig::default();
    assert!(!default.enabled);
    assert_eq!(
        default.lock_screen_programs,
        vec![
            "/usr/bin/swaylock".to_string(),
            "/usr/bin/hyprlock".to_string(),
            "/usr/bin/gtklock".to_string(),
            "/usr/bin/waylock".to_string(),
        ]
    );

    // The shared harness (default configuration, no alert wiring).
    {
        let h = Harness::start().await;
        assert_eq!(view(&h).await, disabled_view());
        let r = h
            .request(
                "POST",
                "/api/alerts/ack",
                &[
                    ("X-Soos-Action", "alerts-ack"),
                    ("X-Soos-Alerts-Epoch", "0000000000000000"),
                    ("X-Soos-Alerts-Through", "0"),
                ],
            )
            .await;
        assert_result(&r, 403, "alerts_disabled");
    }

    // Disabled with a source wired: the source is never used.
    let env = Env::with_random(failing_random());
    let j = ScriptedJournal::new();
    let h = start_alerts(
        &env,
        &j,
        AlertOptions {
            enabled: false,
            ..AlertOptions::with_locker(&env)
        },
    )
    .await;
    assert_eq!(view(&h).await, disabled_view());
    let head = h.request("HEAD", "/api/alerts", &[]).await;
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty(), "HEAD carries no body");
    assert_result(
        &ack(&h, "0000000000000000", "0").await,
        403,
        "alerts_disabled",
    );
    // Disabled is answered before the snapshot headers are read.
    let r = h
        .request(
            "POST",
            "/api/alerts/ack",
            &[("X-Soos-Action", "alerts-ack")],
        )
        .await;
    assert_result(&r, 403, "alerts_disabled");
    let mut stream = h.open_stream_via(&Via::Tailnet).await.expect("stream");
    let (name, first) = frame_now(&mut stream).await;
    assert_eq!(name, "status");
    assert_eq!(first["state"], "unlocked");
    let (frames, eof) = frames_during(&mut stream, &h, 5000, 250).await;
    assert!(!eof);
    assert!(
        frames.iter().all(|(_, name, _, _)| name == "status"),
        "no alerts event while disabled"
    );
    assert_eq!(j.probes(), 0, "never probed");
    assert_eq!(j.follows(), 0, "never followed");
    assert!(!env.ack_path().exists(), "no ack file written");
    assert_eq!(view(&h).await, disabled_view());
}

// ---------------------------------------------------------------------------------------
// Test 24 — authentication
// ---------------------------------------------------------------------------------------

/// Test 24 (RMC53, RMC55, A-10, F-4): a tailnet identity or a Funnel session is required;
/// anonymous Funnel is `403 login_required` on both routes; a Funnel session stream is
/// re-validated before every `alerts` event and ends without it once the session is gone
/// (logout from another connection, or its passkey removed), well before the 15 s re-check.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_routes_require_authentication() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
    let v = until_active(&h, &j).await;
    let epoch = epoch_of(&v);

    // Tailnet identity.
    assert_eq!(h.get("/api/alerts").await.status, 200);
    // A foreign identity.
    let r = h
        .raw(&raw_request(
            "GET",
            "/api/alerts",
            &[
                ("Host", HOST),
                ("Tailscale-User-Login", "intruder@example.com"),
            ],
        ))
        .await;
    assert_result(&r, 403, "forbidden");
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/alerts/ack",
            &[
                ("Host", HOST),
                ("Tailscale-User-Login", "intruder@example.com"),
                ("X-Soos-Action", "alerts-ack"),
                ("X-Soos-Alerts-Epoch", &epoch),
                ("X-Soos-Alerts-Through", "0"),
            ],
        ))
        .await;
    assert_result(&r, 403, "forbidden");
    // Anonymous Funnel.
    let anonymous = Via::funnel(IP_A);
    assert_result(
        &h.get_via(&anonymous, "/api/alerts").await,
        403,
        "login_required",
    );
    let r = h
        .send(
            &anonymous,
            "POST",
            "/api/alerts/ack",
            &[
                ("X-Soos-Action", "alerts-ack"),
                ("Origin", ORIGIN),
                ("X-Soos-Alerts-Epoch", &epoch),
                ("X-Soos-Alerts-Through", "0"),
            ],
            None,
        )
        .await;
    assert_result(&r, 403, "login_required");
    // A Funnel session.
    let s = h.session(IP_A).await;
    let v = view_via(&h, &s).await;
    assert_eq!(v["state"], "active");
    let r = ack_via(&h, &s, &epoch, "0").await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    assert_eq!(r.json()["state"], "active");

    // F-4, logout from another connection. A tailnet stream is the positive control.
    let mut tailnet = h.open_stream_via(&Via::Tailnet).await.expect("stream");
    assert_eq!(frame_now(&mut tailnet).await.0, "status");
    assert_eq!(frame_now(&mut tailnet).await.0, "alerts");
    let s1 = h.session(IP_B).await;
    let mut funnel = h.open_stream_via(&s1).await.expect("funnel stream");
    assert_eq!(frame_now(&mut funnel).await.0, "status");
    let (name, first) = frame_now(&mut funnel).await;
    assert_eq!(
        name, "alerts",
        "an alerts event right after the first status"
    );
    assert_eq!(first["state"], "active");
    assert_result(&h.logout(&s1).await, 200, "logged_out");
    j.push(&sudo_failure(now_us(&h)));
    let (frames, eof) = frames_during(&mut funnel, &h, 2 * ALERT_EVENT_MIN_INTERVAL_MS, 250).await;
    assert!(
        eof,
        "the revoked session's stream ends before the 15 s re-check"
    );
    assert!(
        frames.iter().all(|(_, name, _, _)| name != "alerts"),
        "no alerts event after the logout"
    );
    let (frames, _) = frames_during(&mut tailnet, &h, 0, 250).await;
    let alerts: Vec<&Value> = frames
        .iter()
        .filter(|(_, name, _, _)| name == "alerts")
        .map(|(_, _, data, _)| data)
        .collect();
    assert_eq!(alerts.len(), 1, "the tailnet stream got the new attempt");
    assert_eq!(wrong_count(alerts[0]), 1);

    // F-4, passkey removed.
    let s2 = h.session(IP_C).await;
    let mut funnel = h.open_stream_via(&s2).await.expect("funnel stream");
    assert_eq!(frame_now(&mut funnel).await.0, "status");
    assert_eq!(frame_now(&mut funnel).await.0, "alerts");
    write_store(
        &h.store_path,
        &[StoredPasskey::of(&Authenticator::device(
            0x55,
            b"other-device-55",
        ))],
    );
    j.push(&polkit_failure(now_us(&h)));
    let (frames, eof) = frames_during(&mut funnel, &h, 2 * ALERT_EVENT_MIN_INTERVAL_MS, 250).await;
    assert!(eof, "the revoked passkey's stream ends");
    assert!(frames.iter().all(|(_, name, _, _)| name != "alerts"));
    const { assert!(2 * ALERT_EVENT_MIN_INTERVAL_MS < SSE_KEEPALIVE_MS) };
    assert_result(&h.get_via(&s2, "/api/alerts").await, 403, "login_required");
}

// ---------------------------------------------------------------------------------------
// Test 25 — end-to-end day
// ---------------------------------------------------------------------------------------

/// Offset of a wall time of the research day (from 16:00:00).
fn at(h: u64, m: u64, s: u64, ms: u64) -> u64 {
    ((h - 16) * 3600 + m * 60 + s) * SECOND_US + ms * MS_US
}

/// The research §2.2 day (lock screen, sudo, polkit, gdm, lockouts) plus the §2.3 noise,
/// in journal order; `locker` is the lock-screen program's `_EXE`.
fn research_day(base: u64, locker: &str) -> Vec<JLine> {
    let deleted = format!("{locker} (deleted)");
    let t = |h, m, s, ms| base + at(h, m, s, ms);
    vec![
        chkpwd_line(OWNER_UID, OWNER, t(16, 6, 10, 0)),
        pam_unix_line("swaylock", OWNER_UID, Some(locker), OWNER, t(16, 6, 10, 5)),
        chkpwd_line(OWNER_UID, OWNER, t(16, 22, 46, 0)),
        pam_unix_line("swaylock", OWNER_UID, Some(&deleted), OWNER, t(16, 22, 46, 5)),
        chkpwd_line(OWNER_UID, OWNER, t(16, 24, 43, 0)),
        chkpwd_line(OWNER_UID, OWNER, t(16, 25, 38, 0)),
        process_line(
            OWNER_UID,
            Some(locker),
            "pam_faillock(swaylock:auth): Consecutive login failures for user sooshost account temporarily locked",
            t(16, 25, 38, 10),
        ),
        faillock_locked_line("swaylock", OWNER_UID, Some(locker), OWNER, t(16, 25, 59, 0)),
        chkpwd_line(OWNER_UID, OWNER, t(16, 26, 0, 0)),
        faillock_locked_line("swaylock", OWNER_UID, Some(locker), OWNER, t(16, 34, 7, 0)),
        faillock_locked_line("swaylock", OWNER_UID, Some(locker), OWNER, t(16, 34, 16, 0)),
        chkpwd_line(OWNER_UID, OWNER, t(16, 34, 17, 0)),
        pam_unix_line("swaylock", OWNER_UID, Some(locker), OWNER, t(16, 34, 17, 4)),
        faillock_locked_line("swaylock", OWNER_UID, Some(locker), OWNER, t(16, 34, 23, 0)),
        faillock_locked_line("gdm-password", 0, Some(GDM_WORKER_EXE), OWNER, t(16, 34, 38, 0)),
        faillock_locked_line("gdm-password", 0, Some(GDM_WORKER_EXE), OWNER, t(16, 34, 51, 0)),
        faillock_locked_line("gdm-password", 0, Some(GDM_WORKER_EXE), OWNER, t(16, 34, 56, 0)),
        // Noise: a developer test binary and its helper line, its faillock chatter,
        // soos-pam lines, a sudo session line, another local account.
        pam_unix_line("swaylock", OWNER_UID, Some(TEST_BINARY), OWNER, t(17, 0, 0, 0)),
        chkpwd_line(OWNER_UID, OWNER, t(17, 0, 0, 3)),
        process_line(
            OWNER_UID,
            Some(TEST_BINARY),
            "pam_faillock(swaylock:auth): User unknown: drift_test_user",
            t(17, 0, 1, 0),
        ),
        process_line(0, None, "soos-pam: could not resolve the PAM user", t(17, 0, 2, 0)),
        process_line(
            0,
            Some(SUDO_EXE),
            "pam_unix(sudo:session): session opened for user root(uid=0) by sooshost(uid=1000)",
            t(17, 0, 3, 0),
        ),
        chkpwd_line(OTHER_UID, "guest", t(17, 0, 4, 0)),
        pam_unix_line("sudo", OTHER_UID, Some(SUDO_EXE), "guest", t(17, 0, 5, 0)),
        process_line(
            0,
            Some(GDM_WORKER_EXE),
            "pam_unix(gdm-password:auth): conversation failed",
            t(17, 0, 6, 0),
        ),
        // sudo: two checks on one handle, then a second sudo.
        chkpwd_line(0, OWNER, t(18, 5, 21, 0)),
        pam_unix_line("sudo", OWNER_UID, Some(SUDO_EXE), OWNER, t(18, 5, 21, 4)),
        chkpwd_line(0, OWNER, t(18, 5, 28, 0)),
        chkpwd_line(0, OWNER, t(18, 9, 5, 0)),
        pam_unix_line("sudo", OWNER_UID, Some(SUDO_EXE), OWNER, t(18, 9, 5, 3)),
        // polkit twice.
        chkpwd_line(0, OWNER, t(18, 14, 27, 0)),
        pam_unix_line("polkit-1", 0, Some(POLKIT_HELPER_EXE), OWNER, t(18, 14, 27, 2)),
        chkpwd_line(0, OWNER, t(21, 4, 46, 0)),
        pam_unix_line("polkit-1", 0, Some(POLKIT_HELPER_EXE), OWNER, t(21, 4, 46, 2)),
    ]
}

async fn replay_day(env: &Env, options: AlertOptions, locker: &str) -> (Harness, Value) {
    let j = ScriptedJournal::new();
    let h = start_alerts(env, &j, options).await;
    wait_follows(&h, &j, 1, 1000).await;
    let base = now_us(&h) - 12 * HOUR_US;
    for line in research_day(base, locker) {
        j.push(&line);
    }
    pump().await;
    let (v, _) = view_within(&h, 4 * JOURNAL_IDLE_TICK_MS, 250, |v| {
        v["state"] == "active"
    })
    .await;
    // Two more idle ticks: every pending check is resolved.
    h.advance_ms(2 * JOURNAL_IDLE_TICK_MS + 500).await;
    let v2 = view_settled(&h).await;
    assert_eq!(v, v2, "nothing left pending after catch-up");
    (h, v2)
}

/// Test 25 (RMC47, RMC49, RMC50, RMC52, F-2): the research day replayed gives the exact
/// counts per source and kind when the locker is configured (one locker line with the
/// ` (deleted)` suffix); with the defaults the lock screen yields nothing and the other
/// sources are unchanged; a configured locker that does not exist is reported
/// `not_configured`.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_end_to_end_lock_screen_burst() {
    // Case 1: locker configured (a regular file in the test directory).
    let env = Env::new();
    let locker = env.locker();
    let (h, v) = replay_day(&env, AlertOptions::with_locker(&env), &locker).await;
    assert_eq!(v["lock_screen"], "monitored");
    let expected: BTreeMap<_, _> = [
        (key("lock_screen", "wrong_password"), 6),
        (key("lock_screen", "locked_out"), 4),
        (key("login", "locked_out"), 3),
        (key("sudo", "wrong_password"), 3),
        (key("other", "wrong_password"), 2),
    ]
    .into_iter()
    .collect();
    assert_eq!(counts(&v), expected, "{v}");
    assert_eq!(
        wrong_count(&v),
        11,
        "one per real password check (11 that day)"
    );
    assert_eq!(locked_count(&v), 7);
    for r in v["history"].as_array().unwrap() {
        assert_eq!(r["account"], "owner", "{r}");
    }
    assert_eq!(v["last_source"], "other");
    drop(h);

    // Case 2: defaults (none of them is the owner's locker).
    let env = Env::new();
    let locker = env.locker();
    let (h, v) = replay_day(&env, AlertOptions::enabled(), &locker).await;
    let expected: BTreeMap<_, _> = [
        (key("login", "locked_out"), 3),
        (key("sudo", "wrong_password"), 3),
        (key("other", "wrong_password"), 2),
    ]
    .into_iter()
    .collect();
    assert_eq!(counts(&v), expected, "{v}");
    assert_eq!(wrong_count(&v), 5);
    assert_eq!(locked_count(&v), 3);
    drop(h);

    // Case 3: a configured path that does not exist.
    let env = Env::new();
    let locker = env.locker();
    let missing = env.path("missing-locker").to_string_lossy().into_owned();
    let (h, v) = replay_day(
        &env,
        AlertOptions {
            lock_screen_programs: Some(vec![missing]),
            ..AlertOptions::enabled()
        },
        &locker,
    )
    .await;
    assert_eq!(v["lock_screen"], "not_configured");
    assert!(
        counts(&v).keys().all(|(source, _)| source != "lock_screen"),
        "{v}"
    );
    assert_eq!(wrong_count(&v), 5);
    drop(h);
}

// ---------------------------------------------------------------------------------------
// Tests 26–29 — reader lifecycle
// ---------------------------------------------------------------------------------------

/// Asserts the probe count reaches `n` exactly `delay_ms` after now (not 1 ms earlier).
async fn probe_after(h: &Harness, j: &ScriptedJournal, delay_ms: u64, n: usize) {
    let before = j.probes();
    assert_eq!(before + 1, n, "precondition");
    h.advance_ms(delay_ms - 1).await;
    pump().await;
    assert_eq!(j.probes(), before, "no retry before {delay_ms} ms");
    h.advance_ms(1).await;
    pump().await;
    assert_eq!(j.probes(), n, "retry at exactly {delay_ms} ms");
}

/// Test 26 (RMC52, A-3, G-9): a failing probe is `unavailable` with its reason (never a
/// count claim), retried with a 1 s → 60 s doubling backoff; success leads to `starting`
/// then `active`; the backoff resets after a run of at least `JOURNAL_STABLE_RUN_MS`.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_probe_failure_is_unavailable_never_zero() {
    {
        let env = Env::new();
        let j = ScriptedJournal::new();
        j.set_probe(Err(JournalError::NoAccess));
        let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
        pump().await;
        assert_eq!(j.probes(), 1);
        let v = view_settled(&h).await;
        assert_eq!(
            state_of(&v),
            ("unavailable".to_string(), json!("no_journal_access"))
        );
        assert!(!v["epoch"].is_null(), "the epoch exists once drawn");
        assert_eq!(j.follows(), 0);
        j.set_probe(Err(JournalError::NotFound));
        probe_after(&h, &j, JOURNAL_RESTART_MIN_MS, 2).await;
        assert_eq!(
            state_of(&view_settled(&h).await),
            ("unavailable".to_string(), json!("journal_reader_failed"))
        );
        probe_after(&h, &j, 2 * JOURNAL_RESTART_MIN_MS, 3).await;
        j.set_probe(Err(JournalError::ProbeTimeout));
        probe_after(&h, &j, 4 * JOURNAL_RESTART_MIN_MS, 4).await;
        assert_eq!(
            state_of(&view_settled(&h).await),
            ("unavailable".to_string(), json!("journal_reader_failed"))
        );
        j.set_probe(Err(JournalError::Spawn));
        probe_after(&h, &j, 8 * JOURNAL_RESTART_MIN_MS, 5).await;
        assert_eq!(j.follows(), 0);
        j.set_probe(Ok(()));
        probe_after(&h, &j, 16 * JOURNAL_RESTART_MIN_MS, 6).await;
        wait_follows(&h, &j, 1, 0).await;
        let v = view_settled(&h).await;
        assert_eq!(state_of(&v), ("starting".to_string(), Value::Null));
        let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
            v["state"] == "active"
        })
        .await;
        assert_eq!(wrong_count(&v), 0);
    }
    {
        // The cap.
        let env = Env::new();
        let j = ScriptedJournal::new();
        j.set_probe(Err(JournalError::NotFound));
        let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
        pump().await;
        let mut n = 1;
        for delay in [1, 2, 4, 8, 16, 32, 60, 60, 60] {
            n += 1;
            probe_after(&h, &j, delay * 1000, n).await;
        }
        assert_eq!(JOURNAL_RESTART_MAX_MS, 60_000);
    }
    {
        // Reset after a stable run.
        let env = Env::new();
        let j = ScriptedJournal::new();
        j.queue_probes(&[
            Err(JournalError::NotFound),
            Err(JournalError::NotFound),
            Err(JournalError::NotFound),
        ]);
        let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
        pump().await;
        probe_after(&h, &j, 1000, 2).await;
        probe_after(&h, &j, 2000, 3).await;
        probe_after(&h, &j, 4000, 4).await;
        wait_follows(&h, &j, 1, 0).await;
        h.step_ms(2000, JOURNAL_STABLE_RUN_MS).await;
        j.end();
        pump().await;
        assert_eq!(
            state_of(&view_settled(&h).await),
            ("unavailable".to_string(), json!("journal_reader_failed"))
        );
        probe_after(&h, &j, JOURNAL_RESTART_MIN_MS, 5).await;
        wait_follows(&h, &j, 2, 0).await;
        // A short run doubles again.
        j.end();
        pump().await;
        probe_after(&h, &j, 2 * JOURNAL_RESTART_MIN_MS, 6).await;
        wait_follows(&h, &j, 3, 0).await;
    }
}

/// Test 27 (RMC52, §5 steps 2 and 4, G-9): a follower that ends is `unavailable` with the
/// history and the epoch kept, and restarts after its last cursor; a cursor start that ends
/// within 1 s without a line falls back to `Since` and skips entries already seen;
/// lock-screen coverage is re-evaluated at every follower start.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_follower_restart_resumes_after_cursor() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let missing = env.path("locker-installed-later");
    let h = start_alerts(
        &env,
        &j,
        AlertOptions {
            lock_screen_programs: Some(vec![missing.to_string_lossy().into_owned()]),
            ..AlertOptions::enabled()
        },
    )
    .await;
    wait_follows(&h, &j, 1, 1000).await;
    assert_eq!(view_settled(&h).await["lock_screen"], "not_configured");
    let t1 = now_us(&h);
    j.push(&sudo_failure(t1));
    let v = view_settled(&h).await;
    assert_eq!(
        v["state"], "active",
        "an entry newer than the spawn ends the catch-up"
    );
    assert_eq!(wrong_count(&v), 1);
    let epoch = epoch_of(&v);

    fs::write(&missing, b"#!/bin/false\n").unwrap();
    h.advance_ms(5000).await;
    j.end();
    let v = view_settled(&h).await;
    assert_eq!(
        state_of(&v),
        ("unavailable".to_string(), json!("journal_reader_failed"))
    );
    assert_eq!(wrong_count(&v), 1, "history kept");
    assert_eq!(epoch_of(&v), epoch, "epoch kept");
    wait_follows(&h, &j, 2, 3000).await;
    let starts = j.starts();
    assert!(
        starts[1]
            == FollowStart::AfterCursor(
                soos_remote::journal::JournalCursor::parse(&cursor_for(t1)).unwrap()
            ),
        "restart after the last cursor, got {}",
        describe_start(&starts[1])
    );
    let v = view_settled(&h).await;
    assert_eq!(
        v["lock_screen"], "monitored",
        "coverage re-evaluated at the restart"
    );
    assert_eq!(epoch_of(&v), epoch);

    // The cursor start ends at once without a line: the cursor is dropped.
    j.end();
    wait_follows(&h, &j, 3, 10_000).await;
    let starts = j.starts();
    assert!(
        starts[2]
            == FollowStart::Since {
                unix_s: t1 / SECOND_US
            },
        "fallback to Since(last seen second), got {}",
        describe_start(&starts[2])
    );
    // Entries at or before the last seen time are skipped.
    j.push(&sudo_failure(t1));
    j.push(&polkit_failure(t1 - 5 * SECOND_US));
    assert_eq!(wrong_count(&view_settled(&h).await), 1, "already seen");
    j.push(&polkit_failure(t1 + 3 * SECOND_US));
    let v = view_settled(&h).await;
    assert_eq!(wrong_count(&v), 2);
    assert_eq!(epoch_of(&v), epoch);
}

/// Test 28 (RMC52, F-5): the first follower rebuilds 24 h (`--since` saturating at 0); the
/// view stays `starting` while backlog lines arrive without a 2 s gap (even with attempts),
/// and becomes `active` at the first entry newer than the spawn, or after an idle tick;
/// batch pauses never count as idle.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_first_start_rebuilds_24h() {
    {
        let env = Env::new();
        let j = ScriptedJournal::new();
        let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
        wait_follows(&h, &j, 1, 1000).await;
        let spawn_us = now_us(&h);
        let since = spawn_us / SECOND_US - HISTORY_REBUILD_WINDOW_S;
        assert!(
            j.starts()[0] == FollowStart::Since { unix_s: since },
            "got {}",
            describe_start(&j.starts()[0])
        );
        let v = view_settled(&h).await;
        assert_eq!(v["state"], "starting");
        for i in 0..6u64 {
            j.push(&sudo_failure(spawn_us - 10 * HOUR_US + i * 10 * SECOND_US));
            h.advance_ms(JOURNAL_IDLE_TICK_MS / 4).await;
            let v = view_settled(&h).await;
            assert_eq!(v["state"], "starting", "backlog line {i}: {v}");
            assert_eq!(wrong_count(&v), i + 1, "attempts are already counted");
        }
        j.push(&sudo_failure(now_us(&h)));
        let v = view_settled(&h).await;
        assert_eq!(v["state"], "active", "an entry newer than the spawn");
    }
    {
        // Idle tick after the backlog.
        let env = Env::new();
        let j = ScriptedJournal::new();
        let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
        wait_follows(&h, &j, 1, 1000).await;
        let spawn_us = now_us(&h);
        for i in 0..4u64 {
            if i > 0 {
                h.advance_ms(JOURNAL_IDLE_TICK_MS / 2).await;
            }
            j.push(&sudo_failure(spawn_us - HOUR_US + i * SECOND_US));
            assert_eq!(view_settled(&h).await["state"], "starting");
        }
        h.advance_ms(JOURNAL_IDLE_TICK_MS * 3 / 4).await;
        assert_eq!(
            view_settled(&h).await["state"],
            "starting",
            "less than one idle tick since the last line"
        );
        let (v, _) = view_within(&h, 2 * JOURNAL_IDLE_TICK_MS, 100, |v| {
            v["state"] == "active"
        })
        .await;
        assert_eq!(wrong_count(&v), 4);
    }
    {
        // A clock before the window: `--since=@0`.
        let env = Env::new();
        env.clock.shift_ms(-(BASE_UNIX_MS as i64) + 5_000);
        let j = ScriptedJournal::new();
        let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
        wait_follows(&h, &j, 1, 1000).await;
        assert!(
            j.starts()[0] == FollowStart::Since { unix_s: 0 },
            "got {}",
            describe_start(&j.starts()[0])
        );
    }
    {
        // A backlog long enough for batch pauses stays `starting` while it is read.
        let env = Env::new();
        let j = ScriptedJournal::new();
        let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
        wait_follows(&h, &j, 1, 1000).await;
        let spawn_us = now_us(&h);
        let total = 3 * JOURNAL_LINES_PER_BATCH as u64 + 10;
        for i in 0..total {
            j.push(&process_line(
                0,
                Some(SUDO_EXE),
                "pam_unix(sudo:session): session closed for user root",
                spawn_us - 2 * HOUR_US + i * MS_US,
            ));
        }
        for round in 0..4 {
            h.advance_ms(JOURNAL_BATCH_PAUSE_MS).await;
            assert_eq!(
                view_settled(&h).await["state"],
                "starting",
                "batch pause {round} is not idle"
            );
        }
        let (_, waited) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 100, |v| {
            v["state"] == "active"
        })
        .await;
        assert!(
            waited >= JOURNAL_IDLE_TICK_MS / 2,
            "active only after an idle gap"
        );
    }
}

/// Test 29 (RMC52, §3.3): an unresolved owner login is `unavailable` / `owner_unresolved`,
/// nothing is probed or spawned, the service keeps serving.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_owner_unresolved() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let h = start_alerts(
        &env,
        &j,
        AlertOptions {
            owner_login: None,
            ..AlertOptions::enabled()
        },
    )
    .await;
    h.step_ms(500, 10_000).await;
    let v = view_settled(&h).await;
    assert_eq!(
        state_of(&v),
        ("unavailable".to_string(), json!("owner_unresolved"))
    );
    assert_eq!(wrong_count(&v), 0);
    assert_eq!(j.probes(), 0);
    assert_eq!(j.follows(), 0);
    assert_eq!(h.get("/api/status").await.status, 200);
    assert_eq!(h.lock(&[]).await.status, 202);
}

// ---------------------------------------------------------------------------------------
// Tests 30–31, 53–55 — acknowledgement
// ---------------------------------------------------------------------------------------

/// Test 30 (RMC54, A-10, F-1): lock-style CSRF with `alerts-ack`; strict snapshot headers
/// (`400 bad_request`, query never read); one acknowledgement per second (`429`); a body is
/// `413 body_not_allowed`.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_ack_csrf_headers_and_rate() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
    until_active(&h, &j).await;
    let t = now_us(&h);
    j.push(&sudo_failure(t));
    j.push(&polkit_failure(t + SECOND_US));
    let v = view_settled(&h).await;
    assert_eq!(through_of(&v), 2);
    let epoch = epoch_of(&v);
    assert_eq!(epoch.len(), 16);
    assert!(epoch
        .bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    let post = |extra: Vec<(&'static str, String)>, target: &'static str| {
        let h = &h;
        async move {
            let borrowed: Vec<(&str, &str)> = extra.iter().map(|(n, v)| (*n, v.as_str())).collect();
            h.send(&Via::Tailnet, "POST", target, &borrowed, None).await
        }
    };
    let e = || ("X-Soos-Alerts-Epoch", epoch.clone());
    let th = |v: &str| ("X-Soos-Alerts-Through", v.to_string());
    let action = || ("X-Soos-Action", "alerts-ack".to_string());
    let path = "/api/alerts/ack";

    // CSRF.
    assert_result(&post(vec![e(), th("1")], path).await, 403, "forbidden");
    assert_result(
        &post(vec![("X-Soos-Action", "lock".into()), e(), th("1")], path).await,
        403,
        "forbidden",
    );
    assert_result(
        &post(
            vec![
                action(),
                ("X-Soos-Action", "alerts-ack".into()),
                e(),
                th("1"),
            ],
            path,
        )
        .await,
        403,
        "forbidden",
    );
    assert_result(
        &post(
            vec![
                action(),
                ("Sec-Fetch-Site", "cross-site".into()),
                e(),
                th("1"),
            ],
            path,
        )
        .await,
        403,
        "forbidden",
    );
    assert_result(
        &post(
            vec![
                action(),
                ("Origin", "https://evil.example.com".into()),
                e(),
                th("1"),
            ],
            path,
        )
        .await,
        403,
        "forbidden",
    );

    // Snapshot headers.
    let upper = epoch.to_ascii_uppercase();
    let bad_epochs: Vec<Vec<(&'static str, String)>> = vec![
        vec![action(), th("1")],
        vec![action(), e(), e(), th("1")],
        vec![
            action(),
            ("X-Soos-Alerts-Epoch", epoch[..15].to_string()),
            th("1"),
        ],
        vec![
            action(),
            ("X-Soos-Alerts-Epoch", format!("{epoch}0")),
            th("1"),
        ],
        vec![
            action(),
            (
                "X-Soos-Alerts-Epoch",
                if upper == epoch {
                    "ABCDEFABCDEFABCD".to_string()
                } else {
                    upper
                },
            ),
            th("1"),
        ],
        vec![
            action(),
            ("X-Soos-Alerts-Epoch", "0123456789abcdeg".into()),
            th("1"),
        ],
        vec![action(), ("X-Soos-Alerts-Epoch", String::new()), th("1")],
    ];
    for headers in bad_epochs {
        let shown = format!("{headers:?}");
        let r = post(headers, path).await;
        assert_result(&r, 400, "bad_request");
        assert!(
            !r.body.windows(4).any(|w| w == &epoch.as_bytes()[..4]),
            "{shown}: no echo"
        );
    }
    for through in [
        None,
        Some(vec!["1", "1"]),
        Some(vec![""]),
        Some(vec!["a"]),
        Some(vec!["-1"]),
        Some(vec!["+1"]),
        Some(vec!["1.0"]),
        Some(vec!["0x1"]),
        Some(vec!["123456789012345678901"]),
        Some(vec!["18446744073709551616"]),
    ] {
        let mut headers = vec![action(), e()];
        for value in through.clone().unwrap_or_default() {
            headers.push(th(value));
        }
        assert_result(&post(headers, path).await, 400, "bad_request");
    }
    // `through` beyond the newest attempt of this epoch (this request reached the rate
    // gate, so the next acknowledgement waits one interval).
    assert_result(
        &post(vec![action(), e(), th("3")], path).await,
        400,
        "bad_request",
    );
    h.advance_ms(MIN_ALERT_ACK_INTERVAL_MS + 1).await;
    // The query string is never read.
    assert_result(
        &post(vec![action(), e()], "/api/alerts/ack?through=1").await,
        400,
        "bad_request",
    );
    assert_eq!(
        wrong_count(&view(&h).await),
        2,
        "nothing acknowledged so far"
    );
    // A body is refused before anything else.
    let bytes = request_bytes(
        &Via::Tailnet,
        "POST",
        path,
        &[
            ("X-Soos-Action", "alerts-ack"),
            ("X-Soos-Alerts-Epoch", &epoch),
            ("X-Soos-Alerts-Through", "1"),
        ],
        Some("{}"),
    );
    let r = h.raw(&bytes).await;
    assert_result(&r, 413, "body_not_allowed");

    // Valid headers with a misleading query: the headers decide (through = 1).
    let r = post(vec![action(), e(), th("1")], "/api/alerts/ack?through=2").await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    r.assert_mandatory_headers();
    r.assert_json_body();
    let v = r.json();
    assert_view_shape(&v);
    assert_eq!(wrong_count(&v), 1, "only the record of seq 1");
    // Contract migration (owner request 2026-10-06): the acknowledged sudo record is
    // removed from the history.
    assert!(
        !counts(&v).contains_key(&key("sudo", "wrong_password")),
        "{v}"
    );
    assert_eq!(counts(&v)[&key("other", "wrong_password")], 1);
    // Rate limit.
    assert_result(
        &post(vec![action(), e(), th("2")], path).await,
        429,
        "rate_limited",
    );
    h.advance_ms(MIN_ALERT_ACK_INTERVAL_MS + 1).await;
    // Leading zeros are accepted.
    let r = post(vec![action(), e(), th("00000000000000000002")], path).await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    assert_eq!(wrong_count(&r.json()), 0);
    h.advance_ms(MIN_ALERT_ACK_INTERVAL_MS + 1).await;
    // `through = 0` is a no-op `200`.
    let r = post(vec![action(), e(), th("0")], path).await;
    assert_eq!(r.status, 200);
    // Same-origin `Origin` and `Sec-Fetch-Site` are accepted.
    h.advance_ms(MIN_ALERT_ACK_INTERVAL_MS + 1).await;
    let r = post(
        vec![
            action(),
            ("Origin", ORIGIN.into()),
            ("Sec-Fetch-Site", "same-origin".into()),
            e(),
            th("2"),
        ],
        path,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    // Wrong methods.
    let r = h.get("/api/alerts/ack").await;
    assert_result(&r, 405, "method_not_allowed");
    assert_eq!(r.header("allow"), Some("POST"));
    let r = h
        .request("POST", "/api/alerts", &[("X-Soos-Action", "alerts-ack")])
        .await;
    assert_result(&r, 405, "method_not_allowed");
    assert_eq!(r.header("allow"), Some("GET, HEAD"));
}

fn read_ack_value(env: &Env) -> Value {
    let path = env.ack_path();
    let meta = fs::symlink_metadata(&path).expect("ack file exists");
    assert!(meta.is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap()
}

fn ack_marker(env: &Env) -> u64 {
    let v = read_ack_value(env);
    assert_eq!(v["version"], 1, "{v}");
    v["acknowledged_until_us"].as_u64().unwrap()
}

/// Test 31 (RMC54, A-9, G-1): an acknowledgement is persisted `0600` and survives two
/// restarts (the second without a new acknowledgement): replayed attempts at or before the
/// marker are discarded and never shown again (round 3, owner request 2026-10-06), newer
/// ones are shown, the file keeps its value; every start has its own epoch.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_ack_persists_and_survives_restart() {
    let env = Env::new();
    // Run 1.
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled().socket("r1.sock")).await;
    wait_follows(&h, &j, 1, 1000).await;
    let s1 = now_us(&h);
    let t1 = s1 - HOUR_US;
    let t2 = s1 - HOUR_US / 2;
    let lines = [sudo_failure(t1), polkit_failure(t2)];
    for line in &lines {
        j.push(line);
    }
    let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
        v["state"] == "active"
    })
    .await;
    assert_eq!(wrong_count(&v), 2);
    let e1 = epoch_of(&v);
    assert!(
        !env.ack_path().exists(),
        "nothing written before an acknowledgement"
    );
    let r = ack(&h, &e1, "2").await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    assert_eq!(wrong_count(&r.json()), 0);
    assert_eq!(
        read_ack_value(&env),
        json!({"version": 1, "acknowledged_until_us": t2})
    );
    h.shutdown().await.unwrap();
    tokio::time::advance(ms(10_000)).await;

    // Run 2: the journal rebuild replays both attempts plus a newer one.
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled().socket("r2.sock")).await;
    wait_follows(&h, &j, 1, 1000).await;
    let t3 = now_us(&h) - 60 * SECOND_US;
    for line in &lines {
        j.push(line);
    }
    j.push(&gdm_failure(t3));
    let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
        v["state"] == "active"
    })
    .await;
    let e2 = epoch_of(&v);
    assert_ne!(e1, e2, "a new epoch per start");
    // Contract migration (owner request 2026-10-06): the two acknowledged replays are
    // discarded and never shown again.
    let expected: BTreeMap<_, _> = [(key("login", "wrong_password"), 1)].into_iter().collect();
    assert_eq!(counts(&v), expected, "{v}");
    assert_eq!(wrong_count(&v), 1);
    h.advance_ms(3000).await;
    assert_eq!(ack_marker(&env), t2, "the file keeps its value");
    h.shutdown().await.unwrap();
    tokio::time::advance(ms(10_000)).await;

    // Run 3: no acknowledgement since run 1; the marker still applies.
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled().socket("r3.sock")).await;
    wait_follows(&h, &j, 1, 1000).await;
    for line in &lines {
        j.push(line);
    }
    j.push(&gdm_failure(t3));
    let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
        v["state"] == "active"
    })
    .await;
    assert_eq!(counts(&v), expected, "{v}");
    assert_ne!(epoch_of(&v), e2);
    h.advance_ms(3000).await;
    assert_eq!(ack_marker(&env), t2);
}

/// Test 53 (RMC54, A-13, F-3 a): an acknowledgement naming another epoch is `409
/// stale_view` and acknowledges nothing, also after a restart where its `through` is in
/// range.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_ack_stale_epoch_is_refused() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled().socket("r1.sock")).await;
    wait_follows(&h, &j, 1, 1000).await;
    let t1 = now_us(&h) - HOUR_US;
    j.push(&sudo_failure(t1));
    let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
        v["state"] == "active"
    })
    .await;
    let e1 = epoch_of(&v);
    let r = ack(&h, &other_epoch(&e1), "1").await;
    assert_result(&r, 409, "stale_view");
    let after = view(&h).await;
    assert_eq!(after, v, "unchanged");
    h.shutdown().await.unwrap();
    tokio::time::advance(ms(10_000)).await;

    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled().socket("r2.sock")).await;
    wait_follows(&h, &j, 1, 1000).await;
    j.push(&sudo_failure(t1));
    j.push(&polkit_failure(t1 + 3_600 * SECOND_US / 2));
    let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
        v["state"] == "active"
    })
    .await;
    assert_eq!(through_of(&v), 2);
    assert_ne!(epoch_of(&v), e1);
    let r = ack(&h, &e1, "1").await;
    assert_result(&r, 409, "stale_view");
    assert_eq!(wrong_count(&view(&h).await), 2, "nothing acknowledged");
    assert!(!env.ack_path().exists(), "nothing persisted");
}

/// Test 54 (RMC54, §4.3 rules R and M, F-3 b): a pending lock-screen check older than an
/// acknowledged record is shown unacknowledged once it resolves, the persisted marker stays
/// below it, and a restart shows it unacknowledged again.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_pending_check_older_than_ack_stays_unacknowledged() {
    let env = Env::new();
    let locker = env.locker();
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::with_locker(&env).socket("r1.sock")).await;
    wait_follows(&h, &j, 1, 1000).await;
    let t = now_us(&h) - 30 * SECOND_US;
    let lines = [
        // The lock screen's first failure (trusted, anchors the owner side).
        pam_unix_line(
            "swaylock",
            OWNER_UID,
            Some(&locker),
            OWNER,
            t - 20 * SECOND_US,
        ),
        chkpwd_line(OWNER_UID, OWNER, t - 20 * SECOND_US + 3 * MS_US),
        // A later check on the same lock-screen handle (helper only, pending for 2 s).
        chkpwd_line(OWNER_UID, OWNER, t),
        // A root-side attempt 1 s later (never pairs with an owner-side check).
        sudo_failure(t + SECOND_US),
    ];
    for line in &lines {
        j.push(line);
    }
    let v = view_settled(&h).await;
    assert_eq!(through_of(&v), 2, "the check is still pending: {v}");
    let r = ack(&h, &epoch_of(&v), "2").await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    assert_eq!(wrong_count(&r.json()), 0);
    assert!(
        ack_marker(&env) < t,
        "the marker stays below the pending check"
    );
    // The check resolves (wall-clock idle expiry) into an unacknowledged lock-screen record.
    let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| through_of(v) == 3).await;
    assert_eq!(wrong_count(&v), 1);
    let newest = &v["history"][0];
    assert_eq!(newest["source"], "lock_screen");
    // Contract migration (owner request 2026-10-06): records carry no acknowledged key.
    assert!(newest.get("acknowledged").is_none(), "{newest}");
    assert_eq!(newest["first_unix_ms"], t / 1000);
    h.advance_ms(2000).await;
    assert!(ack_marker(&env) < t);
    h.shutdown().await.unwrap();
    tokio::time::advance(ms(10_000)).await;

    // Restart: the check is shown unacknowledged again; the first failure is not shown.
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::with_locker(&env).socket("r2.sock")).await;
    wait_follows(&h, &j, 1, 1000).await;
    for line in &lines {
        j.push(line);
    }
    let (v, _) = view_within(&h, 4 * JOURNAL_IDLE_TICK_MS, 250, |v| {
        v["state"] == "active" && through_of(v) == 3
    })
    .await;
    // Contract migration (owner request 2026-10-06): the acknowledged first failure at
    // t − 20 s is at or below the marker and absent; the check at t is shown again; the
    // acknowledged sudo attempt at t + 1 s lies above the marker (< t) and is shown again
    // (rule-M fail-safe residual, spec R3.4, plan-evaluator F-3).
    let expected: BTreeMap<_, _> = [
        (key("lock_screen", "wrong_password"), 1),
        (key("sudo", "wrong_password"), 1),
    ]
    .into_iter()
    .collect();
    assert_eq!(counts(&v), expected, "{v}");
    let lock_screen: Vec<&Value> = v["history"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["source"] == "lock_screen")
        .collect();
    assert_eq!(lock_screen.len(), 1);
    assert_eq!(lock_screen[0]["first_unix_ms"], t / 1000);
    assert_eq!(through_of(&v), 3, "the discarded replay consumes a seq");
}

/// Test 60 (RMC75, spec R3.3 A′ / R3.4, owner request 2026-10-06 "once the acknowledge
/// button is clicked, delete the entries"): a successful acknowledgement removes the
/// acknowledged records from the `200` view, from `GET /api/alerts` and from the next
/// `event: alerts`; a later attempt is shown alone; after a restart the acknowledged
/// attempts never reappear and the file marker is unchanged.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_ack_clears_history_end_to_end() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled().socket("r1.sock")).await;
    until_active(&h, &j).await;
    let mut stream = h.open_stream_via(&Via::Tailnet).await.expect("stream");
    assert_sse_head(&stream.head);
    let (name, _) = frame_now(&mut stream).await;
    assert_eq!(name, "status");
    let (name, _) = frame_now(&mut stream).await;
    assert_eq!(name, "alerts");

    let t = now_us(&h);
    let acknowledged_lines = [sudo_failure(t), polkit_failure(t + SECOND_US)];
    for line in &acknowledged_lines {
        j.push(line);
    }
    let v = view_settled(&h).await;
    assert_eq!(through_of(&v), 2, "{v}");
    assert_eq!(wrong_count(&v), 2);
    assert_eq!(v["history"].as_array().unwrap().len(), 2);
    // Let the throttled `alerts` event of the two attempts go out.
    let (frames, eof) = frames_during(&mut stream, &h, ALERT_EVENT_MIN_INTERVAL_MS + 500, 50).await;
    assert!(!eof);
    let last = frames
        .iter()
        .rfind(|(_, n, _, _)| n == "alerts")
        .expect("an alerts event for the two attempts");
    assert_eq!(wrong_count(&last.2), 2);

    // Acknowledge what is displayed: the `200` view has no record left.
    let r = ack(&h, &epoch_of(&v), "2").await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    r.assert_mandatory_headers();
    r.assert_json_body();
    let acked = r.json();
    assert_view_shape(&acked);
    assert_eq!(acked["history"], json!([]), "{acked}");
    assert_eq!(wrong_count(&acked), 0);
    assert_eq!(locked_count(&acked), 0);
    assert_eq!(acked["last_unix_ms"], Value::Null);
    assert_eq!(acked["last_source"], Value::Null);
    assert_eq!(through_of(&acked), 2, "through unchanged");
    assert_eq!(ack_marker(&env), t + SECOND_US, "written at once");
    // `GET /api/alerts` shows the same.
    assert_eq!(view_settled(&h).await, acked);
    // The next `alerts` event shows the same.
    let (frames, eof) = frames_during(&mut stream, &h, ALERT_EVENT_MIN_INTERVAL_MS + 500, 50).await;
    assert!(!eof);
    let events: Vec<&Value> = frames
        .iter()
        .filter(|(_, n, _, _)| n == "alerts")
        .map(|(_, _, d, _)| d)
        .collect();
    assert!(
        !events.is_empty(),
        "the acknowledgement sends an alerts event"
    );
    for data in &events {
        assert_eq!(data["history"], json!([]), "{data}");
        assert_eq!(wrong_count(data), 0);
    }

    // A new attempt after the acknowledgement is shown alone.
    let t3 = now_us(&h);
    let newer = sudo_failure(t3);
    j.push(&newer);
    let v = view_settled(&h).await;
    assert_eq!(through_of(&v), 3);
    let history = v["history"].as_array().unwrap();
    assert_eq!(history.len(), 1, "{v}");
    assert_eq!(history[0]["id"], 3);
    assert_eq!(history[0]["source"], "sudo");
    assert_eq!(history[0]["count"], 1);
    assert_eq!(history[0]["first_unix_ms"], t3 / 1000);
    assert_eq!(wrong_count(&v), 1);
    let (frames, eof) = frames_during(&mut stream, &h, ALERT_EVENT_MIN_INTERVAL_MS + 500, 50).await;
    assert!(!eof);
    let last = frames
        .iter()
        .rfind(|(_, n, _, _)| n == "alerts")
        .expect("an alerts event for the new attempt");
    assert_eq!(last.2["history"].as_array().unwrap().len(), 1);
    h.advance_ms(3000).await;
    assert_eq!(ack_marker(&env), t + SECOND_US);
    drop(stream);
    h.shutdown().await.unwrap();
    tokio::time::advance(ms(10_000)).await;

    // Restart: the same journal lines are replayed; the acknowledged ones never reappear.
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled().socket("r2.sock")).await;
    wait_follows(&h, &j, 1, 1000).await;
    for line in &acknowledged_lines {
        j.push(line);
    }
    j.push(&newer);
    let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
        v["state"] == "active"
    })
    .await;
    let expected: BTreeMap<_, _> = [(key("sudo", "wrong_password"), 1)].into_iter().collect();
    assert_eq!(counts(&v), expected, "{v}");
    let history = v["history"].as_array().unwrap();
    assert_eq!(history.len(), 1, "{v}");
    assert_eq!(history[0]["first_unix_ms"], t3 / 1000);
    assert_eq!(history[0]["id"], 3, "discarded replays consume a seq");
    assert_eq!(through_of(&v), 3);
    assert_eq!(wrong_count(&v), 1);
    h.advance_ms(3000).await;
    assert_eq!(
        ack_marker(&env),
        t + SECOND_US,
        "the file marker is unchanged"
    );
}

/// Test 55 (RMC52, A-13): a failing CSPRNG leaves alerts `unavailable` / `rng_failed` with
/// no epoch, the journal is never probed, the acknowledgement is `503 unavailable`, and
/// status and lock are unaffected.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_rng_failure_is_unavailable() {
    let env = Env::with_random(failing_random());
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
    h.step_ms(500, 5000).await;
    let v = view_settled(&h).await;
    assert_eq!(
        state_of(&v),
        ("unavailable".to_string(), json!("rng_failed"))
    );
    assert!(v["epoch"].is_null());
    assert_eq!(through_of(&v), 0);
    assert_eq!(j.probes(), 0);
    assert_eq!(j.follows(), 0);
    assert_result(&ack(&h, "0000000000000000", "0").await, 503, "unavailable");
    let status = h.get("/api/status").await;
    assert_eq!(status.status, 200);
    assert_status_shape(&status.json());
    assert_eq!(h.lock(&[]).await.status, 202);
}

// ---------------------------------------------------------------------------------------
// Test 32 — SSE
// ---------------------------------------------------------------------------------------

/// Test 32 (RMC55): the first `status` event, then one `alerts` event; a burst of attempts
/// gives at most one `alerts` event per second with the final counts; the status keep-alive
/// timing is unchanged.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_sse_event_order_and_throttle() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
    until_active(&h, &j).await;
    let mut stream = h.open_stream_via(&Via::Tailnet).await.expect("stream");
    assert_sse_head(&stream.head);
    let (name, status) = frame_now(&mut stream).await;
    assert_eq!(name, "status");
    assert_eq!(status["state"], "unlocked");
    let (name, alerts) = frame_now(&mut stream).await;
    assert_eq!(name, "alerts");
    assert_eq!(alerts["state"], "active");
    assert_eq!(wrong_count(&alerts), 0);

    // Ten attempts in 100 ms.
    let mut all = Vec::new();
    let mut offset = 0;
    for i in 0..10u64 {
        j.push(&sudo_failure(now_us(&h) + i));
        let (frames, eof) = frames_during(&mut stream, &h, 10, 10).await;
        assert!(!eof);
        all.extend(frames.into_iter().map(|(t, n, d, _)| (t + offset, n, d)));
        offset += 10;
    }
    let (frames, eof) = frames_during(&mut stream, &h, 2000, 10).await;
    assert!(!eof);
    all.extend(frames.into_iter().map(|(t, n, d, _)| (t + offset, n, d)));
    offset += 2000;
    let alert_events: Vec<&(u64, String, Value)> =
        all.iter().filter(|(_, n, _)| n == "alerts").collect();
    assert!(
        !alert_events.is_empty() && alert_events.len() <= 2,
        "at most 2 alerts events for the burst, got {}",
        alert_events.len()
    );
    assert_eq!(
        wrong_count(&alert_events.last().unwrap().2),
        10,
        "final counts"
    );
    // At most one per second, counting from the first alerts event at t = 0.
    let mut previous = 0;
    for (t, _, _) in &alert_events {
        assert!(
            *t + 10 >= previous + ALERT_EVENT_MIN_INTERVAL_MS,
            "alerts events {previous} ms and {t} ms are closer than the throttle"
        );
        previous = *t;
    }
    assert!(
        alert_events.last().unwrap().0 <= 100 + ALERT_EVENT_MIN_INTERVAL_MS + 50,
        "the newest view is sent at the end of the interval"
    );
    // Status keep-alive: the next status event comes SSE_KEEPALIVE_MS after the first one,
    // alerts events never reset it.
    let (frames, eof) = frames_during(&mut stream, &h, SSE_KEEPALIVE_MS + 1000 - offset, 50).await;
    assert!(!eof);
    let status_times: Vec<u64> = frames
        .iter()
        .filter(|(_, n, _, _)| n == "status")
        .map(|(t, _, _, _)| t + offset)
        .collect();
    assert!(
        all.iter().all(|(_, n, _)| n != "status"),
        "no status event during the burst"
    );
    assert_eq!(status_times.len(), 1, "{status_times:?}");
    assert!(
        (SSE_KEEPALIVE_MS - 1000..=SSE_KEEPALIVE_MS + 1000).contains(&status_times[0]),
        "keep-alive at {} ms",
        status_times[0]
    );
}

// ---------------------------------------------------------------------------------------
// Tests 33–36 — hygiene and isolation
// ---------------------------------------------------------------------------------------

/// Test 33 (RMC48, O-2): no owner login, typed user name, service name, executable path,
/// cursor, foreign uid or raw message text in any response body, SSE byte or log line;
/// every view object has the exact key sets.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_never_expose_raw_fields() {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let env = Env::with_random(fixed_random());
    let locker = env.locker();
    let j = ScriptedJournal::new();
    let h = start_alerts(&env, &j, AlertOptions::with_locker(&env)).await;
    wait_follows(&h, &j, 1, 1000).await;
    let mut stream = h.open_stream_via(&Via::Tailnet).await.expect("stream");
    let mut seen = Vec::<String>::new();
    let now = now_us(&h) / SECOND_US * SECOND_US;
    let typed = "Tr0ub4dor&3xyz";
    let lines = vec![
        pam_unix_line(
            "gdm-password",
            0,
            Some(GDM_WORKER_EXE),
            typed,
            now - 50 * SECOND_US,
        ),
        chkpwd_line(0, typed, now - 50 * SECOND_US + MS_US),
        pam_unix_line(
            "swaylock",
            OWNER_UID,
            Some(&locker),
            OWNER,
            now - 40 * SECOND_US,
        ),
        chkpwd_line(OWNER_UID, OWNER, now - 40 * SECOND_US + MS_US),
        chkpwd_line(3_141_592_653, OWNER, now - 35 * SECOND_US),
        pam_unix_line(
            "polkit-1",
            0,
            Some(POLKIT_HELPER_EXE),
            OWNER,
            now - 30 * SECOND_US,
        )
        .with_str("__CURSOR", "s=needlecursor;i=7"),
        faillock_locked_line(
            "gdm-password",
            0,
            Some(GDM_WORKER_EXE),
            OWNER,
            now - 20 * SECOND_US,
        ),
        pam_unix_line(
            "sudo",
            OWNER_UID,
            Some(SUDO_EXE),
            "root",
            now - 10 * SECOND_US,
        ),
    ];
    for line in &lines {
        j.push(line);
    }
    let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
        v["state"] == "active"
    })
    .await;
    assert_eq!(wrong_count(&v), 4);
    assert_eq!(locked_count(&v), 1);
    seen.push(v.to_string());
    let r = ack(&h, &epoch_of(&v), &through_of(&v).to_string()).await;
    assert_eq!(r.status, 200);
    seen.push(String::from_utf8_lossy(&r.body).into_owned());
    let (frames, _) = frames_during(&mut stream, &h, 3000, 250).await;
    assert!(frames.iter().any(|(_, n, _, _)| n == "alerts"));
    for (_, _, data, raw) in &frames {
        seen.push(raw.clone());
        if data.get("history").is_some() {
            assert_view_shape(data);
        }
    }
    let funnel = h.session(IP_A).await;
    seen.push(view_via(&h, &funnel).await.to_string());
    seen.push(capture.text());

    let needles = [
        OWNER.to_string(),
        "Tr0ub4dor".to_string(),
        "3xyz".to_string(),
        "gdm-password".to_string(),
        "swaylock".to_string(),
        "polkit-1".to_string(),
        "unix_chkpwd".to_string(),
        "pam_unix".to_string(),
        "pam_faillock".to_string(),
        GDM_WORKER_EXE.to_string(),
        POLKIT_HELPER_EXE.to_string(),
        SUDO_EXE.to_string(),
        locker.clone(),
        "needlecursor".to_string(),
        cursor_for(now - 50 * SECOND_US),
        "3141592653".to_string(),
        "authentication failure".to_string(),
        "password check failed".to_string(),
        "rhost".to_string(),
        "logname".to_string(),
        "tty=".to_string(),
        "_EXE".to_string(),
        "SYSLOG".to_string(),
        "MESSAGE".to_string(),
    ];
    for text in &seen {
        for needle in &needles {
            assert!(
                !text.contains(needle.as_str()),
                "leak of {needle:?} in {}",
                &text[..text.len().min(200)]
            );
        }
    }
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

/// Test 34 (RMC56, A-11): exactly one `password alerts active` (INFO) and one `password
/// alerts unavailable` (WARN) per transition, one `password alert acknowledgement not
/// persisted` (WARN) per failure transition and per invalid ack file; none carries a field.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_audit_lines() {
    {
        let capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(capture.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let env = Env::new();
        let j = ScriptedJournal::new();
        j.set_probe(Err(JournalError::NoAccess));
        let unwritable = env.path("no-such-dir").join(ALERTS_ACK_FILE_NAME);
        let h = start_alerts(
            &env,
            &j,
            AlertOptions {
                ack_path: AckPath::Custom(unwritable),
                ..AlertOptions::enabled()
            },
        )
        .await;
        pump().await;
        h.step_ms(500, 7_500).await;
        assert!(j.probes() >= 3);
        let text = capture.text();
        assert_eq!(
            count_lines(&text, "password alerts unavailable"),
            1,
            "{text}"
        );
        assert_eq!(count_lines(&text, "password alerts active"), 0);
        j.set_probe(Ok(()));
        wait_follows(&h, &j, 1, 10_000).await;
        view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
            v["state"] == "active"
        })
        .await;
        let text = capture.text();
        assert_eq!(count_lines(&text, "password alerts active"), 1, "{text}");
        j.end();
        pump().await;
        let text = capture.text();
        assert_eq!(
            count_lines(&text, "password alerts unavailable"),
            2,
            "{text}"
        );
        wait_follows(&h, &j, 2, JOURNAL_RESTART_MAX_MS + 1000).await;
        view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
            v["state"] == "active"
        })
        .await;
        assert_eq!(count_lines(&capture.text(), "password alerts active"), 2);

        // The ack file cannot be written: `200` and one WARN per failure transition.
        j.push(&sudo_failure(now_us(&h)));
        let v = view_settled(&h).await;
        let r = ack(&h, &epoch_of(&v), "1").await;
        assert_eq!(r.status, 200, "{}", r.result_or_body());
        assert_eq!(wrong_count(&r.json()), 0, "acknowledged in memory");
        h.advance_ms(MIN_ALERT_ACK_INTERVAL_MS + 1).await;
        j.push(&polkit_failure(now_us(&h)));
        let v = view_settled(&h).await;
        assert_eq!(ack(&h, &epoch_of(&v), "2").await.status, 200);
        h.advance_ms(3000).await;
        let text = capture.text();
        assert_eq!(
            count_lines(&text, "password alert acknowledgement not persisted"),
            1,
            "{text}"
        );
        assert_audit_lines(&text, "password alerts active", "INFO");
        assert_audit_lines(&text, "password alerts unavailable", "WARN");
        assert_audit_lines(
            &text,
            "password alert acknowledgement not persisted",
            "WARN",
        );
    }
    {
        // An invalid ack file at start: one WARN, nothing acknowledged.
        let capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(capture.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let env = Env::new();
        write_file_mode(&env.ack_path(), b"{\"version\":2}", 0o600);
        let j = ScriptedJournal::new();
        let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
        wait_follows(&h, &j, 1, 1000).await;
        j.push(&sudo_failure(now_us(&h) - HOUR_US));
        let (v, _) = view_within(&h, 3 * JOURNAL_IDLE_TICK_MS, 250, |v| {
            v["state"] == "active"
        })
        .await;
        assert_eq!(wrong_count(&v), 1, "fail-safe: more alerts, never fewer");
        let text = capture.text();
        assert_eq!(
            count_lines(&text, "password alert acknowledgement not persisted"),
            1,
            "{text}"
        );
        assert_audit_lines(
            &text,
            "password alert acknowledgement not persisted",
            "WARN",
        );
    }
}

/// Test 35 (RMC51, RMC52, §7): with the reader unavailable, or ending again and again,
/// status, status events and lock behave as with alerts disabled and `serve` keeps running.
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_unavailable_never_affects_status_or_lock() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    j.set_probe(Err(JournalError::NotFound));
    let h = start_alerts(&env, &j, AlertOptions::enabled()).await;
    // 9 s: three retries, and a `checked_unix_ms` without the digits of the uid.
    h.step_ms(500, 9_000).await;
    assert_eq!(view_settled(&h).await["state"], "unavailable");
    let status = h.get("/api/status").await;
    assert_eq!(status.status, 200);
    assert_status_shape(&status.json());
    assert_eq!(status.json()["state"], "unlocked");
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 202);
    assert_eq!(r.result(), "lock_requested");
    let mut stream = h.open_stream_via(&Via::Tailnet).await.expect("stream");
    assert_eq!(frame_now(&mut stream).await.0, "status");
    assert_eq!(frame_now(&mut stream).await.0, "alerts");
    h.source.set_locked();
    let (frames, eof) = frames_during(&mut stream, &h, 2 * DEFAULT_POLL_INTERVAL_MS, 250).await;
    assert!(!eof);
    assert!(
        frames
            .iter()
            .any(|(_, n, d, _)| n == "status" && d["state"] == "locked"),
        "status events unaffected"
    );
    // The reader now starts and ends repeatedly.
    j.set_probe(Ok(()));
    for round in 1..=5 {
        wait_follows(&h, &j, round, 120_000).await;
        j.end();
        pump().await;
        assert!(
            !h.server.as_ref().unwrap().is_finished(),
            "serve keeps running"
        );
    }
    h.source.set_unlocked();
    h.advance_ms(3000).await;
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 202, "{}", r.result_or_body());
    assert_eq!(h.get("/api/status").await.status, 200);
    assert!(!h.server.as_ref().unwrap().is_finished());
}

/// Test 36 (RMC55, A-12): with alerts enabled and lines flowing, status, lock and status
/// events behave as in `server_tests.rs` (spot checks).
#[tokio::test(start_paused = true)]
async fn test_rmc_alerts_existing_behaviour_unchanged_when_enabled() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    // No credential directory for the ack file: acknowledgements live in memory only.
    let h = start_alerts(
        &env,
        &j,
        AlertOptions {
            ack_path: AckPath::Absent,
            ..AlertOptions::enabled()
        },
    )
    .await;
    until_active(&h, &j).await;
    let mut stream = h.open_stream_via(&Via::Tailnet).await.expect("stream");
    let (name, first) = frame_now(&mut stream).await;
    assert_eq!(name, "status", "the first event is still a status event");
    assert_eq!(first["state"], "unlocked");
    for i in 0..5u64 {
        j.push(&sudo_failure(now_us(&h) + i));
        j.push(&process_line(
            0,
            Some(SUDO_EXE),
            "pam_unix(sudo:session): session closed for user root",
            now_us(&h) + i,
        ));
    }
    let status = h.get("/api/status").await;
    assert_eq!(status.status, 200);
    status.assert_json_body();
    assert_status_shape(&status.json());
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 202);
    assert_eq!(r.result(), "lock_requested");
    assert_eq!(h.source.lock_ids(), vec![SESSION_ID.to_string()]);
    assert_result(&h.lock(&[]).await, 429, "rate_limited");
    h.source.set_locked();
    let (frames, eof) = frames_during(&mut stream, &h, 2 * DEFAULT_POLL_INTERVAL_MS, 250).await;
    assert!(!eof);
    let statuses: Vec<&Value> = frames
        .iter()
        .filter(|(_, n, _, _)| n == "status")
        .map(|(_, _, d, _)| d)
        .collect();
    assert!(
        statuses.iter().any(|d| d["state"] == "locked"),
        "{statuses:?}"
    );
    for d in statuses {
        assert_status_shape(d);
    }
    let alerts = frames.iter().filter(|(_, n, _, _)| n == "alerts").count();
    assert!(alerts >= 1, "alerts events still flow");
    // In-memory acknowledgement without an ack file.
    let v = view(&h).await;
    let r = ack(&h, &epoch_of(&v), &through_of(&v).to_string()).await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    assert_eq!(wrong_count(&r.json()), 0);
    assert!(
        !env.ack_path().exists(),
        "nothing written without an ack path"
    );
    // Unknown routes and assets are unchanged.
    assert_eq!(h.get("/api/nope").await.status, 404);
    assert_eq!(h.get("/").await.status, 200);
}

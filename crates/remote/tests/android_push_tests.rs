//! Android contract tests of Web Push in `soos-remote` (GitHub #349, ADR 2026-10-09 "Android
//! Support for the `soos-remote` Phone Companion and Web Push", architect spec
//! `AI/architect_spec_remote_android.md` §2.2, §3.3 and §11.2; matrix RAN2, RAN8, RAN9).
//!
//! Same deterministic harness as `push_server_tests.rs` (frozen paused clock, scripted logind,
//! raw HTTP/1.1 over the Unix socket in a `TempDir`, the owner's passkey for Funnel sessions,
//! a `ScriptedJournal` for live failed passwords and a `FakeTransport` for the sender): no
//! network, no push service.
//!
//! - `test_ran_push_service_maps_each_host` is red until `PushService::of` is public and
//!   matches `PushHost` exhaustively (spec §2.2).
//! - The other tests are coverage contracts of behaviour the issue keeps (spec §0.1, §3.3):
//!   Android endpoints end to end, the Chrome `toJSON()` shape, and the service-worker request
//!   accepted exactly where the page's request is, refused by the unchanged checks elsewhere.

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
use push::*;
use soos_push_protocol::{Outcome, PushEndpoint, Urgency};
use soos_remote::alerts::AlertSettings;
use soos_remote::auth::{RandomError, RandomSource};
use soos_remote::config::{
    AlertsConfig, AuthConfig, PushConfig, PushPreviews, RemoteConfig, TailscaleLogin,
};
use soos_remote::journal::OwnerLogin;
use soos_remote::push::{PushService, PushSettings, PushStore};
use soos_remote::server::{serve, ServerState};
use soos_remote::{
    ALERTS_ACK_FILE_NAME, CREDENTIALS_FILE_NAME, DEFAULT_POLL_INTERVAL_MS, JOURNAL_IDLE_TICK_MS,
    PUSH_COALESCE_MS, PUSH_ROUTE_MIN_INTERVAL_MS, PUSH_STORE_FILE_NAME, WEB_SESSION_IDLE_MS,
};

const IP_A: &str = "203.0.113.10";
const SUBJECT: &str = "https://pc.tail1234.ts.net";

/// Chrome / Samsung Internet legacy FCM endpoint (`/fcm/send/`).
const FCM_SEND: &str = "https://fcm.googleapis.com/fcm/send/eT4wQx9-R_k:APA91bF0q8mD2nK7-vL3_pZ9sXyWcE5tH1uJ6oR4iA8gB2dN0mQ7kS3fV9xC1zY5wT6rE2_uI8oP4aL0jH7gD3sK9nM5bV1cX6zQ2wE8rT4yU0iO7pA3sD";
/// Chrome current FCM endpoint (`/wp/`).
const FCM_WP: &str = "https://fcm.googleapis.com/wp/fXy7Kd2pQ1s:APA91bE8rT4yU0iO7pA3sD9fG2hJ5kL1zX6cV8bN3mQ0wE4rT7yU2iO5pA9sD1fG6hJ3kL8zX0cV4bN7mQ2wE5rT";
/// Firefox for Android autopush v1.
const MOZILLA_V1: &str = "https://updates.push.services.mozilla.com/wpush/v1/gAAAAABl9x2Kq7Lm3Np8Rs1Tu4Vw6Xy0Za5Bc9De2Fg7Hi1Jk4Lm8No3Pq6Rs0Tu5Vw9Xy2Za7Bc1De4Fg8Hi3Jk6Lm0No5Pq9Rs2Tu7Vw";
/// Firefox for Android autopush v2 (base64url ending `=`).
const MOZILLA_V2: &str = "https://updates.push.services.mozilla.com/wpush/v2/gAAAAABm1a2Bb3Cc4Dd5Ee6Ff7Gg8Hh9Ii0Jj1Kk2Ll3Mm4Nn5Oo6Pp7Qq8Rr9Ss0Tt1Uu2Vv3Ww4Xx5Yy6Zz7_-aB3cD4eF5gH6iJ7kL8mN9o=";

/// The headers a same-origin service-worker `fetch` carries besides the harness's `Via`
/// headers and the automatic `Content-Type: application/json` (spec §3.3).
const WORKER_HEADERS: [(&str, &str); 5] = [
    ("X-Soos-Action", "push-subscribe"),
    ("Origin", ORIGIN),
    ("Sec-Fetch-Site", "same-origin"),
    ("Sec-Fetch-Mode", "cors"),
    ("Sec-Fetch-Dest", "empty"),
];

// ---------------------------------------------------------------------------------------
// Push harness (the push_server_tests.rs harness, reduced to what this suite needs)
// ---------------------------------------------------------------------------------------

struct Env {
    dir: TempDir,
    clock: Arc<TestClock>,
    random: RandomSource,
}

impl Env {
    fn new() -> Self {
        let counter = Arc::new(AtomicU64::new(1));
        let random: RandomSource = Arc::new(move |buf: &mut [u8]| -> Result<(), RandomError> {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            let seed = sha256(&n.to_be_bytes());
            for (i, b) in buf.iter_mut().enumerate() {
                *b = seed[i % 32] ^ (i / 32) as u8;
            }
            Ok(())
        });
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

    fn locker(&self) -> String {
        let path = self.path("swaylock-plugin");
        if !path.exists() {
            fs::write(&path, b"#!/bin/false\n").unwrap();
        }
        path.to_string_lossy().into_owned()
    }
}

/// Starts `serve` with alerts and push (detailed previews) wired to `transport`; Funnel and
/// passkeys configured (the owner's passkey stored); unlock off.
async fn start_push(env: &Env, journal: &ScriptedJournal, transport: &FakeTransport) -> Harness {
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
        enabled: true,
        lock_screen_programs: vec![env.locker()],
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
        push: PushConfig {
            enabled: true,
            vapid_subject: None,
            socket_path: None,
            previews: PushPreviews::Detailed,
        },
        camera: soos_remote::config::CameraConfig::default(),
        battery: soos_remote::config::BatteryConfig::default(),
    };
    let alert_settings = AlertSettings {
        owner_login: Some(OwnerLogin::parse(OWNER).unwrap()),
        lock_screen_programs: alerts.lock_screen_programs.clone(),
        ack_path: Some(env.path(ALERTS_ACK_FILE_NAME)),
    };
    let push_settings = PushSettings {
        store_path: env.push_store(),
        subject: SUBJECT.to_string(),
        previews: PushPreviews::Detailed,
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

async fn pump() {
    for _ in 0..POLL_ROUNDS {
        yield_now().await;
    }
}

fn assert_result(r: &HttpResponse, status: u16, result: &str) {
    assert_eq!(
        (r.status, r.result_or_body()),
        (status, result.to_string()),
        "expected {status} {result}"
    );
    r.assert_mandatory_headers();
}

async fn push_view(h: &Harness) -> Value {
    pump().await;
    let r = h.get_via(&Via::Tailnet, "/api/push").await;
    assert_eq!(r.status, 200, "GET /api/push: {}", r.result_or_body());
    r.json()
}

/// Waits out the shared subscribe/unsubscribe gate.
async fn gap(h: &Harness) {
    h.advance_ms(PUSH_ROUTE_MIN_INTERVAL_MS + 1).await;
}

/// The page's subscribe request (`X-Soos-Action` + `Origin`), then the gate is waited out.
async fn page_subscribe(h: &Harness, via: &Via, body: &str) -> HttpResponse {
    let r = h
        .send(
            via,
            "POST",
            "/api/push/subscribe",
            &[("X-Soos-Action", "push-subscribe"), ("Origin", ORIGIN)],
            Some(body),
        )
        .await;
    gap(h).await;
    r
}

/// The service worker's subscribe request with `headers`, then the gate is waited out.
async fn worker_subscribe(
    h: &Harness,
    via: &Via,
    headers: &[(&str, &str)],
    body: &str,
) -> HttpResponse {
    let r = h
        .send(via, "POST", "/api/push/subscribe", headers, Some(body))
        .await;
    gap(h).await;
    r
}

/// Starts, waits for the journal follower and for alerts `active`.
async fn until_active(h: &Harness, j: &ScriptedJournal) {
    let mut elapsed = 0;
    loop {
        pump().await;
        if j.follows() >= 1 {
            let r = h.get("/api/alerts").await;
            if r.status == 200 && r.json()["state"] == "active" {
                return;
            }
        }
        assert!(
            elapsed < 3 * JOURNAL_IDLE_TICK_MS + 1000,
            "alerts not active"
        );
        h.advance_ms(250).await;
        elapsed += 250;
    }
}

/// Moves the clock until the transport saw `n` calls.
async fn wait_calls(h: &Harness, t: &FakeTransport, n: usize, max_ms: u64) {
    let mut elapsed = 0;
    loop {
        pump().await;
        if t.count() >= n {
            return;
        }
        assert!(
            elapsed < max_ms,
            "{n} calls not reached within {max_ms} ms ({} so far)",
            t.count()
        );
        h.advance_ms(100).await;
        elapsed += 100;
    }
}

/// Polls `GET /api/push` (real-time sleeps for blocking-pool store work) until `done`.
async fn wait_view(h: &Harness, done: impl Fn(&Value) -> bool) -> Value {
    let mut view = push_view(h).await;
    for _ in 0..200 {
        if done(&view) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        pump().await;
        view = push_view(h).await;
    }
    view
}

fn stored_endpoints(env: &Env) -> Vec<String> {
    PushStore::new(env.push_store(), own_uid())
        .load()
        .unwrap()
        .expect("the store exists")
        .subscriptions
        .iter()
        .map(|s| s.endpoint.as_str().to_string())
        .collect()
}

fn chrome_body(endpoint: &str, ua: &UaFixture, keys_first: bool) -> String {
    let (p, a) = (ua.p256dh_b64(), ua.auth_b64());
    if keys_first {
        format!(
            "{{\"keys\":{{\"p256dh\":\"{p}\",\"auth\":\"{a}\"}},\"endpoint\":\"{endpoint}\",\"expirationTime\":null}}"
        )
    } else {
        format!(
            "{{\"endpoint\":\"{endpoint}\",\"expirationTime\":null,\"keys\":{{\"p256dh\":\"{p}\",\"auth\":\"{a}\"}}}}"
        )
    }
}

// ---------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------

/// RAN8 (spec §2.2): `PushService::of` maps each host to its own service (no fallback arm);
/// the JSON names are unchanged.
#[test]
fn test_ran_push_service_maps_each_host() {
    let apple = PushEndpoint::parse(
        "https://web.push.apple.com/QGuQyavXutnMH8l2ce1fmbpFBjgLv9SQK3-ASsYlvuDH",
    )
    .unwrap();
    let cases = [
        (apple, PushService::Apple, "apple"),
        (
            PushEndpoint::parse(FCM_SEND).unwrap(),
            PushService::Google,
            "google",
        ),
        (
            PushEndpoint::parse(FCM_WP).unwrap(),
            PushService::Google,
            "google",
        ),
        (
            PushEndpoint::parse(MOZILLA_V1).unwrap(),
            PushService::Mozilla,
            "mozilla",
        ),
        (
            PushEndpoint::parse(MOZILLA_V2).unwrap(),
            PushService::Mozilla,
            "mozilla",
        ),
    ];
    for (endpoint, service, name) in cases {
        let of = PushService::of(&endpoint);
        assert_eq!(of, service, "{}", endpoint.host());
        assert_eq!(serde_json::to_value(of).unwrap(), json!(name));
    }
}

/// RAN9 (spec §11.2, coverage): the four Android endpoints subscribe, list as `google`,
/// `google`, `mozilla`, `mozilla`, round-trip through the store, each receive one high-urgency
/// request with the VAPID `aud` of their origin and a payload that decrypts with their key;
/// a `410` removes that endpoint only.
#[tokio::test(start_paused = true)]
async fn test_ran_endpoints_subscribe_store_and_dispatch() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t).await;
    until_active(&h, &j).await;
    let endpoints = [FCM_SEND, FCM_WP, MOZILLA_V1, MOZILLA_V2];
    let mut uas = Vec::new();
    for (i, endpoint) in endpoints.iter().enumerate() {
        let ua = UaFixture::new(i as u8 + 1);
        let r = page_subscribe(&h, &Via::Tailnet, &subscribe_body(endpoint, &ua)).await;
        assert_result(&r, 200, "subscribed");
        uas.push(ua);
    }
    let view = push_view(&h).await;
    assert_eq!(view["state"], "active", "{view}");
    assert_eq!(view["subscriptions"], 4, "{view}");
    let mut services: Vec<String> = view["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["service"].as_str().unwrap().to_string())
        .collect();
    services.sort();
    assert_eq!(services, ["google", "google", "mozilla", "mozilla"]);
    let public_key = view["public_key"].as_str().unwrap().to_string();

    // The store round-trips the endpoints and keys.
    let store = PushStore::new(env.push_store(), own_uid())
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(store.subscriptions.len(), 4);
    for ((endpoint, ua), stored) in endpoints.iter().zip(&uas).zip(&store.subscriptions) {
        assert_eq!(stored.endpoint.as_str(), *endpoint);
        assert!(
            stored.keys.p256dh == ua.ua_keys().p256dh,
            "{endpoint}: p256dh"
        );
        assert_eq!(*stored.keys.auth, *ua.ua_keys().auth, "{endpoint}: auth");
    }

    // One live failed password: one request per endpoint; the first answer is 410.
    t.queue(&[reply(Outcome::Gone, Some(410), None)]);
    let base = h.clock.now_ms() * 1000;
    j.push(&pam_unix_line(
        "sudo",
        OWNER_UID,
        Some(SUDO_EXE),
        OWNER,
        base,
    ));
    pump().await;
    wait_calls(&h, &t, 4, PUSH_COALESCE_MS + JOURNAL_IDLE_TICK_MS + 10_000).await;
    let calls = t.calls();
    assert_eq!(calls.len(), 4, "one request per subscription");
    for (endpoint, ua) in endpoints.iter().zip(&uas) {
        let call = t.calls_to(endpoint);
        assert_eq!(call.len(), 1, "{endpoint}");
        let req = &call[0].request;
        assert_eq!(req.endpoint.as_str(), *endpoint);
        assert_eq!(req.urgency, Urgency::High, "{endpoint}");
        let vapid = verify_vapid(&req.authorization, Some(&public_key));
        let claims: Value = serde_json::from_str(&vapid.claims).unwrap();
        let origin = PushEndpoint::parse(endpoint).unwrap().origin();
        assert!(
            origin == "https://fcm.googleapis.com"
                || origin == "https://updates.push.services.mozilla.com"
        );
        assert_eq!(claims["aud"], origin.as_str(), "{endpoint}");
        assert_eq!(claims["sub"], SUBJECT);
        let payload = decrypt_json(&call[0].request.body, ua);
        assert_eq!(payload["soos"]["kind"], "alerts", "{endpoint}");
        assert_eq!(payload["soos"]["wrong_password"], 1, "{endpoint}");
    }

    // The 410 removed that endpoint only.
    let gone = calls[0].endpoint().to_string();
    let view = wait_view(&h, |v| v["subscriptions"] == 3).await;
    assert_eq!(view["subscriptions"], 3, "{view}");
    let left: BTreeSet<String> = stored_endpoints(&env).into_iter().collect();
    let expected: BTreeSet<String> = endpoints
        .iter()
        .filter(|e| **e != gone)
        .map(|e| (*e).to_string())
        .collect();
    assert_eq!(left, expected, "only the gone endpoint is removed");
}

/// RAN9 (spec §11.2, coverage): Chrome's exact `toJSON()` shape, and the same members in
/// another order, are accepted.
#[tokio::test(start_paused = true)]
async fn test_ran_chrome_subscription_json_parses() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t).await;
    let second = format!("{FCM_WP}2");
    for (endpoint, keys_first) in [(FCM_WP, false), (second.as_str(), true)] {
        let ua = ua_keys();
        let r = page_subscribe(&h, &Via::Tailnet, &chrome_body(endpoint, &ua, keys_first)).await;
        assert_result(&r, 200, "subscribed");
    }
    assert_eq!(
        stored_endpoints(&env),
        vec![FCM_WP.to_string(), second.clone()]
    );
    assert_eq!(push_view(&h).await["subscriptions"], 2);
}

/// RAN2 (spec §3.3, coverage): the worker's request (page headers plus the browser's
/// `Sec-Fetch-*`) is accepted from the tailnet and over Funnel with a valid session.
#[tokio::test(start_paused = true)]
async fn test_ran_worker_resubscribe_shape_is_accepted() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t).await;
    let ua = ua_keys();
    let r = worker_subscribe(
        &h,
        &Via::Tailnet,
        &WORKER_HEADERS,
        &chrome_body(FCM_WP, &ua, false),
    )
    .await;
    assert_result(&r, 200, "subscribed");

    let session = h.session(IP_A).await;
    let renewed = format!("{FCM_SEND}renewed");
    let r = worker_subscribe(
        &h,
        &session,
        &WORKER_HEADERS,
        &chrome_body(&renewed, &ua, false),
    )
    .await;
    assert_result(&r, 200, "subscribed");
    assert_eq!(
        stored_endpoints(&env),
        vec![FCM_WP.to_string(), renewed.clone()]
    );
    assert_eq!(t.count(), 0, "subscribing sends nothing");
}

/// RAN2 (spec §3.3, coverage; regression guard that nothing was loosened): over Funnel
/// without a session or with an expired one the worker's request is `403 login_required`;
/// `Sec-Fetch-Site: same-site`/`cross-site` and a missing `X-Soos-Action` are
/// `403 forbidden`; nothing is stored.
#[tokio::test(start_paused = true)]
async fn test_ran_worker_resubscribe_without_funnel_session_is_refused() {
    let env = Env::new();
    let j = ScriptedJournal::new();
    let t = FakeTransport::new();
    let h = start_push(&env, &j, &t).await;
    let ua = ua_keys();
    let body = chrome_body(FCM_WP, &ua, false);

    // No session.
    let anonymous = Via::funnel(IP_A);
    let r = worker_subscribe(&h, &anonymous, &WORKER_HEADERS, &body).await;
    assert_result(&r, 403, "login_required");

    // An expired session (idle for WEB_SESSION_IDLE_MS).
    let session = h.session(IP_A).await;
    h.advance_ms(WEB_SESSION_IDLE_MS).await;
    let r = worker_subscribe(&h, &session, &WORKER_HEADERS, &body).await;
    assert_result(&r, 403, "login_required");

    // A cross-site or same-site initiator, from the tailnet and with a fresh session.
    let fresh = h.session(IP_A).await;
    for via in [Via::Tailnet, fresh.clone()] {
        for site in ["same-site", "cross-site"] {
            let headers = [
                ("X-Soos-Action", "push-subscribe"),
                ("Origin", ORIGIN),
                ("Sec-Fetch-Site", site),
                ("Sec-Fetch-Mode", "cors"),
                ("Sec-Fetch-Dest", "empty"),
            ];
            let r = worker_subscribe(&h, &via, &headers, &body).await;
            assert_result(&r, 403, "forbidden");
        }
        // No X-Soos-Action.
        let r = worker_subscribe(&h, &via, &WORKER_HEADERS[1..], &body).await;
        assert_result(&r, 403, "forbidden");
    }

    let stored = PushStore::new(env.push_store(), own_uid()).load().unwrap();
    assert!(
        stored.is_none_or(|s| s.subscriptions.is_empty()),
        "nothing was stored"
    );
    assert_eq!(t.count(), 0);
}

//! End-to-end contract tests of the offline history and the back-online push in
//! `soos-remote::server::serve` (GitHub #376, tier 1 of #359 Part B; ADR 2026-10-13
//! "Offline History and Back-Online Push Notifications in `soos-remote`"; architect spec
//! `AI/architect_spec_remote_offline_alerts.md` §10.5, tests 22–30; matrix ROA1, ROA4–ROA10).
//!
//! Same deterministic harness as `push_server_tests.rs` (frozen paused clock, scripted
//! logind, raw HTTP/1.1 over the Unix socket in a `TempDir`, [`FakeTransport`] for the push
//! sender) plus a fake bus ([`FakeNetwork`], the `NetworkSource` seam: scripted
//! NetworkManager readings and logind `PreparingForShutdown` answers, every call counted)
//! and a fake clock ([`FakeClock`], the `OfflineClock` seam: wall, `CLOCK_BOOTTIME` and
//! `CLOCK_MONOTONIC` derived from the paused tokio clock, with an injectable suspend). No
//! real bus, clock, sender or push service is touched.

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

use std::fs;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::UnixListener;
use tokio::sync::{oneshot, watch};
use tokio::task::yield_now;
use tokio::time::{timeout, Instant};

use harness::*;
use journal::*;
use passkey::*;
use push::*;
use soos_remote::alerts::AlertSettings;
use soos_remote::auth::{RandomError, RandomSource};
use soos_remote::config::{
    AlertsConfig, AuthConfig, OfflineConfig, PushConfig, PushPreviews, RemoteConfig, TailscaleLogin,
};
use soos_remote::journal::{BoxFuture, OwnerLogin};
use soos_remote::logind::SourceError;
use soos_remote::offline::{ClockSample, NetworkSource, NmReading, OfflineClock, OfflineSettings};
use soos_remote::push::PushSettings;
use soos_remote::server::{serve, ServerState};
use soos_remote::{
    ALERTS_ACK_FILE_NAME, CREDENTIALS_FILE_NAME, DEFAULT_POLL_INTERVAL_MS, JOURNAL_IDLE_TICK_MS,
    NETWORK_STORE_FILE_NAME, OFFLINE_TICK_MS, PUSH_COALESCE_MS, PUSH_MIN_INTERVAL_MS,
    PUSH_OFFLINE_TOPIC, PUSH_ROUTE_MIN_INTERVAL_MS, PUSH_STORE_FILE_NAME, PUSH_TOPIC,
};

const SUBJECT: &str = "https://pc.tail1234.ts.net";
const IP_A: &str = "203.0.113.10";
const HOUR_MS: u64 = 3_600_000;
/// NetworkManager `CONNECTED_GLOBAL` / `DISCONNECTED`, connectivity `FULL` / `NONE`.
const NM_CONNECTED: NmReading = NmReading {
    state: 70,
    connectivity: 4,
    wireless_enabled: Some(true),
};
const NM_DISCONNECTED: NmReading = NmReading {
    state: 20,
    connectivity: 1,
    wireless_enabled: Some(true),
};
const NM_PORTAL: NmReading = NmReading {
    state: 70,
    connectivity: 2,
    wireless_enabled: Some(true),
};

// ---------------------------------------------------------------------------------------
// Fake bus and fake clock
// ---------------------------------------------------------------------------------------

struct NetInner {
    reading: Mutex<Result<NmReading, SourceError>>,
    shutdown: Mutex<Result<bool, SourceError>>,
    observes: watch::Sender<u64>,
    shutdown_reads: AtomicU64,
}

/// The `NetworkSource` seam driven by the test.
#[derive(Clone)]
struct FakeNetwork(Arc<NetInner>);

impl FakeNetwork {
    fn new(reading: NmReading) -> Self {
        Self(Arc::new(NetInner {
            reading: Mutex::new(Ok(reading)),
            shutdown: Mutex::new(Ok(false)),
            observes: watch::Sender::new(0),
            shutdown_reads: AtomicU64::new(0),
        }))
    }

    fn set(&self, reading: NmReading) {
        *self.0.reading.lock().unwrap() = Ok(reading);
    }

    fn fail(&self, error: SourceError) {
        *self.0.reading.lock().unwrap() = Err(error);
    }

    fn answer_shutdown(&self, answer: Result<bool, SourceError>) {
        *self.0.shutdown.lock().unwrap() = answer;
    }

    fn observes(&self) -> u64 {
        *self.0.observes.borrow()
    }

    fn shutdown_reads(&self) -> u64 {
        self.0.shutdown_reads.load(Ordering::SeqCst)
    }

    async fn wait_observes(&self, n: u64) {
        let mut rx = self.0.observes.subscribe();
        timeout(IO_BOUND, rx.wait_for(|c| *c >= n))
            .await
            .expect("a network read must start within the bound")
            .expect("fake alive");
        settle().await;
    }

    fn source(&self) -> Arc<dyn NetworkSource> {
        Arc::new(self.clone())
    }
}

impl NetworkSource for FakeNetwork {
    fn observe(&self) -> BoxFuture<'_, Result<NmReading, SourceError>> {
        self.0.observes.send_modify(|c| *c += 1);
        let answer = self.0.reading.lock().unwrap().clone();
        Box::pin(async move { answer })
    }

    fn preparing_for_shutdown(&self) -> BoxFuture<'_, Result<bool, SourceError>> {
        self.0.shutdown_reads.fetch_add(1, Ordering::SeqCst);
        let answer = self.0.shutdown.lock().unwrap().clone();
        Box::pin(async move { answer })
    }
}

/// The `OfflineClock` seam: every clock follows the paused tokio clock; a suspend moves the
/// wall and boot clocks only.
struct FakeClock {
    base: Instant,
    unix_base: u64,
    slept: AtomicU64,
    invalid: std::sync::atomic::AtomicBool,
}

impl FakeClock {
    fn new(unix_base: u64) -> Arc<Self> {
        Arc::new(Self {
            base: Instant::now(),
            unix_base,
            slept: AtomicU64::new(0),
            invalid: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// While set, every sample fails (a failed `clock_gettime`).
    fn set_invalid(&self, invalid: bool) {
        self.invalid.store(invalid, Ordering::SeqCst);
    }

    fn suspend(&self, ms: u64) {
        self.slept.fetch_add(ms, Ordering::SeqCst);
    }

    fn now(&self) -> ClockSample {
        self.valid_sample()
    }

    fn valid_sample(&self) -> ClockSample {
        let awake = Instant::now()
            .saturating_duration_since(self.base)
            .as_millis() as u64;
        let slept = self.slept.load(Ordering::SeqCst);
        ClockSample {
            unix_ms: self.unix_base + awake + slept,
            boot_ms: 1_000_000 + awake + slept,
            mono_ms: 500_000 + awake,
        }
    }
}

impl OfflineClock for FakeClock {
    fn sample(&self) -> Option<ClockSample> {
        if self.invalid.load(Ordering::SeqCst) {
            return None;
        }
        Some(self.valid_sample())
    }
}

// ---------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------

struct Env {
    dir: TempDir,
    clock: Arc<TestClock>,
    random: RandomSource,
}

impl Env {
    fn new() -> Self {
        let counter = Arc::new(AtomicU64::new(1));
        Self {
            dir: tempfile::tempdir().unwrap(),
            clock: TestClock::new(),
            random: Arc::new(move |buf: &mut [u8]| -> Result<(), RandomError> {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let seed = sha256(&n.to_be_bytes());
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = seed[i % 32] ^ (i / 32) as u8;
                }
                Ok(())
            }),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn network_store(&self) -> PathBuf {
        self.path(NETWORK_STORE_FILE_NAME)
    }

    fn locker(&self) -> String {
        let path = self.path("swaylock-plugin");
        if !path.exists() {
            fs::write(&path, b"#!/bin/false\n").unwrap();
        }
        path.to_string_lossy().into_owned()
    }
}

#[derive(Clone)]
struct Opts {
    offline: bool,
    push: bool,
    alerts: bool,
    previews: PushPreviews,
}

impl Opts {
    fn on() -> Self {
        Self {
            offline: true,
            push: true,
            alerts: false,
            previews: PushPreviews::Detailed,
        }
    }
}

/// Starts `serve` with Funnel and passkeys configured (the owner's passkey stored), push
/// wired to `transport`, the offline history wired to `net` and `clock`, and, with
/// `opts.alerts`, the failed-password alerts wired to `journal`.
async fn start(
    env: &Env,
    net: &FakeNetwork,
    clock: &Arc<FakeClock>,
    transport: &FakeTransport,
    journal: &ScriptedJournal,
    opts: Opts,
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
        enabled: opts.alerts,
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
            enabled: opts.push,
            vapid_subject: None,
            socket_path: None,
            previews: opts.previews,
        },
        camera: soos_remote::config::CameraConfig::default(),
        battery: soos_remote::config::BatteryConfig::default(),
        offline: OfflineConfig {
            enabled: opts.offline,
        },
    };
    let push_settings = PushSettings {
        store_path: env.path(PUSH_STORE_FILE_NAME),
        subject: SUBJECT.to_string(),
        previews: opts.previews,
        rp_id: HOST.to_string(),
    };
    let offline_settings = OfflineSettings {
        store_path: env.network_store(),
    };
    let alert_settings = AlertSettings {
        owner_login: Some(OwnerLogin::parse(OWNER).unwrap()),
        lock_screen_programs: alerts.lock_screen_programs.clone(),
        ack_path: Some(env.path(ALERTS_ACK_FILE_NAME)),
    };
    let source = MockSource::unlocked();
    let clock_fn = Arc::clone(&env.clock);
    let offline_clock: Arc<dyn OfflineClock> = clock.clone();
    let state = ServerState::new(config, UID, source.clone())
        .with_unix_clock(Arc::new(move || clock_fn.now_ms()))
        .with_credentials_path(store_path.clone())
        .with_file_owner_uid(own_uid())
        .with_random(env.random.clone())
        .with_password_alerts(alert_settings, Arc::new(journal.clone()))
        .with_push(push_settings, transport.transport())
        .with_offline_alerts(offline_settings, net.source(), offline_clock);
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

/// Advances one watcher period and waits until the read of that tick started.
async fn tick(h: &Harness, net: &FakeNetwork) {
    let before = net.observes();
    h.advance_ms(OFFLINE_TICK_MS).await;
    net.wait_observes(before + 1).await;
    pump().await;
}

async fn ticks(h: &Harness, net: &FakeNetwork, n: u64) {
    for _ in 0..n {
        tick(h, net).await;
    }
}

/// Lets blocking-pool file work finish in real time (bounded, about 3 s).
async fn real_pause() {
    tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(10)))
        .await
        .unwrap();
}

/// Waits until the network store satisfies `done` (bounded real time); returns its JSON.
async fn wait_store(env: &Env, done: impl Fn(&Value) -> bool) -> Value {
    let mut last = Value::Null;
    for _ in 0..300 {
        if let Ok(bytes) = fs::read(env.network_store()) {
            if let Ok(json) = serde_json::from_slice::<Value>(&bytes) {
                if done(&json) {
                    return json;
                }
                last = json;
            }
        }
        real_pause().await;
        pump().await;
    }
    panic!("network store never satisfied the condition: {last}");
}

async fn offline_view_via(h: &Harness, via: &Via) -> Value {
    pump().await;
    let r = h.get_via(via, "/api/offline").await;
    assert_eq!(r.status, 200, "GET /api/offline: {}", r.result_or_body());
    r.assert_mandatory_headers();
    r.assert_json_body();
    let v = r.json();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec!["intervals", "network_monitor", "reason", "state"],
        "{v}"
    );
    v
}

async fn offline_view(h: &Harness) -> Value {
    offline_view_via(h, &Via::Tailnet).await
}

/// Polls the view (real time for the blocking-pool start) until `done` holds.
async fn wait_view(h: &Harness, done: impl Fn(&Value) -> bool) -> Value {
    let mut view = offline_view(h).await;
    for _ in 0..300 {
        if done(&view) {
            return view;
        }
        real_pause().await;
        view = offline_view(h).await;
    }
    panic!("view never satisfied the condition: {view}");
}

async fn subscribe_one(h: &Harness) -> UaFixture {
    let ua = UaFixture::new(1);
    let r = h
        .send(
            &Via::Tailnet,
            "POST",
            "/api/push/subscribe",
            &[("X-Soos-Action", "push-subscribe"), ("Origin", ORIGIN)],
            Some(&subscribe_body(&apple_endpoint(1), &ua)),
        )
        .await;
    assert_eq!(
        (r.status, r.result_or_body()),
        (200, "subscribed".to_string())
    );
    h.advance_ms(PUSH_ROUTE_MIN_INTERVAL_MS + 1).await;
    ua
}

/// Moves the clock in `step` ms steps until the transport saw `n` calls.
async fn wait_calls(h: &Harness, t: &FakeTransport, n: usize, max_ms: u64, step: u64) {
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
        h.advance_ms(step).await;
        elapsed += step;
    }
}

fn write_mode(path: &Path, bytes: &[u8], mode: u32) {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .unwrap();
    std::io::Write::write_all(&mut file, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn disabled_view() -> Value {
    json!({"state": "disabled", "reason": null, "network_monitor": "unknown", "intervals": []})
}

// ---------------------------------------------------------------------------------------
// Test 22 — off
// ---------------------------------------------------------------------------------------

/// Test 22 (ROA8, D-8): with `offline_alerts` off (the default) a wired source is never
/// called (no bus read, so no match and no property read), no file is created, no push is
/// sent, and the view is `disabled`; the shared harness without offline wiring answers the
/// same view.
#[tokio::test(start_paused = true)]
async fn test_roa_off_installs_nothing() {
    assert!(!OfflineConfig::default().enabled);
    {
        let h = Harness::start().await;
        assert_eq!(offline_view(&h).await, disabled_view());
    }
    let env = Env::new();
    let net = FakeNetwork::new(NM_CONNECTED);
    let clock = FakeClock::new(BASE_UNIX_MS);
    let t = FakeTransport::new();
    let j = ScriptedJournal::new();
    let h = start(
        &env,
        &net,
        &clock,
        &t,
        &j,
        Opts {
            offline: false,
            ..Opts::on()
        },
    )
    .await;
    assert_eq!(offline_view(&h).await, disabled_view());
    net.set(NM_DISCONNECTED);
    h.step_ms(1000, 60_000).await;
    net.set(NM_CONNECTED);
    h.step_ms(1000, 60_000).await;
    clock.suspend(HOUR_MS);
    h.step_ms(1000, 30_000).await;
    pump().await;
    assert_eq!(net.observes(), 0, "no network read while off");
    let head = h.request("HEAD", "/api/offline", &[]).await;
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    h.shutdown().await.unwrap();
    assert_eq!(net.shutdown_reads(), 0, "no logind read while off");
    assert!(
        fs::symlink_metadata(env.network_store()).is_err(),
        "no file"
    );
    assert!(t.calls_with_topic(PUSH_OFFLINE_TOPIC).is_empty());
}

// ---------------------------------------------------------------------------------------
// Test 23 — offline without push is inert
// ---------------------------------------------------------------------------------------

/// Test 23 (ROA8, D-9): the configuration refuses `offline_alerts` without push (test 17);
/// a state built by hand with offline on and push off still wires nothing: no bus read, no
/// file, the `disabled` view, the rest of the service unaffected.
#[tokio::test(start_paused = true)]
async fn test_roa_offline_without_push_is_inert() {
    let env = Env::new();
    let net = FakeNetwork::new(NM_DISCONNECTED);
    let clock = FakeClock::new(BASE_UNIX_MS);
    let t = FakeTransport::new();
    let j = ScriptedJournal::new();
    let h = start(
        &env,
        &net,
        &clock,
        &t,
        &j,
        Opts {
            push: false,
            ..Opts::on()
        },
    )
    .await;
    assert_eq!(offline_view(&h).await, disabled_view());
    let status = h.get("/api/status").await;
    assert_eq!(status.status, 200);
    let lock = h.lock(&[]).await;
    assert_eq!(lock.status, 202, "{}", lock.result_or_body());
    h.step_ms(1000, 60_000).await;
    assert_eq!(net.observes(), 0);
    h.shutdown().await.unwrap();
    assert_eq!(net.shutdown_reads(), 0);
    assert!(
        fs::symlink_metadata(env.network_store()).is_err(),
        "no file"
    );
    assert_eq!(t.count(), 0);
}

// ---------------------------------------------------------------------------------------
// Test 24 — one back-online push
// ---------------------------------------------------------------------------------------

/// Test 24 (ROA1, ROA6, ROA10): an outage of about two minutes gives exactly one encrypted
/// back-online push with the fixed vocabulary on topic `soosonline` once the network is
/// back, recorded newest first in the view and in the store; a captive portal is online;
/// with `push_previews = "generic"` the lock-screen text is fixed.
#[tokio::test(start_paused = true)]
async fn test_roa_network_outage_sends_one_back_online_push() {
    for previews in [PushPreviews::Detailed, PushPreviews::Generic] {
        let env = Env::new();
        let net = FakeNetwork::new(NM_CONNECTED);
        let clock = FakeClock::new(BASE_UNIX_MS);
        let t = FakeTransport::new();
        let j = ScriptedJournal::new();
        let h = start(
            &env,
            &net,
            &clock,
            &t,
            &j,
            Opts {
                previews,
                ..Opts::on()
            },
        )
        .await;
        net.wait_observes(1).await;
        let view = wait_view(&h, |v| v["state"] == "active").await;
        assert_eq!(view["network_monitor"], "active", "{view}");
        assert_eq!(view["intervals"], json!([]));
        let ua = subscribe_one(&h).await;

        // A captive portal is online: nothing recorded.
        net.set(NM_PORTAL);
        ticks(&h, &net, 2).await;
        net.set(NM_DISCONNECTED);
        tick(&h, &net).await;
        let offline_at = clock.now();
        ticks(&h, &net, 24).await;
        net.set(NM_CONNECTED);
        tick(&h, &net).await;
        let back_at = clock.now();
        let duration = back_at.boot_ms - offline_at.boot_ms;
        assert!((120_000..180_000).contains(&duration), "{duration}");

        wait_calls(&h, &t, 1, PUSH_COALESCE_MS + 2 * OFFLINE_TICK_MS, 250).await;
        h.step_ms(1000, 2 * PUSH_MIN_INTERVAL_MS).await;
        pump().await;
        let calls = t.calls();
        assert_eq!(calls.len(), 1, "exactly one push");
        assert_eq!(calls[0].topic(), Some(PUSH_OFFLINE_TOPIC));
        let payload = decrypt_json(&calls[0].request.body, &ua);
        match previews {
            PushPreviews::Detailed => {
                assert_eq!(payload["notification"]["title"], "Your PC is back online");
                assert_eq!(
                    payload["notification"]["body"],
                    "Offline for 2 min (network lost)"
                );
                assert_eq!(payload["soos"]["reason"], "network_lost");
            }
            PushPreviews::Generic => {
                assert_eq!(payload["notification"]["title"], "Update from your PC");
                assert_eq!(payload["notification"]["body"], "Open soos for details");
                assert_eq!(payload["soos"]["reason"], Value::Null);
            }
        }
        assert_eq!(payload["soos"]["kind"], "back_online");
        assert_eq!(payload["soos"]["intervals"], 1);
        assert_eq!(payload["soos"]["first_start_unix_ms"], offline_at.unix_ms);
        assert_eq!(payload["soos"]["last_end_unix_ms"], back_at.unix_ms);

        let expected = json!({
            "reason": "network_lost",
            "start_unix_ms": offline_at.unix_ms,
            "end_unix_ms": back_at.unix_ms,
            "duration_ms": duration,
        });
        let view = offline_view(&h).await;
        assert_eq!(view["intervals"], json!([expected.clone()]), "{view}");
        let stored = wait_store(&env, |s| {
            s["intervals"].as_array().is_some_and(|a| a.len() == 1)
        })
        .await;
        assert_eq!(stored["intervals"][0], expected);
        assert_eq!(stored["run"], "running");
        let meta = fs::symlink_metadata(env.network_store()).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o600);
        h.shutdown().await.unwrap();
    }
}

// ---------------------------------------------------------------------------------------
// Test 25 — unclean previous stop
// ---------------------------------------------------------------------------------------

/// Test 25 (ROA5): a store left `running` (crash or power loss) gives one
/// `shutdown_or_crash` interval from its last sign of life to the start, held while the
/// network is down and pushed once it is back.
#[tokio::test(start_paused = true)]
async fn test_roa_unclean_previous_stop_is_shutdown_or_crash() {
    let env = Env::new();
    let last_alive = BASE_UNIX_MS - 600_000;
    write_mode(
        &env.network_store(),
        format!(
            r#"{{"version":1,"run":"running","last_alive_unix_ms":{last_alive},"intervals":[]}}"#
        )
        .as_bytes(),
        0o600,
    );
    let net = FakeNetwork::new(NM_DISCONNECTED);
    let clock = FakeClock::new(BASE_UNIX_MS);
    let t = FakeTransport::new();
    let j = ScriptedJournal::new();
    let h = start(&env, &net, &clock, &t, &j, Opts::on()).await;
    net.wait_observes(1).await;
    let view = wait_view(&h, |v| v["state"] == "active").await;
    let expected = json!({
        "reason": "shutdown_or_crash",
        "start_unix_ms": last_alive,
        "end_unix_ms": BASE_UNIX_MS,
        "duration_ms": 600_000,
    });
    assert_eq!(view["intervals"], json!([expected.clone()]), "{view}");
    let stored = wait_store(&env, |s| {
        s["intervals"].as_array().is_some_and(|a| a.len() == 1)
    })
    .await;
    assert_eq!(stored["intervals"][0], expected);
    assert_eq!(stored["run"], "running");

    let ua = subscribe_one(&h).await;
    // Still offline: nothing is sent.
    ticks(&h, &net, 3).await;
    h.step_ms(1000, PUSH_COALESCE_MS + 1000).await;
    assert_eq!(t.count(), 0, "held while offline");
    net.set(NM_CONNECTED);
    tick(&h, &net).await;
    wait_calls(&h, &t, 1, PUSH_COALESCE_MS + OFFLINE_TICK_MS, 250).await;
    let calls = t.calls();
    assert_eq!(calls[0].topic(), Some(PUSH_OFFLINE_TOPIC));
    let payload = decrypt_json(&calls[0].request.body, &ua);
    assert_eq!(
        payload["notification"]["body"],
        "Offline for 10 min (shutdown or crash)"
    );
    h.step_ms(1000, 2 * PUSH_MIN_INTERVAL_MS).await;
    assert_eq!(t.count(), 1);
    h.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------------------
// Test 26 — clean stop marker
// ---------------------------------------------------------------------------------------

/// Test 26 (ROA5, D-7): a clean stop reads logind `PreparingForShutdown` once: `false` →
/// `stopped` (no interval at the next start), `true` or a failed read → `shutdown` (one
/// `shutdown_or_crash` interval at the next start); an outage still open at the stop is
/// recorded without a push.
#[tokio::test(start_paused = true)]
async fn test_roa_clean_stop_writes_marker_and_shutdown_read() {
    for (answer, marker) in [
        (Ok(false), "stopped"),
        (Ok(true), "shutdown"),
        (Err(SourceError::Call), "shutdown"),
        (Err(SourceError::BusUnavailable), "shutdown"),
    ] {
        let env = Env::new();
        let net = FakeNetwork::new(NM_CONNECTED);
        let clock = FakeClock::new(BASE_UNIX_MS);
        let t = FakeTransport::new();
        let j = ScriptedJournal::new();
        let h = start(&env, &net, &clock, &t, &j, Opts::on()).await;
        net.wait_observes(1).await;
        wait_view(&h, |v| v["state"] == "active").await;
        net.set(NM_DISCONNECTED);
        tick(&h, &net).await;
        let offline_at = clock.now();
        ticks(&h, &net, 8).await;
        net.answer_shutdown(answer.clone());
        let stop_at = clock.now();
        h.shutdown().await.unwrap();
        assert_eq!(net.shutdown_reads(), 1, "{marker}");
        let stored = wait_store(&env, |s| s["run"] == marker).await;
        assert_eq!(stored["last_alive_unix_ms"], stop_at.unix_ms, "{answer:?}");
        assert_eq!(
            stored["intervals"],
            json!([{
                "reason": "network_lost",
                "start_unix_ms": offline_at.unix_ms,
                "end_unix_ms": stop_at.unix_ms,
                "duration_ms": stop_at.boot_ms - offline_at.boot_ms,
            }]),
            "{answer:?}"
        );
        assert!(t.calls_with_topic(PUSH_OFFLINE_TOPIC).is_empty());

        // Next start, 10 minutes later.
        let net = FakeNetwork::new(NM_CONNECTED);
        let clock = FakeClock::new(stop_at.unix_ms + 600_000);
        let h = start(&env, &net, &clock, &t, &j, Opts::on()).await;
        net.wait_observes(1).await;
        let view = wait_view(&h, |v| v["state"] == "active").await;
        let intervals = view["intervals"].as_array().unwrap().clone();
        if marker == "stopped" {
            assert_eq!(intervals.len(), 1, "{view}");
        } else {
            assert_eq!(intervals.len(), 2, "{view}");
            assert_eq!(
                intervals[0],
                json!({
                    "reason": "shutdown_or_crash",
                    "start_unix_ms": stop_at.unix_ms,
                    "end_unix_ms": stop_at.unix_ms + 600_000,
                    "duration_ms": 600_000,
                })
            );
        }
        let stored = wait_store(&env, |s| s["run"] == "running").await;
        assert_eq!(
            stored["intervals"].as_array().unwrap().len(),
            intervals.len()
        );
        h.shutdown().await.unwrap();
    }
}

// ---------------------------------------------------------------------------------------
// Test 27 — store failure
// ---------------------------------------------------------------------------------------

/// Test 27 (ROA4, D-14): an insecure or malformed store makes the feature `unavailable` /
/// `store_failed`: no bus read at all, the file never rewritten (not even at the stop), the
/// rest of the service unaffected.
#[tokio::test(start_paused = true)]
async fn test_roa_store_failed_is_unavailable_and_never_rewritten() {
    for (content, mode) in [
        (b"{".to_vec(), 0o600),
        (
            br#"{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[]}"#.to_vec(),
            0o644,
        ),
        (
            br#"{"version":9,"run":"stopped","last_alive_unix_ms":1,"intervals":[]}"#.to_vec(),
            0o600,
        ),
    ] {
        let env = Env::new();
        write_mode(&env.network_store(), &content, mode);
        let net = FakeNetwork::new(NM_DISCONNECTED);
        let clock = FakeClock::new(BASE_UNIX_MS);
        let t = FakeTransport::new();
        let j = ScriptedJournal::new();
        let h = start(&env, &net, &clock, &t, &j, Opts::on()).await;
        let view = wait_view(&h, |v| v["state"] != "disabled").await;
        assert_eq!(
            view,
            json!({"state": "unavailable", "reason": "store_failed", "network_monitor": "unknown", "intervals": []})
        );
        assert_eq!(h.get("/api/status").await.status, 200);
        h.step_ms(1000, 60_000).await;
        h.shutdown().await.unwrap();
        assert_eq!(net.observes(), 0);
        assert_eq!(net.shutdown_reads(), 0);
        assert_eq!(fs::read(env.network_store()).unwrap(), content);
        assert_eq!(
            fs::metadata(env.network_store()).unwrap().mode() & 0o777,
            mode
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 28 — Funnel
// ---------------------------------------------------------------------------------------

/// Test 28 (ROA9, §6): over Funnel the view needs a web session (`403 login_required`
/// before any work); a POST is `405`.
#[tokio::test(start_paused = true)]
async fn test_roa_view_over_funnel_requires_session() {
    let env = Env::new();
    let net = FakeNetwork::new(NM_CONNECTED);
    let clock = FakeClock::new(BASE_UNIX_MS);
    let t = FakeTransport::new();
    let j = ScriptedJournal::new();
    let h = start(&env, &net, &clock, &t, &j, Opts::on()).await;
    wait_view(&h, |v| v["state"] == "active").await;
    let r = h.get_via(&Via::funnel(IP_A), "/api/offline").await;
    assert_eq!(
        (r.status, r.result_or_body()),
        (403, "login_required".to_string())
    );
    let r = h.request("POST", "/api/offline", &[]).await;
    assert_eq!(r.status, 405);
    assert_eq!(r.header("allow"), Some("GET, HEAD"));
    h.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------------------
// Test 29 — no NetworkManager
// ---------------------------------------------------------------------------------------

/// Test 29 (ROA3, D-15): without NetworkManager every read fails: the monitor is
/// `unavailable` and nothing network-related is recorded, but a suspend is still recorded
/// as `sleep` (never pushed).
#[tokio::test(start_paused = true)]
async fn test_roa_network_monitor_unavailable_without_networkmanager() {
    let env = Env::new();
    let net = FakeNetwork::new(NM_CONNECTED);
    net.fail(SourceError::Call);
    let clock = FakeClock::new(BASE_UNIX_MS);
    let t = FakeTransport::new();
    let j = ScriptedJournal::new();
    let h = start(&env, &net, &clock, &t, &j, Opts::on()).await;
    net.wait_observes(1).await;
    let view = wait_view(&h, |v| v["state"] == "active").await;
    assert_eq!(view["network_monitor"], "unavailable", "{view}");
    let _ua = subscribe_one(&h).await;
    ticks(&h, &net, 2).await;
    // A failed clock read skips the whole step (no bus read).
    clock.set_invalid(true);
    let reads = net.observes();
    h.step_ms(1000, 3 * OFFLINE_TICK_MS).await;
    pump().await;
    assert_eq!(
        net.observes(),
        reads,
        "no read without a valid clock sample"
    );
    clock.set_invalid(false);
    tick(&h, &net).await;
    let before = clock.now();
    clock.suspend(HOUR_MS);
    tick(&h, &net).await;
    let after = clock.now();
    let view = offline_view(&h).await;
    let rows = view["intervals"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{view}");
    assert_eq!(rows[0]["reason"], "sleep");
    assert_eq!(rows[0]["duration_ms"], HOUR_MS);
    assert_eq!(rows[0]["end_unix_ms"], after.unix_ms);
    assert!(rows[0]["start_unix_ms"].as_u64().unwrap() >= before.unix_ms);
    h.step_ms(1000, 2 * PUSH_MIN_INTERVAL_MS).await;
    assert_eq!(t.count(), 0, "a sleep is never pushed");
    h.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------------------
// Test 30 — shared budget
// ---------------------------------------------------------------------------------------

/// Test 30 (ROA7, D-10): a failed-password alert and a back-online notice due together share
/// the push budget: the alert first, the notice at least `PUSH_MIN_INTERVAL_MS` later.
#[tokio::test(start_paused = true)]
async fn test_roa_alert_and_back_online_share_the_budget() {
    let env = Env::new();
    let net = FakeNetwork::new(NM_CONNECTED);
    let clock = FakeClock::new(BASE_UNIX_MS);
    let t = FakeTransport::new();
    let j = ScriptedJournal::new();
    let h = start(
        &env,
        &net,
        &clock,
        &t,
        &j,
        Opts {
            alerts: true,
            ..Opts::on()
        },
    )
    .await;
    // The alerts follower becomes active.
    let mut waited = 0;
    loop {
        pump().await;
        let alerts = h.get("/api/alerts").await.json();
        if j.follows() >= 1 && alerts["state"] == "active" {
            break;
        }
        assert!(
            waited < 3 * JOURNAL_IDLE_TICK_MS + 1000,
            "alerts not active"
        );
        h.advance_ms(250).await;
        waited += 250;
    }
    wait_view(&h, |v| v["state"] == "active").await;
    let _ua = subscribe_one(&h).await;
    net.set(NM_DISCONNECTED);
    ticks(&h, &net, 10).await;

    // A live sudo failure, then the network comes back at the next tick.
    let now_us = (h.clock.now_ms() * 1000 / 1000) * 1000;
    j.push(&pam_unix_line(
        "sudo",
        OWNER_UID,
        Some(SUDO_EXE),
        OWNER,
        now_us,
    ));
    pump().await;
    net.set(NM_CONNECTED);
    tick(&h, &net).await;

    wait_calls(&h, &t, 2, 3 * PUSH_MIN_INTERVAL_MS, 250).await;
    let calls = t.calls();
    assert_eq!(calls[0].topic(), Some(PUSH_TOPIC), "the alert goes first");
    assert_eq!(calls[1].topic(), Some(PUSH_OFFLINE_TOPIC));
    assert!(
        calls[1].at_ms - calls[0].at_ms >= PUSH_MIN_INTERVAL_MS,
        "{} then {}",
        calls[0].at_ms,
        calls[1].at_ms
    );
    h.step_ms(1000, 2 * PUSH_MIN_INTERVAL_MS).await;
    assert_eq!(t.count(), 2);
    h.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------------------
// Heartbeat cadence (plan evaluation F-11)
// ---------------------------------------------------------------------------------------

/// ROA4 (§3, §5.5): without any change the store is rewritten at most once per
/// `OFFLINE_HEARTBEAT_MS` (each save replaces the inode), and the heartbeat advances
/// `last_alive_unix_ms`.
#[tokio::test(start_paused = true)]
async fn test_roa_heartbeat_saves_at_most_once_per_minute() {
    let env = Env::new();
    let net = FakeNetwork::new(NM_CONNECTED);
    let clock = FakeClock::new(BASE_UNIX_MS);
    let t = FakeTransport::new();
    let j = ScriptedJournal::new();
    let h = start(&env, &net, &clock, &t, &j, Opts::on()).await;
    net.wait_observes(1).await;
    let first = wait_store(&env, |s| s["run"] == "running").await;
    assert_eq!(first["last_alive_unix_ms"], BASE_UNIX_MS);
    let inode = fs::metadata(env.network_store()).unwrap().ino();
    // 50 s of unchanged online readings: no write.
    ticks(&h, &net, 10).await;
    for _ in 0..20 {
        real_pause().await;
    }
    assert_eq!(
        fs::metadata(env.network_store()).unwrap().ino(),
        inode,
        "no write before a minute"
    );
    // Past the minute: exactly one heartbeat.
    ticks(&h, &net, 3).await;
    let beat = wait_store(&env, |s| {
        s["last_alive_unix_ms"].as_u64() > Some(BASE_UNIX_MS)
    })
    .await;
    let alive = beat["last_alive_unix_ms"].as_u64().unwrap();
    assert!(
        alive >= BASE_UNIX_MS + 60_000 && alive <= BASE_UNIX_MS + 65_000,
        "{alive}"
    );
    let second = fs::metadata(env.network_store()).unwrap().ino();
    assert_ne!(second, inode);
    ticks(&h, &net, 4).await;
    for _ in 0..20 {
        real_pause().await;
    }
    assert_eq!(
        fs::metadata(env.network_store()).unwrap().ino(),
        second,
        "one write per minute"
    );
    h.shutdown().await.unwrap();
}

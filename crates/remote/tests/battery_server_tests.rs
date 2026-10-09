//! End-to-end contract tests of the live battery level in `soos-remote::server::serve`
//! (ADR 2026-10-07 "Live Battery Level in `soos-remote`", architect spec
//! `AI/architect_spec_remote_battery.md` §5.5, §6, §10.4 tests 20–31 and §10.6 test 38;
//! matrix RBS1, RBS6–RBS10).
//!
//! Same deterministic harness as `alerts_server_tests.rs` (frozen paused clock, scripted
//! logind, raw HTTP/1.1 over the Unix socket in a `TempDir`, passkey store fixture for
//! Funnel sessions) plus a battery source injected through `ServerState::with_battery`: a
//! [`FakeSysfs`] tempdir root read by the production `SysfsBattery`, or a
//! [`ScriptedBattery`]. Nothing reads the real `/sys`.
//!
//! Time model (spec §10, Round 2 F-4 and Round 3 R2-2): virtual time moves only through
//! `tokio::time::advance`; real-time waits are used only for blocking-pool completion
//! ([`quiesce`], [`wait_until`]) and never move the clock. A cached entry is reused by
//! `GET /api/battery` up to and including an age of `BATTERY_SAMPLE_INTERVAL_MS`, so a read
//! that must miss the cache advances `BATTERY_SAMPLE_INTERVAL_MS + 1`. A gated source is
//! released by a `GateRelease` guard created before the first assertion.

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

#[path = "common/battery.rs"]
mod battery;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;
use tokio::sync::oneshot;
use tokio::task::yield_now;

use battery::*;
use harness::*;
use journal::*;
use passkey::*;
use soos_remote::alerts::AlertSettings;
use soos_remote::battery::{BatteryRuntime, BatterySource, BatteryView, ChargeStatus};
use soos_remote::config::{AlertsConfig, AuthConfig, BatteryConfig, RemoteConfig, TailscaleLogin};
use soos_remote::journal::OwnerLogin;
use soos_remote::server::{serve, ServerState};
use soos_remote::{
    BATTERY_READ_TIMEOUT_MS, BATTERY_SAMPLE_INTERVAL_MS, CREDENTIALS_FILE_NAME,
    DEFAULT_POLL_INTERVAL_MS, SSE_KEEPALIVE_MS,
};

const IP_A: &str = "203.0.113.10";
const IP_B: &str = "203.0.113.20";
const IP_C: &str = "203.0.113.30";

const PRESENT_82_JSON: &str =
    r#"{"state":"present","percent":82,"charge":"discharging","external_power":false}"#;
const DISABLED_JSON: &str =
    r#"{"state":"disabled","percent":null,"charge":null,"external_power":null}"#;
const UNAVAILABLE_JSON: &str =
    r#"{"state":"unavailable","percent":null,"charge":null,"external_power":null}"#;

// ---------------------------------------------------------------------------------------
// Battery harness
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Wiring {
    /// `battery_status` of the configuration.
    enabled: bool,
    /// `password_alerts` on, with a scripted journal.
    alerts: bool,
}

impl Wiring {
    const ON: Self = Self {
        enabled: true,
        alerts: false,
    };
}

/// Starts `serve` with Funnel and passkeys configured (the owner's passkey stored, so
/// Funnel sessions can be opened), unlock off, and `source` wired through
/// `ServerState::with_battery` when given.
async fn start(
    source: Option<Arc<dyn BatterySource>>,
    wiring: Wiring,
) -> (Harness, ScriptedJournal) {
    let frozen = FrozenClock::hold();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.sock");
    let store_path = dir.path().join(CREDENTIALS_FILE_NAME);
    let owner = Authenticator::owner();
    write_store(&store_path, &[StoredPasskey::of(&owner)]);
    let alerts = AlertsConfig {
        enabled: wiring.alerts,
        ..AlertsConfig::default()
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
        battery: BatteryConfig {
            enabled: wiring.enabled,
        },
    };
    let journal = ScriptedJournal::new();
    let logind = MockSource::unlocked();
    let clock = TestClock::new();
    let clock_fn = Arc::clone(&clock);
    let mut state = ServerState::new(config, UID, logind.clone())
        .with_unix_clock(Arc::new(move || clock_fn.now_ms()))
        .with_credentials_path(store_path.clone())
        .with_file_owner_uid(own_uid());
    if wiring.alerts {
        let settings = AlertSettings {
            owner_login: Some(OwnerLogin::parse(OWNER).unwrap()),
            lock_screen_programs: alerts.lock_screen_programs.clone(),
            ack_path: None,
        };
        state = state.with_password_alerts(settings, Arc::new(journal.clone()));
    }
    if let Some(source) = source {
        state = state.with_battery(source);
    }
    let listener = UnixListener::bind(&path).unwrap();
    let (tx, rx) = oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, Arc::new(state), async move {
        let _ = rx.await;
    }));
    settle().await;
    let h = Harness {
        dir: dir.path().to_path_buf(),
        _dir: dir,
        path,
        store_path,
        owner,
        source: logind,
        clock,
        shutdown: Some(tx),
        server: Some(server),
        _frozen: frozen,
        secrets: Mutex::new(Vec::new()),
    };
    (h, journal)
}

/// A `FakeSysfs` laptop: BAT0 82 % discharging, ADP0 offline.
fn laptop() -> FakeSysfs {
    let sysfs = FakeSysfs::new();
    sysfs.battery(
        "BAT0",
        &[("capacity", b"82\n"), ("status", b"Discharging\n")],
    );
    sysfs.mains("ADP0", "0");
    sysfs
}

/// `sysfs` wired through a [`Tracked`] production source.
fn tracked(sysfs: &FakeSysfs) -> (Arc<dyn BatterySource>, Probe) {
    let source = Tracked::new(sysfs.source());
    let probe = source.probe();
    (Arc::new(source), probe)
}

/// Waits (real time, bounded; never moves the clock) until `cond` holds, letting every task
/// run between two checks.
async fn wait_until(what: &str, cond: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    loop {
        settle().await;
        if cond() {
            return;
        }
        assert!(start.elapsed() < REAL_WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Waits (real time) until no read of `probe` runs, then lets the server consume the result.
async fn quiesce(probe: &Probe) {
    wait_until("the battery read to finish", || probe.idle()).await;
    std::thread::sleep(Duration::from_millis(2));
    settle().await;
    wait_until("the battery read to finish", || probe.idle()).await;
    settle().await;
}

async fn get_battery(h: &Harness, via: &Via) -> HttpResponse {
    h.get_via(via, "/api/battery").await
}

/// A `200` battery response: mandatory headers, JSON body, exact four keys.
fn battery_body(r: &HttpResponse) -> Value {
    assert_eq!(r.status, 200, "GET /api/battery: {}", r.result_or_body());
    r.assert_mandatory_headers();
    r.assert_json_body();
    let json = r.json();
    assert_battery_shape(&json);
    json
}

fn assert_battery_shape(v: &Value) {
    let keys: BTreeSet<&str> = v
        .as_object()
        .unwrap_or_else(|| panic!("battery view is an object: {v}"))
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from(["charge", "external_power", "percent", "state"]),
        "{v}"
    );
}

/// The five-key status shape, re-implemented locally (RC-5; the harness stays untouched).
fn assert_status_view(v: &Value) {
    let keys: BTreeSet<&str> = v
        .as_object()
        .unwrap_or_else(|| panic!("status view is an object: {v}"))
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from([
            "active",
            "checked_unix_ms",
            "idle",
            "idle_since_unix_s",
            "state"
        ]),
        "{v}"
    );
}

fn assert_result(r: &HttpResponse, status: u16, result: &str) {
    assert_eq!(
        (r.status, r.result_or_body()),
        (status, result.to_string()),
        "expected {status} {result}"
    );
    r.assert_mandatory_headers();
}

// ---------------------------------------------------------------------------------------
// SSE frames of any event name
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
struct Frame {
    name: String,
    /// The raw `data:` text.
    data: String,
    json: Value,
}

enum Polled2 {
    Frame(Frame),
    Eof,
    Pending,
}

fn poll_frame(c: &mut SseClient) -> Polled2 {
    loop {
        if let Some(pos) = c.buf.windows(2).position(|w| w == b"\n\n") {
            let frame: Vec<u8> = c.buf.drain(..pos + 2).collect();
            let raw = String::from_utf8(frame).expect("UTF-8 frame");
            let mut name = None;
            let mut data = None;
            for line in raw.lines() {
                if let Some(rest) = line.strip_prefix("event: ") {
                    assert!(name.is_none(), "one event line per frame: {raw:?}");
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
                ["status", "alerts", "battery"].contains(&name.as_str()),
                "unknown event name {name:?}"
            );
            let data = data.unwrap_or_else(|| panic!("event without data: {raw:?}"));
            let json: Value = serde_json::from_str(&data).expect("event data is JSON");
            match name.as_str() {
                "battery" => assert_battery_shape(&json),
                "status" => assert_status_view(&json),
                _ => {}
            }
            return Polled2::Frame(Frame { name, data, json });
        }
        let mut chunk = [0u8; 4096];
        match c.stream.try_read(&mut chunk) {
            Ok(0) => return Polled2::Eof,
            Ok(n) => c.buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Polled2::Pending,
            Err(_) => return Polled2::Eof,
        }
    }
}

/// The next frame, waiting in real time (bounded) for blocking-pool work; never moves the
/// clock.
async fn next_frame(c: &mut SseClient) -> Frame {
    let start = std::time::Instant::now();
    loop {
        for _ in 0..POLL_ROUNDS {
            match poll_frame(c) {
                Polled2::Frame(f) => return f,
                Polled2::Eof => panic!("stream ended while a frame was expected"),
                Polled2::Pending => yield_now().await,
            }
        }
        assert!(start.elapsed() < REAL_WAIT, "no frame arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Every frame already available (no clock movement); `true` when the stream ended.
async fn frames_now(c: &mut SseClient) -> (Vec<Frame>, bool) {
    let mut out = Vec::new();
    let mut idle_rounds = 0;
    while idle_rounds < POLL_ROUNDS {
        match poll_frame(c) {
            Polled2::Frame(f) => {
                out.push(f);
                idle_rounds = 0;
            }
            Polled2::Eof => return (out, true),
            Polled2::Pending => {
                idle_rounds += 1;
                yield_now().await;
            }
        }
    }
    (out, false)
}

/// Moves the clock by `total_ms` in `step` ms steps, letting every battery read of `probe`
/// finish after each step, and returns every frame with its arrival time; stops at EOF.
async fn frames_during(
    c: &mut SseClient,
    h: &Harness,
    probe: &Probe,
    total_ms: u64,
    step: u64,
) -> (Vec<(u64, Frame)>, bool) {
    let mut out = Vec::new();
    let mut elapsed = 0;
    loop {
        quiesce(probe).await;
        let (frames, eof) = frames_now(c).await;
        out.extend(frames.into_iter().map(|f| (elapsed, f)));
        if eof {
            return (out, true);
        }
        if elapsed >= total_ms {
            return (out, false);
        }
        let n = step.min(total_ms - elapsed);
        h.advance_ms(n).await;
        elapsed += n;
    }
}

fn battery_frames(frames: &[(u64, Frame)]) -> Vec<&Frame> {
    frames
        .iter()
        .map(|(_, f)| f)
        .filter(|f| f.name == "battery")
        .collect()
}

fn status_frames(frames: &[(u64, Frame)]) -> usize {
    frames.iter().filter(|(_, f)| f.name == "status").count()
}

/// Opens a stream through `via` and returns it with its first two frames read (status,
/// then battery) when `battery` is true, or only the status frame otherwise.
async fn open(h: &Harness, via: &Via) -> SseClient {
    match h.open_stream_via(via).await {
        Ok(client) => {
            assert_sse_head(&client.head);
            client
        }
        Err(Outcome::Response(r)) => panic!("stream refused: {}", r.result_or_body()),
        Err(other) => panic!("stream not opened: {other:?}"),
    }
}

/// An always-idle probe (sources that are not tracked).
fn no_probe() -> Probe {
    Tracked::new(ScriptedBattery::new(BatteryView::unavailable())).probe()
}

/// A [`Probe`]-like idleness check for a scripted source.
async fn quiesce_scripted(source: &ScriptedBattery) {
    wait_until("the scripted read to finish", || {
        source.entered() == source.returned()
    })
    .await;
    std::thread::sleep(Duration::from_millis(2));
    settle().await;
}

// ---------------------------------------------------------------------------------------
// Tests 20–22: route, visibility, off switch
// ---------------------------------------------------------------------------------------

/// Test 20 (RBS8, §6.2): `GET /api/battery` on the tailnet answers `200` with the view of
/// the wired source and the standard headers; `HEAD` the same headers and no body.
#[tokio::test(start_paused = true)]
async fn test_rbs_get_battery_tailnet() {
    let sysfs = laptop();
    let (source, _probe) = tracked(&sysfs);
    let (h, _j) = start(Some(source), Wiring::ON).await;
    let r = get_battery(&h, &Via::Tailnet).await;
    let json = battery_body(&r);
    assert_eq!(String::from_utf8(r.body.clone()).unwrap(), PRESENT_82_JSON);
    assert_eq!(json["percent"], 82);
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert_eq!(r.header("content-security-policy"), Some(CSP));

    let head = h.request("HEAD", "/api/battery", &[]).await;
    assert_eq!(head.status, 200);
    head.assert_mandatory_headers();
    assert_eq!(head.header("content-type"), Some("application/json"));
    assert!(head.body.is_empty(), "HEAD carries no body");

    let post = h.request("POST", "/api/battery", &[]).await;
    assert_eq!(post.status, 405);
    assert_eq!(post.header("allow"), Some("GET, HEAD"));
}

/// Test 21 (RBS8, RBS-I1, B-5): visibility equals `/api/status`: a foreign tailnet login is
/// `403`, anonymous Funnel is `403 login_required` without any source read, a Funnel
/// session is `200`, a wrong host is `421`.
#[tokio::test(start_paused = true)]
async fn test_rbs_battery_visibility_matches_status() {
    let source = ScriptedBattery::new(present(64, ChargeStatus::Charging, Some(true)));
    let (h, _j) = start(Some(Arc::new(source.clone())), Wiring::ON).await;

    let r = h
        .raw(&raw_request(
            "GET",
            "/api/battery",
            &[
                ("Host", HOST),
                ("Tailscale-User-Login", "intruder@example.com"),
            ],
        ))
        .await;
    assert_result(&r, 403, "forbidden");
    let status = h
        .raw(&raw_request(
            "GET",
            "/api/status",
            &[
                ("Host", HOST),
                ("Tailscale-User-Login", "intruder@example.com"),
            ],
        ))
        .await;
    assert_eq!(status.status, r.status, "same answer as /api/status");

    let anonymous = Via::funnel(IP_A);
    assert_result(&get_battery(&h, &anonymous).await, 403, "login_required");
    let head = h.send(&anonymous, "HEAD", "/api/battery", &[], None).await;
    assert_eq!(head.status, 403, "anonymous Funnel HEAD");
    assert!(head.body.is_empty());
    match h.open_stream_via(&anonymous).await {
        Err(Outcome::Response(r)) => assert_result(&r, 403, "login_required"),
        Ok(_) => panic!("an anonymous Funnel stream must be refused"),
        Err(other) => panic!("unexpected outcome {other:?}"),
    }

    let wrong_host = h
        .raw(&raw_request(
            "GET",
            "/api/battery",
            &[
                ("Host", "evil.example.com"),
                ("Tailscale-User-Login", LOGIN),
            ],
        ))
        .await;
    assert_eq!(wrong_host.status, 421);
    settle().await;
    assert_eq!(source.calls(), 0, "no source read before every gate passed");

    let s = h.session(IP_B).await;
    let json = battery_body(&get_battery(&h, &s).await);
    assert_eq!(json["state"], "present");
    assert_eq!(json["percent"], 64);
    assert_eq!(source.calls(), 1);
    let json = battery_body(&get_battery(&h, &Via::Tailnet).await);
    assert_eq!(json["percent"], 64);
}

/// Test 22 (RBS1, RBS-I8): `battery_status = false` with a wired source is `disabled`, the
/// source is never called and no `battery` frame is ever sent; enabled but not wired is
/// `disabled` too.
#[tokio::test(start_paused = true)]
async fn test_rbs_disabled_or_unwired() {
    let source = ScriptedBattery::new(present(64, ChargeStatus::Charging, Some(true)));
    {
        let (h, _j) = start(
            Some(Arc::new(source.clone())),
            Wiring {
                enabled: false,
                alerts: false,
            },
        )
        .await;
        let r = get_battery(&h, &Via::Tailnet).await;
        battery_body(&r);
        assert_eq!(String::from_utf8(r.body.clone()).unwrap(), DISABLED_JSON);
        let mut stream = open(&h, &Via::Tailnet).await;
        assert_eq!(next_frame(&mut stream).await.name, "status");
        let probe = no_probe();
        let (frames, eof) =
            frames_during(&mut stream, &h, &probe, 2 * BATTERY_SAMPLE_INTERVAL_MS, 250).await;
        assert!(!eof);
        assert!(
            battery_frames(&frames).is_empty(),
            "no battery frame while disabled"
        );
        let r = get_battery(&h, &Via::Tailnet).await;
        assert_eq!(String::from_utf8(r.body.clone()).unwrap(), DISABLED_JSON);
        assert_eq!(source.calls(), 0, "a disabled source is never read");
    }
    {
        // The default configuration (on) without a wired source.
        let (h, _j) = start(None, Wiring::ON).await;
        let r = get_battery(&h, &Via::Tailnet).await;
        battery_body(&r);
        assert_eq!(String::from_utf8(r.body.clone()).unwrap(), DISABLED_JSON);
        let mut stream = open(&h, &Via::Tailnet).await;
        assert_eq!(next_frame(&mut stream).await.name, "status");
        let probe = no_probe();
        let (frames, _) =
            frames_during(&mut stream, &h, &probe, 2 * BATTERY_SAMPLE_INTERVAL_MS, 250).await;
        assert!(
            battery_frames(&frames).is_empty(),
            "no battery frame when unwired"
        );
    }
    {
        // The shared harness (every existing server test) never wires a battery.
        let h = Harness::start().await;
        let r = h.get("/api/battery").await;
        battery_body(&r);
        assert_eq!(String::from_utf8(r.body.clone()).unwrap(), DISABLED_JSON);
    }
}

// ---------------------------------------------------------------------------------------
// Tests 23, 24: stream frames
// ---------------------------------------------------------------------------------------

/// Test 23 (RBS9, §6.3, F-1, F-3 b): the first frames are `status`, (`alerts`), `battery`;
/// the first battery frame carries the exact view, and one sampler period later with sysfs
/// unchanged no further battery frame (in particular no `unavailable`) arrives.
#[tokio::test(start_paused = true)]
async fn test_rbs_stream_first_frames_order() {
    for alerts in [false, true] {
        let sysfs = laptop();
        let (source, probe) = tracked(&sysfs);
        let (h, _j) = start(
            Some(source),
            Wiring {
                enabled: true,
                alerts,
            },
        )
        .await;
        let mut stream = open(&h, &Via::Tailnet).await;
        let first = next_frame(&mut stream).await;
        assert_eq!(first.name, "status", "alerts={alerts}");
        assert_eq!(first.json["state"], "unlocked");
        if alerts {
            assert_eq!(next_frame(&mut stream).await.name, "alerts");
        }
        let battery = next_frame(&mut stream).await;
        assert_eq!(battery.name, "battery", "alerts={alerts}");
        assert_eq!(battery.data, PRESENT_82_JSON, "alerts={alerts}");

        let (frames, eof) = frames_during(
            &mut stream,
            &h,
            &probe,
            BATTERY_SAMPLE_INTERVAL_MS,
            BATTERY_SAMPLE_INTERVAL_MS,
        )
        .await;
        assert!(!eof);
        assert!(
            battery_frames(&frames).is_empty(),
            "no further battery frame with sysfs unchanged: {frames:?}"
        );
        assert!(
            frames.iter().all(|(_, f)| f.data != UNAVAILABLE_JSON),
            "never a fabricated unavailable"
        );
        assert!(probe.started() >= 2, "the sampler re-read after one period");
    }
}

/// Test 24 (RBS9, B-6): a level or plug change reaches the stream within one sampler
/// period; nothing is sent while sysfs is unchanged, while status keep-alives continue.
#[tokio::test(start_paused = true)]
async fn test_rbs_stream_live_update_on_change_only() {
    let sysfs = laptop();
    let (source, probe) = tracked(&sysfs);
    let (h, _j) = start(Some(source), Wiring::ON).await;
    let mut stream = open(&h, &Via::Tailnet).await;
    assert_eq!(next_frame(&mut stream).await.name, "status");
    assert_eq!(next_frame(&mut stream).await.data, PRESENT_82_JSON);

    sysfs.set("BAT0", "capacity", b"81\n");
    let (frames, _) = frames_during(
        &mut stream,
        &h,
        &probe,
        BATTERY_SAMPLE_INTERVAL_MS + 1000,
        250,
    )
    .await;
    let updates = battery_frames(&frames);
    assert_eq!(updates.len(), 1, "exactly one battery frame: {frames:?}");
    assert_eq!(updates[0].json["percent"], 81);
    assert_eq!(updates[0].json["charge"], "discharging");
    let at = frames
        .iter()
        .find(|(_, f)| f.name == "battery")
        .map(|(t, _)| *t)
        .unwrap();
    assert!(
        at <= BATTERY_SAMPLE_INTERVAL_MS,
        "within one period, got {at} ms"
    );

    let (frames, eof) =
        frames_during(&mut stream, &h, &probe, 3 * BATTERY_SAMPLE_INTERVAL_MS, 250).await;
    assert!(!eof);
    assert!(
        battery_frames(&frames).is_empty(),
        "unchanged: no battery frame"
    );
    assert!(status_frames(&frames) >= 1, "status keep-alives continue");

    sysfs.set("ADP0", "online", b"1\n");
    let (frames, _) = frames_during(
        &mut stream,
        &h,
        &probe,
        BATTERY_SAMPLE_INTERVAL_MS + 1000,
        250,
    )
    .await;
    let updates = battery_frames(&frames);
    assert_eq!(
        updates.len(),
        1,
        "one frame for the plug change: {frames:?}"
    );
    assert_eq!(updates[0].json["external_power"], true);
    assert_eq!(updates[0].json["percent"], 81);
}

// ---------------------------------------------------------------------------------------
// Tests 25, 26: hung and panicking sources
// ---------------------------------------------------------------------------------------

/// Test 25 (RBS6, B-6, §5.5 step 5, R2-2): a read stuck in a hung driver is bounded by
/// `BATTERY_READ_TIMEOUT_MS`, never spawns a second read while it is stuck, never blocks the
/// stream or its keep-alives, and reads resume once it returns.
#[tokio::test(start_paused = true)]
async fn test_rbs_hung_source_is_bounded() {
    assert_eq!(BATTERY_READ_TIMEOUT_MS, 500);
    assert_eq!(BATTERY_SAMPLE_INTERVAL_MS, 5000);
    let source = ScriptedBattery::new(present(77, ChargeStatus::Charging, Some(true)));
    let guard = source.gate();
    let (h, _j) = start(Some(Arc::new(source.clone())), Wiring::ON).await;

    // Step 1: the first GET times out.
    let pending = spawn_exchange(
        &h.path,
        request_bytes(&Via::Tailnet, "GET", "/api/battery", &[], None),
    );
    wait_until("the read to enter the source", || source.entered() >= 1).await;
    assert!(
        !finished_soon(&pending).await,
        "the GET waits for the bounded read"
    );
    h.advance_ms(BATTERY_READ_TIMEOUT_MS).await;
    let first = match pending.await.unwrap() {
        Outcome::Response(r) => r,
        other => panic!("no response after the read bound: {other:?}"),
    };
    let json = battery_body(&first);
    assert_eq!(
        String::from_utf8(first.body.clone()).unwrap(),
        UNAVAILABLE_JSON,
        "{json}"
    );
    assert_eq!(source.calls(), 1);

    // Step 2: one millisecond past the cache lifetime, the second GET can only be answered
    // by the still-stuck read (no new spawn).
    h.advance_ms(BATTERY_SAMPLE_INTERVAL_MS + 1).await;
    let second = get_battery(&h, &Via::Tailnet).await;
    assert_eq!(
        String::from_utf8(second.body.clone()).unwrap(),
        UNAVAILABLE_JSON
    );
    assert_eq!(source.calls(), 1, "no second read while the first is stuck");

    // Step 3: a stream still opens, gets its first frames and its keep-alive.
    let mut stream = open(&h, &Via::Tailnet).await;
    assert_eq!(next_frame(&mut stream).await.name, "status");
    let battery = next_frame(&mut stream).await;
    assert_eq!(battery.name, "battery");
    assert_eq!(battery.data, UNAVAILABLE_JSON);
    let probe = no_probe();
    let (frames, eof) = frames_during(&mut stream, &h, &probe, SSE_KEEPALIVE_MS, 250).await;
    assert!(!eof);
    assert!(
        status_frames(&frames) >= 1,
        "a status keep-alive arrives: {frames:?}"
    );
    assert!(
        battery_frames(&frames).is_empty(),
        "equal unavailable views are filtered"
    );
    assert_eq!(source.calls(), 1, "every sampler tick ends in step 5");
    drop(stream);
    for _ in 0..4 {
        settle().await;
    }

    // Step 4: the driver returns; reads resume.
    drop(guard);
    wait_until("the stuck read to return", || source.returned() >= 1).await;
    std::thread::sleep(Duration::from_millis(50));
    settle().await;
    h.advance_ms(BATTERY_SAMPLE_INTERVAL_MS + 1).await;
    let json = battery_body(&get_battery(&h, &Via::Tailnet).await);
    assert_eq!(json["state"], "present");
    assert_eq!(json["percent"], 77);
    assert_eq!(source.calls(), 2);
}

/// Test 26 (RBS6, §5.5 step 5): a panicking source gives `unavailable`, the server keeps
/// serving, and the in-flight flag is cleared so later reads work.
#[tokio::test(start_paused = true)]
async fn test_rbs_panicking_source_is_unavailable() {
    let source = ScriptedBattery::panicking();
    let (h, _j) = start(Some(Arc::new(source.clone())), Wiring::ON).await;
    let r = get_battery(&h, &Via::Tailnet).await;
    battery_body(&r);
    assert_eq!(String::from_utf8(r.body.clone()).unwrap(), UNAVAILABLE_JSON);
    assert_eq!(source.calls(), 1);
    let status = h.get("/api/status").await;
    assert_eq!(status.status, 200, "the server keeps serving");
    assert_status_view(&status.json());

    source.set_view(present(55, ChargeStatus::Full, Some(true)));
    source.set_mode(Mode::Immediate);
    h.advance_ms(BATTERY_SAMPLE_INTERVAL_MS + 1).await;
    let json = battery_body(&get_battery(&h, &Via::Tailnet).await);
    assert_eq!(json["state"], "present", "the in-flight flag was cleared");
    assert_eq!(json["percent"], 55);
    assert_eq!(source.calls(), 2);
    let mut stream = open(&h, &Via::Tailnet).await;
    assert_eq!(next_frame(&mut stream).await.name, "status");
    assert_eq!(next_frame(&mut stream).await.json["percent"], 55);
}

// ---------------------------------------------------------------------------------------
// Tests 27, 28: sampler and cache
// ---------------------------------------------------------------------------------------

/// Test 27 (RBS9, B-6): the sampler reads only while at least one stream is open.
#[tokio::test(start_paused = true)]
async fn test_rbs_sampler_idle_without_streams() {
    let scripted = ScriptedBattery::new(present(50, ChargeStatus::Discharging, Some(false)));
    let source = Tracked::new(scripted.clone());
    let probe = source.probe();
    let (h, _j) = start(Some(Arc::new(source)), Wiring::ON).await;

    for _ in 0..(3 * BATTERY_SAMPLE_INTERVAL_MS / 250) {
        h.advance_ms(250).await;
        quiesce(&probe).await;
    }
    assert_eq!(scripted.calls(), 0, "no stream, no read");

    let mut stream = open(&h, &Via::Tailnet).await;
    assert_eq!(next_frame(&mut stream).await.name, "status");
    assert_eq!(next_frame(&mut stream).await.name, "battery");
    let at_open = scripted.calls();
    assert!(at_open >= 1);
    let (_, eof) = frames_during(
        &mut stream,
        &h,
        &probe,
        2 * BATTERY_SAMPLE_INTERVAL_MS + 250,
        250,
    )
    .await;
    assert!(!eof);
    assert!(
        scripted.calls() >= at_open + 2,
        "the sampler reads every period while streaming ({} → {})",
        at_open,
        scripted.calls()
    );

    drop(stream);
    for _ in 0..4 {
        settle().await;
    }
    quiesce(&probe).await;
    let after_close = scripted.calls();
    for _ in 0..(3 * BATTERY_SAMPLE_INTERVAL_MS / 250) {
        h.advance_ms(250).await;
        quiesce(&probe).await;
    }
    assert_eq!(
        scripted.calls(),
        after_close,
        "reads stop with the last stream"
    );
}

/// Test 28 (RBS9, R2-2): with no stream open, `GET /api/battery` reuses an entry up to and
/// including an age of `BATTERY_SAMPLE_INTERVAL_MS`, and reads anew one millisecond later.
#[tokio::test(start_paused = true)]
async fn test_rbs_get_uses_cache_within_interval() {
    let source = ScriptedBattery::new(present(70, ChargeStatus::Discharging, Some(false)));
    let (h, _j) = start(Some(Arc::new(source.clone())), Wiring::ON).await;
    let read = |r: HttpResponse| battery_body(&r)["percent"].clone();

    assert_eq!(read(get_battery(&h, &Via::Tailnet).await), 70);
    assert_eq!(source.calls(), 1);
    source.set_view(present(69, ChargeStatus::Discharging, Some(false)));
    assert_eq!(read(get_battery(&h, &Via::Tailnet).await), 70, "cache hit");
    assert_eq!(source.calls(), 1);
    h.advance_ms(BATTERY_SAMPLE_INTERVAL_MS).await;
    assert_eq!(
        read(get_battery(&h, &Via::Tailnet).await),
        70,
        "an entry exactly one interval old is reused"
    );
    assert_eq!(source.calls(), 1);
    h.advance_ms(1).await;
    assert_eq!(
        read(get_battery(&h, &Via::Tailnet).await),
        69,
        "one millisecond older is read anew"
    );
    assert_eq!(source.calls(), 2);
    let head = h.request("HEAD", "/api/battery", &[]).await;
    assert_eq!(head.status, 200);
    assert_eq!(
        source.calls(),
        2,
        "HEAD within the interval uses the cache too"
    );
}

// ---------------------------------------------------------------------------------------
// Tests 29–31: Funnel re-validation, status contract, logging
// ---------------------------------------------------------------------------------------

/// Test 29 (RBS9, RBS-I1, F-3 c): a Funnel stream re-validates its session before every
/// battery event: once the session is revoked, the next change ends the stream without a
/// battery frame, well before the keep-alive session check. The control half proves the
/// change would have been sent.
#[tokio::test(start_paused = true)]
async fn test_rbs_funnel_session_revalidated_before_battery_event() {
    const { assert!(BATTERY_SAMPLE_INTERVAL_MS + 1000 < SSE_KEEPALIVE_MS) };
    for revoke in [false, true] {
        let sysfs = laptop();
        let (source, probe) = tracked(&sysfs);
        let (h, _j) = start(Some(source), Wiring::ON).await;
        let s = h.session(if revoke { IP_C } else { IP_A }).await;
        let mut stream = open(&h, &s).await;
        assert_eq!(next_frame(&mut stream).await.name, "status");
        assert_eq!(next_frame(&mut stream).await.data, PRESENT_82_JSON);
        quiesce(&probe).await;
        if revoke {
            assert_result(&h.logout(&s).await, 200, "logged_out");
        }
        sysfs.set("BAT0", "capacity", b"80\n");
        let (frames, eof) =
            frames_during(&mut stream, &h, &probe, BATTERY_SAMPLE_INTERVAL_MS, 250).await;
        if revoke {
            assert!(
                eof,
                "the revoked session's stream ends before the keep-alive check"
            );
            assert!(
                battery_frames(&frames).is_empty(),
                "no battery frame after the revocation: {frames:?}"
            );
        } else {
            let updates = battery_frames(&frames);
            assert_eq!(updates.len(), 1, "control: the change is sent: {frames:?}");
            assert_eq!(updates[0].json["percent"], 80);
            assert!(!eof);
        }
    }
}

/// Test 30 (RBS10, RBS-I7): with the battery wired, `/api/status` and every `status` frame
/// keep the exact five-key shape.
#[tokio::test(start_paused = true)]
async fn test_rbs_status_contract_unchanged_with_battery() {
    let sysfs = laptop();
    let (source, probe) = tracked(&sysfs);
    let (h, _j) = start(Some(source), Wiring::ON).await;
    let r = h.get("/api/status").await;
    assert_eq!(r.status, 200);
    r.assert_json_body();
    assert_status_view(&r.json());
    let s = h.session(IP_A).await;
    let r = h.get_via(&s, "/api/status").await;
    assert_status_view(&r.json());

    let mut stream = open(&h, &Via::Tailnet).await;
    let first = next_frame(&mut stream).await;
    assert_eq!(first.name, "status");
    assert_status_view(&first.json);
    h.source.set_locked();
    sysfs.set("BAT0", "capacity", b"79\n");
    let (frames, _) = frames_during(
        &mut stream,
        &h,
        &probe,
        SSE_KEEPALIVE_MS + BATTERY_SAMPLE_INTERVAL_MS,
        250,
    )
    .await;
    let statuses: Vec<&Frame> = frames
        .iter()
        .map(|(_, f)| f)
        .filter(|f| f.name == "status")
        .collect();
    assert!(statuses.len() >= 2, "a change and a keep-alive: {frames:?}");
    for f in statuses {
        assert_status_view(&f.json);
        for forbidden in ["percent", "battery", "charge", "external_power"] {
            assert!(
                !f.data.contains(forbidden),
                "{} carries {forbidden}",
                f.data
            );
        }
    }
    assert!(!battery_frames(&frames).is_empty(), "the battery is wired");
}

/// Test 31 (RBS7, RBS-I5, F-3 d, R2-4): no battery value, field or supply name ever reaches
/// the log, at any level. The bare words `capacity` and `percent` are not markers (fixed log
/// text such as "funnel capacity reached" contains the first).
#[tokio::test(start_paused = true)]
async fn test_rbs_never_logs_battery_values() {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let sysfs = FakeSysfs::new();
    sysfs.battery(
        "BATMARKER0",
        &[("capacity", b"87\n"), ("status", b"Discharging\n")],
    );
    sysfs.mains("ADPMARKER1", "1");
    let (source, probe) = tracked(&sysfs);
    let (h, _j) = start(Some(source), Wiring::ON).await;
    let json = battery_body(&get_battery(&h, &Via::Tailnet).await);
    assert_eq!(json["percent"], 87);
    let _ = h.request("HEAD", "/api/battery", &[]).await;
    let s = h.session(IP_A).await;
    let _ = get_battery(&h, &s).await;
    let _ = get_battery(&h, &Via::funnel(IP_B)).await;
    let mut stream = open(&h, &Via::Tailnet).await;
    assert_eq!(next_frame(&mut stream).await.name, "status");
    assert_eq!(next_frame(&mut stream).await.json["percent"], 87);
    sysfs.set("BATMARKER0", "capacity", b"86\n");
    let (frames, _) = frames_during(
        &mut stream,
        &h,
        &probe,
        BATTERY_SAMPLE_INTERVAL_MS + 1000,
        250,
    )
    .await;
    assert_eq!(battery_frames(&frames).len(), 1, "one live update");
    assert_eq!(battery_frames(&frames)[0].json["percent"], 86);
    drop(stream);
    settle().await;
    let _ = h.shutdown().await;

    let log = capture.text();
    for forbidden in [
        "capacity=",
        "capacity:",
        "percent=",
        "percent:",
        "\"percent\":",
        "external_power=",
        "external_power:",
        "charge=",
        "discharging",
        "Discharging",
        "=87",
        ": 87",
        "87%",
        "=86",
        "86%",
        "BATMARKER0",
        "ADPMARKER1",
        "MARKER",
    ] {
        assert!(
            !log.contains(forbidden),
            "the log must not contain {forbidden:?}:\n{log}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 38: single-flight
// ---------------------------------------------------------------------------------------

/// Reads an SSE response head written on `stream`, waiting in real time (bounded).
async fn read_stream_head(stream: tokio::net::UnixStream) -> SseClient {
    let mut buf = Vec::new();
    let start = std::time::Instant::now();
    loop {
        if let Some((head, body_start)) = parse_head(&buf) {
            assert_sse_head(&head);
            let rest = buf[body_start..].to_vec();
            return SseClient {
                stream,
                buf: rest,
                head,
            };
        }
        let mut chunk = [0u8; 4096];
        match stream.try_read(&mut chunk) {
            Ok(0) => panic!("stream closed before its head"),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                yield_now().await;
                assert!(start.elapsed() < REAL_WAIT, "no stream head");
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => panic!("stream read failed: {e}"),
        }
    }
}

/// Test 38 (RBS6, RBS9, F-1): concurrent callers (two HTTP-style readers and the sampler; a
/// stream open and a GET) share one real read and all receive the real value; nothing is
/// cached as `unavailable`.
#[tokio::test(start_paused = true)]
async fn test_rbs_concurrent_readers_share_one_read() {
    let delay = Duration::from_millis(50);
    let view = present(64, ChargeStatus::Discharging, Some(false));

    // (i) Runtime level.
    {
        let _frozen = FrozenClock::hold();
        let source = ScriptedBattery::delayed(view, delay);
        let runtime = Arc::new(BatteryRuntime::new(Arc::new(source.clone())));
        let (a, b, c) = tokio::join!(runtime.current(), runtime.current(), runtime.sample());
        assert_eq!(a, view);
        assert_eq!(b, view);
        assert_eq!(c, Some(view));
        assert_eq!(
            source.calls(),
            1,
            "one real read for three concurrent callers"
        );
        let _receiver = runtime.subscribe();
    }

    // (ii) Server level: a stream request and a GET written before either answer is read.
    let source = ScriptedBattery::delayed(view, delay);
    let (h, _j) = start(Some(Arc::new(source.clone())), Wiring::ON).await;
    let mut raw = connect(&h.path).await;
    raw.write_all(&request_bytes(
        &Via::Tailnet,
        "GET",
        "/api/events",
        &[],
        None,
    ))
    .await
    .unwrap();
    let get = spawn_exchange(
        &h.path,
        request_bytes(&Via::Tailnet, "GET", "/api/battery", &[], None),
    );
    let mut stream = read_stream_head(raw).await;
    assert_eq!(next_frame(&mut stream).await.name, "status");
    let battery = next_frame(&mut stream).await;
    assert_eq!(battery.name, "battery");
    assert_eq!(battery.json["state"], "present", "{}", battery.data);
    assert_eq!(battery.json["percent"], 64);
    let response = match get.await.unwrap() {
        Outcome::Response(r) => r,
        other => panic!("no GET answer: {other:?}"),
    };
    let json = battery_body(&response);
    assert_eq!(json["state"], "present");
    assert_eq!(json["percent"], 64);
    quiesce_scripted(&source).await;
    assert_eq!(
        source.calls(),
        1,
        "the stream, the sampler and the GET shared one read"
    );

    // (iii) Nothing was cached as unavailable.
    h.advance_ms(BATTERY_READ_TIMEOUT_MS).await;
    let json = battery_body(&get_battery(&h, &Via::Tailnet).await);
    assert_eq!(json["state"], "present");
    assert_eq!(json["percent"], 64);
    assert_eq!(source.calls(), 1);
    let (frames, _) = frames_now(&mut stream).await;
    assert!(
        frames.iter().all(|f| f.data != UNAVAILABLE_JSON),
        "no unavailable anywhere: {frames:?}"
    );
}

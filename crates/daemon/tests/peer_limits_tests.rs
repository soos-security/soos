//! Contractual tests for per-peer connection limits (GitHub #157, DMN-04) and the
//! per-peer `PasswordFailed` event quota (GitHub #175, DMN-03 hardening).
//!
//! Contract:
//! - Connection permits are keyed on the kernel `SO_PEERCRED` UID, never on payload data.
//! - An unprivileged UID holds at most `max_connections_per_uid` concurrent connections.
//! - `reserved_root_connections` permits are usable only by root peers (the PAM module
//!   runs in root processes for gdm, sudo, login and su), so unprivileged peers can never
//!   exhaust the global capacity and starve face login for everyone.
//! - A capped peer's connection is closed immediately (bounded, never an unbounded wait);
//!   the PAM client maps that EOF to `PAM_IGNORE`
//!   (`crates/pam/tests/ipc_tests.rs::test_ipc_daemon_crash_immediate_disconnect_returns_ignore`).
//! - A connection serves at most `max_requests_per_connection` requests, lives at most
//!   `max_connection_lifetime`, and is closed right after an `Auth` response (one-shot).
//! - `PasswordFailed` events are rate limited per peer UID (root included).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat};
use soos_daemon::config::{DaemonConfig, DispatcherConfig};
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::limits::{
    ConnectionRejection, PeerConnectionLimiter, PeerLimitsConfig, DEFAULT_EVENT_WINDOW_MS,
    DEFAULT_MAX_CONNECTIONS_PER_UID, DEFAULT_MAX_CONNECTION_LIFETIME_MS,
    DEFAULT_MAX_EVENTS_PER_WINDOW, DEFAULT_MAX_REQUESTS_PER_CONNECTION,
    DEFAULT_RESERVED_ROOT_CONNECTIONS,
};
use soos_daemon::pipeline::PipelineComponents;
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    Event, EventKind, Request, RequestKind, Response, StatusResponse, Verdict, CURRENT_VERSION,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

const ROOT: u32 = 0;
/// Upper bound for "the daemon closed the connection promptly".
const CLOSE_DEADLINE: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dispatcher_config(max_concurrent_connections: usize) -> DispatcherConfig {
    DispatcherConfig {
        max_concurrent_connections,
        connection_timeout: Duration::from_millis(2000),
        enforce_active_session: false,
        logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
    }
}

fn current_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

fn status_request(uid: u32, nonce: u8) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Status,
        request_id: [nonce; 32],
        uid_hint: uid,
        service: "soos-admin".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

fn auth_request(uid: u32) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [0x5A; 32],
        uid_hint: uid,
        service: "sudo".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

async fn spawn_server(dispatcher: Arc<ConnectionDispatcher>, sock_path: &std::path::Path) {
    let listener = UnixListener::bind(sock_path).expect("Bind failed");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let d = dispatcher.clone();
            tokio::spawn(async move {
                let _ = d.handle_connection(stream).await;
            });
        }
    });
}

/// Sends one frame and reads one length-prefixed reply (prefix included).
async fn exchange(client: &mut UnixStream, req: &Request) -> Vec<u8> {
    let framed = encode(req).expect("Encode request");
    client.write_all(&framed).await.expect("Write request");
    client.flush().await.expect("Flush request");
    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.expect("Read len");
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("Read body");
    buf
}

async fn status_roundtrip(client: &mut UnixStream, nonce: u8) {
    let buf = exchange(client, &status_request(current_uid(), nonce)).await;
    let status: StatusResponse = decode(&buf).expect("Decode status response");
    assert_eq!(status.version, CURRENT_VERSION);
}

/// Returns true when the daemon closed the stream (EOF or reset) within `CLOSE_DEADLINE`.
/// Panics if the daemon sent bytes instead.
async fn closed_promptly(client: &mut UnixStream) -> bool {
    let mut byte = [0u8; 1];
    match tokio::time::timeout(CLOSE_DEADLINE, client.read(&mut byte)).await {
        Ok(Ok(0)) => true,
        Ok(Ok(n)) => panic!("Capped connection must not receive a payload, got {n} byte(s)"),
        Ok(Err(e)) => matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
        ),
        Err(_) => false,
    }
}

/// Sends a frame, ignoring write failures (the daemon may already have closed the stream).
async fn send_ignoring_errors(client: &mut UnixStream, req: &Request) {
    let framed = encode(req).expect("Encode request");
    let _ = client.write_all(&framed).await;
    let _ = client.flush().await;
}

// ---------------------------------------------------------------------------
// Configuration contract
// ---------------------------------------------------------------------------

#[test]
fn test_peer_limits_defaults_are_bounded_and_valid() {
    let cfg = PeerLimitsConfig::default();
    assert_eq!(cfg.max_connections_per_uid, DEFAULT_MAX_CONNECTIONS_PER_UID);
    assert_eq!(
        cfg.reserved_root_connections,
        DEFAULT_RESERVED_ROOT_CONNECTIONS
    );
    assert_eq!(
        cfg.max_requests_per_connection,
        DEFAULT_MAX_REQUESTS_PER_CONNECTION
    );
    assert_eq!(
        cfg.max_connection_lifetime,
        Duration::from_millis(DEFAULT_MAX_CONNECTION_LIFETIME_MS)
    );
    assert_eq!(cfg.max_events_per_window, DEFAULT_MAX_EVENTS_PER_WINDOW);
    assert_eq!(
        cfg.event_window,
        Duration::from_millis(DEFAULT_EVENT_WINDOW_MS)
    );

    assert_eq!(DEFAULT_MAX_CONNECTIONS_PER_UID, 2);
    assert_eq!(DEFAULT_RESERVED_ROOT_CONNECTIONS, 2);
    const { assert!(DEFAULT_MAX_REQUESTS_PER_CONNECTION >= 64) };
    const { assert!(DEFAULT_MAX_CONNECTION_LIFETIME_MS <= 60_000) };
    const { assert!(DEFAULT_MAX_EVENTS_PER_WINDOW >= 1) };

    let daemon_defaults = DispatcherConfig::default();
    cfg.validate(daemon_defaults.max_concurrent_connections)
        .expect("Defaults must be valid for the default capacity");
    // Unprivileged peers together keep capacity for the GUI and screen lockers.
    assert!(
        daemon_defaults.max_concurrent_connections > DEFAULT_RESERVED_ROOT_CONNECTIONS,
        "At least one permit must remain usable by unprivileged peers"
    );
}

#[test]
fn test_peer_limits_validate_rejects_unsafe_values() {
    let base = PeerLimitsConfig::default();
    assert!(
        PeerLimitsConfig {
            reserved_root_connections: 8,
            ..base.clone()
        }
        .validate(8)
        .is_err(),
        "Reserving the whole capacity for root must be rejected"
    );
    assert!(PeerLimitsConfig {
        max_connections_per_uid: 0,
        ..base.clone()
    }
    .validate(8)
    .is_err());
    assert!(PeerLimitsConfig {
        max_requests_per_connection: 0,
        ..base.clone()
    }
    .validate(8)
    .is_err());
    assert!(PeerLimitsConfig {
        max_connection_lifetime: Duration::ZERO,
        ..base.clone()
    }
    .validate(8)
    .is_err());
    assert!(PeerLimitsConfig {
        event_window: Duration::ZERO,
        ..base.clone()
    }
    .validate(8)
    .is_err());
    assert!(base.validate(0).is_err(), "Zero capacity is invalid");
}

#[test]
fn test_daemon_config_parses_peer_limits_section() {
    let config = DaemonConfig::from_toml_str(
        r#"
[peer_limits]
max_connections_per_uid = 3
reserved_root_connections = 1
max_requests_per_connection = 100
max_connection_lifetime_ms = 5000
max_events_per_window = 7
event_window_ms = 20000
"#,
    )
    .expect("Valid [peer_limits] section");
    assert_eq!(config.peer_limits.max_connections_per_uid, 3);
    assert_eq!(config.peer_limits.reserved_root_connections, 1);
    assert_eq!(config.peer_limits.max_requests_per_connection, 100);
    assert_eq!(
        config.peer_limits.max_connection_lifetime,
        Duration::from_millis(5000)
    );
    assert_eq!(config.peer_limits.max_events_per_window, 7);
    assert_eq!(
        config.peer_limits.event_window,
        Duration::from_millis(20000)
    );

    let defaults = DaemonConfig::from_toml_str("").expect("Empty config");
    assert_eq!(defaults.peer_limits, PeerLimitsConfig::default());
}

#[test]
fn test_daemon_config_rejects_reservation_not_below_capacity() {
    let res = DaemonConfig::from_toml_str(
        r#"
[dispatcher]
max_concurrent_connections = 2

[peer_limits]
reserved_root_connections = 2
"#,
    );
    assert!(
        res.is_err(),
        "A reservation that leaves no unprivileged permit must fail to load"
    );
    let res = DaemonConfig::from_toml_str(
        r#"
[peer_limits]
max_connections_per_uid = 0
"#,
    );
    assert!(res.is_err(), "A zero per-UID cap must fail to load");
}

// ---------------------------------------------------------------------------
// Pure limiter contract (UID-independent, no sockets)
// ---------------------------------------------------------------------------

#[test]
fn test_limiter_caps_connections_per_unprivileged_uid() {
    let limiter = PeerConnectionLimiter::new(8, &PeerLimitsConfig::default());
    let a1 = limiter.try_acquire(1000).expect("First permit");
    let a2 = limiter.try_acquire(1000).expect("Second permit");
    assert_eq!(
        limiter.try_acquire(1000).err(),
        Some(ConnectionRejection::PerUidCap),
        "Third concurrent connection of the same UID must be rejected"
    );
    let b1 = limiter
        .try_acquire(1001)
        .expect("Another UID is unaffected");
    assert_eq!(limiter.in_use(), 3);
    assert_eq!(limiter.available(), 5);
    drop(a1);
    let a3 = limiter
        .try_acquire(1000)
        .expect("A released permit is reusable by the same UID");
    drop((a2, a3, b1));
    assert_eq!(limiter.in_use(), 0);
    assert_eq!(limiter.available(), 8);
}

/// Issue #157 acceptance scenario: 8 unprivileged connection attempts must not prevent a
/// 9th root-peer (PAM) connection from being served.
#[test]
fn test_limiter_reserves_permits_for_root_peers() {
    let limiter = PeerConnectionLimiter::new(8, &PeerLimitsConfig::default());
    let mut held = Vec::new();
    let mut rejected = Vec::new();
    for i in 0..8u32 {
        match limiter.try_acquire(1000 + i / 2) {
            Ok(permit) => held.push(permit),
            Err(r) => rejected.push(r),
        }
    }
    assert_eq!(
        held.len(),
        6,
        "Unprivileged peers may use capacity - reserve"
    );
    assert_eq!(
        rejected,
        vec![ConnectionRejection::ReservedForPrivileged; 2]
    );

    let r1 = limiter
        .try_acquire(ROOT)
        .expect("Root PAM peer must be served");
    let r2 = limiter
        .try_acquire(ROOT)
        .expect("Root uses the second reserved slot");
    assert_eq!(
        limiter.try_acquire(ROOT).err(),
        Some(ConnectionRejection::GlobalCapacity),
        "The global capacity still bounds root peers"
    );
    drop(r1);
    assert_eq!(
        limiter.try_acquire(2000).err(),
        Some(ConnectionRejection::ReservedForPrivileged),
        "A freed reserved slot is never handed to an unprivileged peer"
    );
    drop((r2, held));
    assert_eq!(limiter.in_use(), 0);
}

#[test]
fn test_limiter_root_is_not_subject_to_the_per_uid_cap() {
    let limiter = PeerConnectionLimiter::new(4, &PeerLimitsConfig::default());
    let permits: Vec<_> = (0..4)
        .map(|_| {
            limiter
                .try_acquire(ROOT)
                .expect("Root may use every permit")
        })
        .collect();
    assert_eq!(limiter.available(), 0);
    drop(permits);
    assert_eq!(limiter.available(), 4);
}

#[test]
fn test_limiter_tracking_is_bounded_and_released() {
    let limiter = PeerConnectionLimiter::new(8, &PeerLimitsConfig::default());
    for round in 0..1000u32 {
        let permit = limiter.try_acquire(10_000 + round).expect("Permit");
        assert!(
            limiter.tracked_uids() <= 8,
            "Tracking map bounded by capacity"
        );
        drop(permit);
        assert_eq!(limiter.tracked_uids(), 0, "Released UIDs are forgotten");
    }
}

#[test]
fn test_limiter_clamps_reservation_to_leave_one_unprivileged_permit() {
    let cfg = PeerLimitsConfig {
        reserved_root_connections: 10,
        ..PeerLimitsConfig::default()
    };
    let limiter = PeerConnectionLimiter::new(2, &cfg);
    let p = limiter
        .try_acquire(1000)
        .expect("Library clamping keeps one unprivileged permit");
    assert_eq!(
        limiter.try_acquire(1001).err(),
        Some(ConnectionRejection::ReservedForPrivileged)
    );
    let r = limiter.try_acquire(ROOT).expect("Root keeps the rest");
    drop((p, r));
}

// ---------------------------------------------------------------------------
// Socket-level contract (the test peer is the current, unprivileged UID)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_same_uid_connection_over_cap_is_closed_without_blocking_others() {
    if current_uid() == 0 {
        return;
    }
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("peer_cap.sock");
    let dispatcher = Arc::new(ConnectionDispatcher::new(
        dispatcher_config(8),
        Arc::new(HealthState::new()),
    ));
    spawn_server(dispatcher.clone(), &sock_path).await;

    let mut c1 = UnixStream::connect(&sock_path).await.expect("Connect c1");
    status_roundtrip(&mut c1, 1).await;
    let mut c2 = UnixStream::connect(&sock_path).await.expect("Connect c2");
    status_roundtrip(&mut c2, 2).await;
    assert_eq!(dispatcher.available_permits(), 6);

    let mut c3 = UnixStream::connect(&sock_path).await.expect("Connect c3");
    send_ignoring_errors(&mut c3, &status_request(current_uid(), 3)).await;
    assert!(
        closed_promptly(&mut c3).await,
        "A connection over the per-UID cap must be closed immediately"
    );

    // Held connections keep working and the rejection consumed no permit.
    status_roundtrip(&mut c1, 4).await;
    assert_eq!(dispatcher.available_permits(), 6);

    drop(c1);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if dispatcher.available_permits() == 7 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Permit must be released on close"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut c4 = UnixStream::connect(&sock_path).await.expect("Connect c4");
    status_roundtrip(&mut c4, 5).await;
    drop(c2);
}

#[tokio::test]
async fn test_idle_polling_peer_cannot_exhaust_global_capacity() {
    if current_uid() == 0 {
        return;
    }
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("peer_flood.sock");
    let dispatcher = Arc::new(ConnectionDispatcher::new(
        dispatcher_config(8),
        Arc::new(HealthState::new()),
    ));
    spawn_server(dispatcher.clone(), &sock_path).await;

    let mut served = Vec::new();
    let mut refused = 0usize;
    for i in 0..8u8 {
        let mut c = UnixStream::connect(&sock_path).await.expect("Connect");
        send_ignoring_errors(&mut c, &status_request(current_uid(), i)).await;
        let mut len = [0u8; 4];
        match tokio::time::timeout(CLOSE_DEADLINE, c.read_exact(&mut len)).await {
            Ok(Ok(_)) => {
                let body_len = u32::from_be_bytes(len) as usize;
                let mut body = vec![0u8; body_len];
                c.read_exact(&mut body).await.expect("Body");
                served.push(c);
            }
            Ok(Err(_)) => refused += 1,
            Err(_) => panic!("A capped connection must never wait unboundedly"),
        }
    }
    assert_eq!(served.len(), DEFAULT_MAX_CONNECTIONS_PER_UID);
    assert_eq!(refused, 8 - DEFAULT_MAX_CONNECTIONS_PER_UID);
    assert!(
        dispatcher.available_permits() >= 8 - DEFAULT_MAX_CONNECTIONS_PER_UID,
        "One unprivileged UID must leave the remaining capacity to other peers"
    );
}

#[tokio::test]
async fn test_auth_request_is_one_shot_per_connection() {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("auth_one_shot.sock");
    let dispatcher = Arc::new(ConnectionDispatcher::new(
        dispatcher_config(4),
        Arc::new(HealthState::new()),
    ));
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    let buf = exchange(&mut client, &auth_request(current_uid())).await;
    let resp: Response = decode(&buf).expect("Decode auth response");
    assert_eq!(
        resp.verdict,
        Verdict::Unavailable,
        "No pipeline: fail closed"
    );
    assert!(
        closed_promptly(&mut client).await,
        "The daemon must close the connection right after an Auth response"
    );
}

#[tokio::test]
async fn test_connection_closed_after_max_requests() {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("request_cap.sock");
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(4), Arc::new(HealthState::new()))
            .with_peer_limits(PeerLimitsConfig {
                max_requests_per_connection: 3,
                ..PeerLimitsConfig::default()
            }),
    );
    assert_eq!(dispatcher.peer_limits().max_requests_per_connection, 3);
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    for nonce in 0..3u8 {
        status_roundtrip(&mut client, nonce).await;
    }
    assert!(
        closed_promptly(&mut client).await,
        "The connection must be closed once the request cap is reached"
    );
}

#[tokio::test]
async fn test_connection_closed_after_max_lifetime() {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("lifetime_cap.sock");
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(4), Arc::new(HealthState::new()))
            .with_peer_limits(PeerLimitsConfig {
                max_connection_lifetime: Duration::from_millis(300),
                ..PeerLimitsConfig::default()
            }),
    );
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    let start = Instant::now();
    let mut served = 0u32;
    let closed = loop {
        assert!(
            start.elapsed() < Duration::from_millis(1500),
            "A polling peer must not keep its permit past the lifetime cap"
        );
        send_ignoring_errors(&mut client, &status_request(current_uid(), 9)).await;
        let mut len = [0u8; 4];
        match tokio::time::timeout(CLOSE_DEADLINE, client.read_exact(&mut len)).await {
            Ok(Ok(_)) => {
                let body_len = u32::from_be_bytes(len) as usize;
                let mut body = vec![0u8; body_len];
                client.read_exact(&mut body).await.expect("Body");
                served += 1;
            }
            Ok(Err(_)) => break true,
            Err(_) => break false,
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(closed, "The daemon must close the connection itself");
    assert!(served >= 1, "Requests within the lifetime are served");
    assert!(
        start.elapsed() >= Duration::from_millis(300),
        "The lifetime cap must not cut the connection early"
    );
}

// ---------------------------------------------------------------------------
// PasswordFailed event quota (per peer UID)
// ---------------------------------------------------------------------------

async fn evidence_pipeline(temp: &std::path::Path) -> (PipelineComponents, Arc<EvidenceStore>) {
    let bio_key = BioMasterKey::generate().expect("Generate bio key");
    let bio_store =
        Arc::new(BiometricStore::new(temp.join("biometrics"), bio_key).expect("BioStore init"));
    let ev_dir = temp.join("evidence");
    std::fs::create_dir_all(&ev_dir).expect("Create ev dir");
    let evidence_store = Arc::new(EvidenceStore::new(
        EvidenceConfig {
            enabled: true,
            base_dir: ev_dir,
            retention_days: 1,
            daily_cap_per_uid: 100,
            key_path: temp.join("evidence.key"),
        },
        EvMasterKey::generate().expect("Generate ev key"),
    ));

    let camera_config = CameraConfigBuilder::new()
        .device_path("/dev/null")
        .resolution(320, 240)
        .fps(30)
        .idle_timeout(Duration::from_secs(60))
        .format(PixelFormat::Rgb24)
        .warmup_frames(0)
        .build();
    let camera = Arc::new(MockCameraManager::new(camera_config));
    for _ in 0..100 {
        if camera.latest_frame().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(camera.latest_frame().is_some(), "Mock camera frame");

    let vision = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_centered_face(320, 240, 0.95)),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig::default(),
    ));
    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        ThresholdConfig::default(),
        RateLimiter::new(RateLimitConfig::new(5, 60_000_000_000)),
    )));
    let components = PipelineComponents {
        camera,
        vision,
        biometric_store: bio_store,
        evidence_store: evidence_store.clone(),
        policy,
    };
    (components, evidence_store)
}

fn count_snapshots(store: &EvidenceStore) -> usize {
    let mut total = 0;
    for entry in std::fs::read_dir(&store.config().base_dir).expect("Read dir") {
        let entry = entry.expect("Dir entry");
        if entry.path().is_dir() {
            let date = entry.file_name().to_string_lossy().to_string();
            total += store.list_snapshots_for_date(&date).expect("List").len();
        }
    }
    total
}

#[tokio::test]
async fn test_password_failed_events_are_rate_limited_per_peer_uid() {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("event_quota.sock");
    let (components, evidence_store) = evidence_pipeline(dir.path()).await;
    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(
            dispatcher_config(4),
            Arc::new(HealthState::new()),
            components,
        )
        .with_peer_limits(PeerLimitsConfig {
            max_events_per_window: 2,
            event_window: Duration::from_secs(60),
            ..PeerLimitsConfig::default()
        }),
    );

    let listener = UnixListener::bind(&sock_path).expect("Bind");
    let disp = dispatcher.clone();
    let server = tokio::spawn(async move {
        for _ in 0..4 {
            if let Ok((stream, _)) = listener.accept().await {
                let _ = disp.handle_connection(stream).await;
            }
        }
    });

    let uid = current_uid();
    for nonce in 0..4u8 {
        let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
        let event = Event {
            version: CURRENT_VERSION,
            kind: EventKind::PasswordFailed,
            request_id: Some([nonce; 32]),
            uid: Some(uid),
            service: "swaylock".into(),
            timestamp_monotonic_ns: 1,
        };
        let framed = encode(&event).expect("Encode event");
        client.write_all(&framed).await.expect("Write event");
        client.flush().await.expect("Flush event");
        drop(client);
    }
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("Server finished")
        .expect("Server task");
    // Setup migration (GitHub #310): PasswordFailed evidence writes run on the blocking
    // pool; wait for every tracked write before counting snapshots.
    let _ = dispatcher
        .evidence_writes()
        .drain(Duration::from_secs(5))
        .await;

    assert_eq!(
        count_snapshots(&evidence_store),
        2,
        "Only max_events_per_window events per peer UID may trigger a snapshot"
    );
}

/// GitHub #175: an unprivileged peer reporting a failure for another UID stores nothing.
#[tokio::test]
async fn test_unprivileged_peer_cannot_report_event_for_foreign_uid() {
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("event_foreign.sock");
    let (components, evidence_store) = evidence_pipeline(dir.path()).await;
    let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
        dispatcher_config(4),
        Arc::new(HealthState::new()),
        components,
    ));
    let listener = UnixListener::bind(&sock_path).expect("Bind");
    let disp = dispatcher.clone();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            if let Ok((stream, _)) = listener.accept().await {
                let _ = disp.handle_connection(stream).await;
            }
        }
    });
    for claimed in [uid.saturating_add(42), 0] {
        let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
        let event = Event {
            version: CURRENT_VERSION,
            kind: EventKind::PasswordFailed,
            request_id: Some([3u8; 32]),
            uid: Some(claimed),
            service: "sudo".into(),
            timestamp_monotonic_ns: 1,
        };
        client
            .write_all(&encode(&event).expect("Encode"))
            .await
            .expect("Write");
        client.flush().await.expect("Flush");
    }
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("Server finished")
        .expect("Server task");
    // Setup migration (GitHub #310): PasswordFailed evidence writes run on the blocking
    // pool; wait for every tracked write before counting snapshots.
    let _ = dispatcher
        .evidence_writes()
        .drain(Duration::from_secs(5))
        .await;
    assert_eq!(
        count_snapshots(&evidence_store),
        0,
        "Foreign-UID events from an unprivileged peer must not create snapshots"
    );
}

// ---------------------------------------------------------------------------
// soos-remote security hardening S-3 (ADR 2026-10-12, spec
// AI/architect_spec_remote_security_hardening.md §4, §11.2; rows RSH4, RSH5): one
// auth-reserved connection per unprivileged UID above `max_connections_per_uid`, admitted
// only for an `Auth` request whose whole first frame arrives within
// `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS`. New tests only; nothing above is changed.
// ---------------------------------------------------------------------------

mod auth_reserve {
    use super::*;
    use soos_daemon::limits::{
        AdmissionClass, AUTH_RESERVED_CONNECTIONS_PER_UID, AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS,
    };
    use soos_protocol::types::ReasonClass;

    const A: u32 = 1000;
    const B: u32 = 1001;

    #[test]
    fn test_rsh_limiter_auth_reserve_admits_one_extra_per_uid() {
        assert_eq!(AUTH_RESERVED_CONNECTIONS_PER_UID, 1);
        let limiter = PeerConnectionLimiter::new(8, &PeerLimitsConfig::default());
        let a1 = limiter.try_acquire(A).expect("first regular permit");
        let a2 = limiter.try_acquire(A).expect("second regular permit");
        assert_eq!(
            limiter.try_acquire(A).err(),
            Some(ConnectionRejection::PerUidCap),
            "the regular cap is unchanged"
        );
        let reserve = limiter
            .try_acquire_auth_reserve(A)
            .expect("one auth-reserved permit above the cap");
        assert_eq!(reserve.peer_uid(), A);
        assert_eq!(reserve.class(), AdmissionClass::AuthReserve);
        assert_eq!(
            limiter.try_acquire_auth_reserve(A).err(),
            Some(ConnectionRejection::PerUidCap),
            "a second reserve of the same UID is refused"
        );
        assert_eq!(limiter.in_use(), 3);

        // Another UID is unaffected: its own two regular permits and its own reserve.
        let b1 = limiter.try_acquire(B).expect("other UID regular");
        let b2 = limiter.try_acquire(B).expect("other UID regular");
        let b_reserve = limiter
            .try_acquire_auth_reserve(B)
            .expect("other UID has its own reserve");
        assert_eq!(b_reserve.class(), AdmissionClass::AuthReserve);
        assert_eq!(limiter.in_use(), 6);

        // Root never uses the reserve (no per-UID cap).
        assert_eq!(
            limiter.try_acquire_auth_reserve(ROOT).err(),
            Some(ConnectionRejection::PerUidCap)
        );
        drop((a1, a2, reserve, b1, b2, b_reserve));
        assert_eq!(limiter.in_use(), 0);
        assert_eq!(limiter.available(), 8);
    }

    #[test]
    fn test_rsh_limiter_auth_reserve_respects_unprivileged_and_global_capacity() {
        // Capacity 3, one root-reserved permit: two unprivileged permits.
        let cfg = PeerLimitsConfig {
            reserved_root_connections: 1,
            ..PeerLimitsConfig::default()
        };
        let limiter = PeerConnectionLimiter::new(3, &cfg);
        let a1 = limiter.try_acquire(A).expect("regular");
        let a2 = limiter.try_acquire(A).expect("regular");
        let root = limiter.try_acquire(ROOT).expect("root reserved permit");
        assert_eq!(
            limiter.try_acquire_auth_reserve(A).err(),
            Some(ConnectionRejection::GlobalCapacity),
            "the global capacity is checked first"
        );
        drop(root);
        assert_eq!(
            limiter.try_acquire_auth_reserve(A).err(),
            Some(ConnectionRejection::ReservedForPrivileged),
            "the reserve consumes an unprivileged permit and never a root-reserved one"
        );
        assert_eq!(limiter.in_use(), 2);
        drop((a1, a2));

        // The reserve counts against the unprivileged capacity (8 - 2 = 6).
        let limiter = PeerConnectionLimiter::new(8, &PeerLimitsConfig::default());
        let held = vec![
            limiter.try_acquire(A).expect("A1"),
            limiter.try_acquire(A).expect("A2"),
            limiter.try_acquire_auth_reserve(A).expect("A reserve"),
            limiter.try_acquire(B).expect("B1"),
            limiter.try_acquire(B).expect("B2"),
            limiter.try_acquire(1002).expect("C1"),
        ];
        assert_eq!(
            limiter.try_acquire(1002).err(),
            Some(ConnectionRejection::ReservedForPrivileged),
            "six unprivileged permits in use, the reserve included"
        );
        let r1 = limiter
            .try_acquire(ROOT)
            .expect("root keeps its reservation");
        let r2 = limiter
            .try_acquire(ROOT)
            .expect("root keeps its reservation");
        assert_eq!(limiter.available(), 0);
        drop((held, r1, r2));
        assert_eq!(limiter.in_use(), 0);
    }

    #[test]
    fn test_rsh_limiter_auth_reserve_released_on_drop_and_tracked() {
        let limiter = PeerConnectionLimiter::new(8, &PeerLimitsConfig::default());
        for round in 0..1000u32 {
            let uid = 20_000 + round;
            let r1 = limiter.try_acquire(uid).expect("regular");
            let r2 = limiter.try_acquire(uid).expect("regular");
            let reserve = limiter.try_acquire_auth_reserve(uid).expect("reserve");
            assert_eq!(limiter.tracked_uids(), 1, "one UID tracked once");
            assert!(limiter.tracked_uids() <= 8, "tracking bounded by capacity");
            drop((r1, r2));
            assert_eq!(
                limiter.tracked_uids(),
                1,
                "a UID holding only its reserve is still tracked"
            );
            assert_eq!(limiter.in_use(), 1);
            drop(reserve);
            assert_eq!(limiter.tracked_uids(), 0, "released UIDs are forgotten");
            assert_eq!(limiter.in_use(), 0);
            assert_eq!(limiter.available(), 8);
        }
        // A released reserve is reusable by the same UID.
        let first = limiter.try_acquire_auth_reserve(A).expect("reserve");
        drop(first);
        let again = limiter
            .try_acquire_auth_reserve(A)
            .expect("a released reserve is reusable");
        drop(again);
        assert_eq!(limiter.in_use(), 0);
    }

    #[test]
    fn test_rsh_limiter_regular_cap_unchanged() {
        let limiter = PeerConnectionLimiter::new(8, &PeerLimitsConfig::default());
        let a1 = limiter.try_acquire(A).expect("regular");
        let a2 = limiter.try_acquire(A).expect("regular");
        let reserve = limiter.try_acquire_auth_reserve(A).expect("reserve");
        assert_eq!(
            limiter.try_acquire(A).err(),
            Some(ConnectionRejection::PerUidCap),
            "a held reserve never adds a regular permit"
        );
        drop(reserve);
        assert_eq!(
            limiter.try_acquire(A).err(),
            Some(ConnectionRejection::PerUidCap),
            "releasing the reserve frees no regular permit"
        );
        drop(a1);
        let a3 = limiter.try_acquire(A).expect("a released regular permit");
        assert_eq!(a3.class(), AdmissionClass::Regular);
        drop((a2, a3));
        assert_eq!(limiter.in_use(), 0);
    }

    #[test]
    fn test_rsh_permit_reports_admission_class() {
        let limiter = PeerConnectionLimiter::new(8, &PeerLimitsConfig::default());
        let regular = limiter.try_acquire(A).expect("regular");
        assert_eq!(regular.class(), AdmissionClass::Regular);
        assert_eq!(regular.peer_uid(), A);
        let root = limiter.try_acquire(ROOT).expect("root");
        assert_eq!(root.class(), AdmissionClass::Regular);
        let reserve = limiter.try_acquire_auth_reserve(A).expect("reserve");
        assert_eq!(reserve.class(), AdmissionClass::AuthReserve);
        assert_ne!(AdmissionClass::Regular, AdmissionClass::AuthReserve);
    }

    // -----------------------------------------------------------------------
    // Socket-level contract (the test peer is the current, unprivileged UID)
    // -----------------------------------------------------------------------

    /// The reserve window plus a scheduling margin stays inside the close deadline.
    const _: () = assert!(AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS + 100 < 500);

    /// A dispatcher with two held, served regular connections of the test UID.
    async fn two_held(
        name: &str,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<ConnectionDispatcher>,
        UnixStream,
        UnixStream,
    ) {
        let dir = tempdir().expect("tempdir");
        let sock_path = dir.path().join(name);
        let dispatcher = Arc::new(ConnectionDispatcher::new(
            dispatcher_config(8),
            Arc::new(HealthState::new()),
        ));
        spawn_server(dispatcher.clone(), &sock_path).await;
        let mut c1 = UnixStream::connect(&sock_path).await.expect("Connect c1");
        status_roundtrip(&mut c1, 1).await;
        let mut c2 = UnixStream::connect(&sock_path).await.expect("Connect c2");
        status_roundtrip(&mut c2, 2).await;
        assert_eq!(dispatcher.available_permits(), 6);
        (dir, sock_path, dispatcher, c1, c2)
    }

    /// Waits (bounded) until the dispatcher reports `expected` free permits.
    async fn permits_back(dispatcher: &ConnectionDispatcher, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while dispatcher.available_permits() != expected {
            assert!(
                Instant::now() < deadline,
                "permits must return to {expected}, got {}",
                dispatcher.available_permits()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn auth_response(buf: &[u8]) -> Response {
        let resp: Response = decode(buf).expect("Auth answered with a standard Response");
        assert_eq!(resp.request_id, [0x5A; 32], "the request id is echoed");
        resp
    }

    #[tokio::test]
    async fn test_rsh_auth_admitted_on_reserve_while_two_connections_held() {
        if current_uid() == 0 {
            return;
        }
        let (_dir, sock_path, dispatcher, mut c1, mut c2) = two_held("reserve_auth.sock").await;

        let mut auth = UnixStream::connect(&sock_path).await.expect("Connect auth");
        let buf = exchange(&mut auth, &auth_request(current_uid())).await;
        let resp = auth_response(&buf);
        assert_eq!(
            resp.verdict,
            Verdict::Unavailable,
            "no pipeline: fail closed"
        );
        assert!(
            closed_promptly(&mut auth).await,
            "the reserve serves one Auth, then closes"
        );
        permits_back(&dispatcher, 6).await;

        // While one reserve is held (idle), a second concurrent reserve is refused.
        let held_reserve = UnixStream::connect(&sock_path)
            .await
            .expect("Connect reserve");
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut second = UnixStream::connect(&sock_path)
            .await
            .expect("Connect second reserve");
        send_ignoring_errors(&mut second, &auth_request(current_uid())).await;
        assert!(
            closed_promptly(&mut second).await,
            "only one auth-reserved connection per UID"
        );
        drop(held_reserve);
        permits_back(&dispatcher, 6).await;

        // The held regular connections keep working.
        status_roundtrip(&mut c1, 3).await;
        status_roundtrip(&mut c2, 4).await;
    }

    #[tokio::test]
    async fn test_rsh_reserve_closes_non_auth_without_payload() {
        if current_uid() == 0 {
            return;
        }
        let (_dir, sock_path, dispatcher, _c1, _c2) = two_held("reserve_non_auth.sock").await;
        let uid = current_uid();
        let preview = Request {
            kind: RequestKind::PreviewFrame,
            service: "soos-gui".to_string(),
            ..status_request(uid, 0x31)
        };
        let event = Event {
            version: CURRENT_VERSION,
            kind: EventKind::PasswordFailed,
            request_id: Some([0x32; 32]),
            uid: Some(uid),
            service: "swaylock".into(),
            timestamp_monotonic_ns: 1,
        };
        let frames: Vec<(&str, Vec<u8>)> = vec![
            (
                "Status",
                encode(&status_request(uid, 0x30)).expect("encode"),
            ),
            ("PreviewFrame", encode(&preview).expect("encode")),
            ("PasswordFailed event", encode(&event).expect("encode")),
        ];
        for (name, framed) in frames {
            let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
            let _ = client.write_all(&framed).await;
            let _ = client.flush().await;
            assert!(
                closed_promptly(&mut client).await,
                "{name} on the auth-reserved connection is closed without an answer"
            );
            permits_back(&dispatcher, 6).await;
        }
        // The reserve is free again: an Auth is still admitted on it.
        let mut auth = UnixStream::connect(&sock_path).await.expect("Connect auth");
        let buf = exchange(&mut auth, &auth_request(uid)).await;
        auth_response(&buf);
    }

    #[tokio::test]
    async fn test_rsh_reserve_idle_connection_closed_within_window() {
        if current_uid() == 0 {
            return;
        }
        let (_dir, sock_path, dispatcher, _c1, _c2) = two_held("reserve_idle.sock").await;
        let started = Instant::now();
        let mut idle = UnixStream::connect(&sock_path).await.expect("Connect idle");
        assert!(
            closed_promptly(&mut idle).await,
            "a silent auth-reserved connection is closed within the window"
        );
        assert!(started.elapsed() < CLOSE_DEADLINE);
        permits_back(&dispatcher, 6).await;
        // The reserve is reusable afterwards.
        let mut auth = UnixStream::connect(&sock_path).await.expect("Connect auth");
        let buf = exchange(&mut auth, &auth_request(current_uid())).await;
        auth_response(&buf);
    }

    #[tokio::test]
    async fn test_rsh_reserve_partial_frame_closed_within_window() {
        if current_uid() == 0 {
            return;
        }
        let (_dir, sock_path, dispatcher, _c1, _c2) = two_held("reserve_partial.sock").await;
        let framed = encode(&auth_request(current_uid())).expect("encode");
        let body_len = framed.len() - 4;
        let partials: Vec<(&str, Vec<u8>)> = vec![
            ("one byte", framed[..1].to_vec()),
            (
                "length prefix and half the body",
                framed[..4 + body_len / 2].to_vec(),
            ),
        ];
        for (name, bytes) in partials {
            let started = Instant::now();
            let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
            client.write_all(&bytes).await.expect("partial write");
            client.flush().await.expect("flush");
            let mut byte = [0u8; 1];
            let remaining = CLOSE_DEADLINE.saturating_sub(started.elapsed());
            match tokio::time::timeout(remaining, client.read(&mut byte)).await {
                Ok(Ok(0)) => {}
                Ok(Ok(n)) => panic!("{name}: no byte may be answered, got {n}"),
                Ok(Err(e)) => assert!(
                    matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                    ),
                    "{name}: {e}"
                ),
                Err(_) => panic!("{name}: a stalled first frame must be closed within 500 ms"),
            }
            assert!(started.elapsed() < CLOSE_DEADLINE, "{name}");
            permits_back(&dispatcher, 6).await;
        }
    }

    #[tokio::test]
    async fn test_rsh_reserve_auth_after_100ms_is_answered() {
        if current_uid() == 0 {
            return;
        }
        let (_dir, sock_path, _dispatcher, _c1, _c2) = two_held("reserve_delay.sock").await;
        let mut auth = UnixStream::connect(&sock_path).await.expect("Connect auth");
        // PAM shows "Looking for face" through the conversation between connect and write.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let buf = exchange(&mut auth, &auth_request(current_uid())).await;
        let resp = auth_response(&buf);
        assert_ne!(resp.reason_class, ReasonClass::RateLimited);
    }

    #[tokio::test]
    async fn test_rsh_reserve_auth_is_one_shot() {
        if current_uid() == 0 {
            return;
        }
        let (_dir, sock_path, dispatcher, _c1, _c2) = two_held("reserve_one_shot.sock").await;
        let mut auth = UnixStream::connect(&sock_path).await.expect("Connect auth");
        let buf = exchange(&mut auth, &auth_request(current_uid())).await;
        auth_response(&buf);
        send_ignoring_errors(&mut auth, &auth_request(current_uid())).await;
        assert!(
            closed_promptly(&mut auth).await,
            "a second Auth on the reserve is never answered"
        );
        permits_back(&dispatcher, 6).await;
    }
}

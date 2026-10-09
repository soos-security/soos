//! Contractual tests for `RequestKind::PreviewFrame` authorization (GitHub #143, CAM-01 / DMN-02).
//!
//! Contract:
//! - By default only a root peer (UID 0) may obtain preview frames.
//! - Unprivileged peers need `[preview] enabled = true` AND membership in `allowed_uids`,
//!   AND `peer_uid == uid_hint`, AND an active logind session when session enforcement is on.
//! - Denied peers receive a standard `Response` (`ProtocolError`) carrying zero frame bytes.
//! - Preview requests are rate limited per peer UID (`ProtocolError` / `RateLimited`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat};
use soos_daemon::config::{DaemonConfig, DispatcherConfig};
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::PipelineComponents;
use soos_daemon::preview::{
    authorize_preview, PreviewConfig, PreviewDenied, DEFAULT_PREVIEW_MAX_REQUESTS_PER_SEC,
    MAX_PREVIEW_ALLOWED_UIDS, PREVIEW_RATE_MAX_TRACKED_UIDS, PREVIEW_RATE_WINDOW_NS,
};
use soos_daemon::session::SessionValidator;
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, decode_preview, encode};
use soos_protocol::types::{
    PreviewResponse, ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION,
    MAX_MESSAGE_SIZE,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dispatcher_config() -> DispatcherConfig {
    DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(1000),
        enforce_active_session: false,
        logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
    }
}

fn preview_request(uid_hint: u32, nonce: u8) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::PreviewFrame,
        request_id: [nonce; 32],
        uid_hint,
        service: "soos-gui".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

fn allow_current_uid(current_uid: u32) -> PreviewConfig {
    PreviewConfig {
        enabled: true,
        allowed_uids: vec![current_uid],
        ..PreviewConfig::default()
    }
}

/// Raw framed reply bytes (4-byte prefix + payload) read from the daemon.
async fn exchange(client: &mut UnixStream, req: &Request) -> Vec<u8> {
    let framed = encode(req).expect("Encoding failed");
    client.write_all(&framed).await.expect("Write request");
    client.flush().await.expect("Flush client");

    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.expect("Read len");
    let resp_len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + resp_len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("Read body");
    buf
}

/// Upper bound of an encoded `Response` frame: length prefix, version, 32-byte nonce, verdict,
/// reason class and two `u64` varints (at most 10 bytes each).
const MAX_DENIAL_REPLY_LEN: usize = 4 + 1 + 32 + 1 + 1 + 10 + 10;

/// Asserts that a reply is a denial: a standard `Response` bounded by `MAX_MESSAGE_SIZE`
/// with the expected verdict/reason, and that zero frame bytes were delivered.
fn assert_denied(buf: &[u8], request_id: [u8; 32], reason: ReasonClass) {
    assert!(
        buf.len() <= MAX_MESSAGE_SIZE + 4,
        "Denial reply must be bounded by MAX_MESSAGE_SIZE, got {} bytes",
        buf.len()
    );
    let resp: Response = decode(buf).expect("Denied preview must decode as a standard Response");
    assert_eq!(
        resp.request_id, request_id,
        "Response must echo the request nonce"
    );
    assert_eq!(resp.verdict, Verdict::ProtocolError);
    assert_eq!(resp.reason_class, reason);
    // Zero frame bytes: a `Response` has no pixel field and its encoding is at most
    // 4 (prefix) + 1 + 32 + 1 + 1 + 10 + 10 bytes, leaving no room for any frame data.
    assert!(
        buf.len() <= MAX_DENIAL_REPLY_LEN,
        "Denied peer must never receive frame bytes (reply is {} bytes)",
        buf.len()
    );
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

/// Builds a pipeline whose only relevant component is a live mock camera.
async fn mock_pipeline(temp: &std::path::Path) -> (PipelineComponents, Arc<MockCameraManager>) {
    let bio_key = BioMasterKey::generate().expect("Generate bio key");
    let bio_store =
        Arc::new(BiometricStore::new(temp.join("biometrics"), bio_key).expect("BioStore init"));
    let ev_dir = temp.join("evidence");
    std::fs::create_dir_all(&ev_dir).expect("Create ev dir");
    let ev_key = EvMasterKey::generate().expect("Generate ev key");
    let evidence_store = Arc::new(EvidenceStore::new(
        EvidenceConfig {
            enabled: false,
            base_dir: ev_dir,
            retention_days: 1,
            daily_cap_per_uid: 1,
            key_path: temp.join("evidence.key"),
        },
        ev_key,
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
    assert!(
        camera.latest_frame().is_some(),
        "Mock camera must produce a frame"
    );

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
        camera: camera.clone(),
        vision,
        biometric_store: bio_store,
        evidence_store,
        policy,
    };
    (components, camera)
}

// ---------------------------------------------------------------------------
// Compile-time bounds on the preview constants
// ---------------------------------------------------------------------------

// The rate limit must leave headroom for the ~30 fps GUI poll.
const _: () = assert!(DEFAULT_PREVIEW_MAX_REQUESTS_PER_SEC >= 30);
// Both allow-list and limiter tables are bounded well below the auth limiter capacity.
const _: () = assert!(PREVIEW_RATE_MAX_TRACKED_UIDS <= 1024);
const _: () = assert!(MAX_PREVIEW_ALLOWED_UIDS <= 1024);

// ---------------------------------------------------------------------------
// Pure authorization function (deterministic, UID independent)
// ---------------------------------------------------------------------------

#[test]
fn test_preview_config_defaults_deny_unprivileged_peers() {
    let cfg = PreviewConfig::default();
    assert!(!cfg.enabled, "Preview must be disabled by default");
    assert!(
        cfg.allowed_uids.is_empty(),
        "Preview allow-list must be empty by default"
    );
    assert_eq!(
        cfg.max_requests_per_sec,
        DEFAULT_PREVIEW_MAX_REQUESTS_PER_SEC
    );
    assert_eq!(PREVIEW_RATE_WINDOW_NS, 1_000_000_000);

    let rl = cfg.rate_limit_config();
    assert_eq!(rl.max_attempts, DEFAULT_PREVIEW_MAX_REQUESTS_PER_SEC);
    assert_eq!(rl.window_duration_ns, PREVIEW_RATE_WINDOW_NS);
    assert_eq!(rl.max_tracked_uids, PREVIEW_RATE_MAX_TRACKED_UIDS);
}

#[test]
fn test_authorize_preview_root_peer_always_allowed() {
    let cfg = PreviewConfig::default();
    assert_eq!(authorize_preview(&cfg, 0, 0), Ok(()));
    assert_eq!(
        authorize_preview(&cfg, 0, 1000),
        Ok(()),
        "Root may request preview on behalf of any uid_hint"
    );
}

#[test]
fn test_authorize_preview_default_denies_unprivileged_peer() {
    let cfg = PreviewConfig::default();
    assert_eq!(
        authorize_preview(&cfg, 1000, 1000),
        Err(PreviewDenied::Disabled { peer_uid: 1000 })
    );
}

#[test]
fn test_authorize_preview_enabled_but_not_listed_is_denied() {
    let cfg = PreviewConfig {
        enabled: true,
        allowed_uids: vec![1001],
        ..PreviewConfig::default()
    };
    assert_eq!(
        authorize_preview(&cfg, 1000, 1000),
        Err(PreviewDenied::NotAllowed { peer_uid: 1000 })
    );
}

#[test]
fn test_authorize_preview_listed_but_disabled_is_denied() {
    let cfg = PreviewConfig {
        enabled: false,
        allowed_uids: vec![1000],
        ..PreviewConfig::default()
    };
    assert_eq!(
        authorize_preview(&cfg, 1000, 1000),
        Err(PreviewDenied::Disabled { peer_uid: 1000 })
    );
}

#[test]
fn test_authorize_preview_uid_mismatch_is_denied_even_when_listed() {
    let cfg = PreviewConfig {
        enabled: true,
        allowed_uids: vec![1000, 1001],
        ..PreviewConfig::default()
    };
    assert_eq!(
        authorize_preview(&cfg, 1000, 1001),
        Err(PreviewDenied::UidMismatch {
            peer_uid: 1000,
            requested_uid: 1001
        })
    );
    assert_eq!(authorize_preview(&cfg, 1000, 1000), Ok(()));
}

#[test]
fn test_preview_config_rejects_oversized_allow_list() {
    let cfg = PreviewConfig {
        enabled: true,
        allowed_uids: (0..=u32::try_from(MAX_PREVIEW_ALLOWED_UIDS).unwrap()).collect(),
        ..PreviewConfig::default()
    };
    assert!(
        cfg.validate().is_err(),
        "Allow-list above MAX_PREVIEW_ALLOWED_UIDS must be rejected"
    );
    let ok = PreviewConfig {
        enabled: true,
        allowed_uids: (0..u32::try_from(MAX_PREVIEW_ALLOWED_UIDS).unwrap()).collect(),
        ..PreviewConfig::default()
    };
    assert!(ok.validate().is_ok());
}

// ---------------------------------------------------------------------------
// TOML configuration
// ---------------------------------------------------------------------------

#[test]
fn test_daemon_config_default_preview_is_disabled() {
    let config = DaemonConfig::default();
    assert!(!config.preview.enabled);
    assert!(config.preview.allowed_uids.is_empty());

    let parsed = DaemonConfig::from_toml_str("[pipeline]\ncamera_device = \"auto\"\n").unwrap();
    assert!(
        !parsed.preview.enabled,
        "A daemon.toml without [preview] must keep preview disabled"
    );
    assert!(parsed.preview.allowed_uids.is_empty());
}

#[test]
fn test_daemon_config_parses_preview_section() {
    let toml = r#"
[preview]
enabled = true
allowed_uids = [1000, 1001]
max_requests_per_sec = 12
"#;
    let config = DaemonConfig::from_toml_str(toml).unwrap();
    assert!(config.preview.enabled);
    assert_eq!(config.preview.allowed_uids, vec![1000, 1001]);
    assert_eq!(config.preview.max_requests_per_sec, 12);
}

#[test]
fn test_daemon_config_rejects_oversized_preview_allow_list() {
    let uids: Vec<String> = (0..=MAX_PREVIEW_ALLOWED_UIDS)
        .map(|u| u.to_string())
        .collect();
    let toml = format!(
        "[preview]\nenabled = true\nallowed_uids = [{}]\n",
        uids.join(", ")
    );
    assert!(
        DaemonConfig::from_toml_str(&toml).is_err(),
        "Configuration with more than MAX_PREVIEW_ALLOWED_UIDS entries must fail closed"
    );
}

// ---------------------------------------------------------------------------
// Dispatcher end-to-end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_preview_frame_rejected_when_disabled() {
    let current_uid = nix::unistd::getuid().as_raw();
    if current_uid == 0 {
        // Root is always authorized; the unprivileged denial path cannot be exercised as root.
        return;
    }
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("preview_disabled.sock");
    let (components, camera) = mock_pipeline(dir.path()).await;
    let health = Arc::new(HealthState::new());
    // Default dispatcher: no preview config supplied => fail closed.
    let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
        dispatcher_config(),
        health,
        components,
    ));
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    let req = preview_request(current_uid, 0x11);
    let buf = exchange(&mut client, &req).await;
    assert_denied(&buf, [0x11; 32], ReasonClass::UidMismatch);
    camera.stop();
}

#[tokio::test]
async fn test_preview_frame_uid_mismatch_rejected() {
    let current_uid = nix::unistd::getuid().as_raw();
    if current_uid == 0 {
        return;
    }
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("preview_mismatch.sock");
    let (components, camera) = mock_pipeline(dir.path()).await;
    let health = Arc::new(HealthState::new());
    let other_uid = current_uid.wrapping_add(1);
    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(dispatcher_config(), health, components)
            .with_preview_config(PreviewConfig {
                enabled: true,
                allowed_uids: vec![current_uid, other_uid],
                ..PreviewConfig::default()
            }),
    );
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    let req = preview_request(other_uid, 0x22);
    let buf = exchange(&mut client, &req).await;
    assert_denied(&buf, [0x22; 32], ReasonClass::UidMismatch);
    camera.stop();
}

#[tokio::test]
async fn test_preview_frame_rejected_when_enabled_but_uid_not_listed() {
    let current_uid = nix::unistd::getuid().as_raw();
    if current_uid == 0 {
        return;
    }
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("preview_unlisted.sock");
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(), health).with_preview_config(PreviewConfig {
            enabled: true,
            allowed_uids: vec![current_uid.wrapping_add(7)],
            ..PreviewConfig::default()
        }),
    );
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    let req = preview_request(current_uid, 0x33);
    let buf = exchange(&mut client, &req).await;
    assert_denied(&buf, [0x33; 32], ReasonClass::UidMismatch);
}

#[tokio::test]
async fn test_preview_frame_rejected_without_active_session() {
    let current_uid = nix::unistd::getuid().as_raw();
    if current_uid == 0 {
        return;
    }
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("preview_nosession.sock");
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(), health)
            .with_session_validator(SessionValidator::with_sessions_dir(sessions_dir))
            .with_preview_config(allow_current_uid(current_uid)),
    );
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    let req = preview_request(current_uid, 0x44);
    let buf = exchange(&mut client, &req).await;
    assert_denied(&buf, [0x44; 32], ReasonClass::UidMismatch);
}

#[tokio::test]
async fn test_preview_frame_allowed_for_configured_uid_serves_frames() {
    let current_uid = nix::unistd::getuid().as_raw();
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("preview_allowed.sock");
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
    std::fs::write(
        sessions_dir.join("3"),
        format!("UID={current_uid}\nACTIVE=1\nSTATE=active\nREMOTE=0\nSEAT=seat0\nCLASS=user\n"),
    )
    .expect("session file");
    let (components, camera) = mock_pipeline(dir.path()).await;
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(dispatcher_config(), health, components)
            .with_session_validator(SessionValidator::with_sessions_dir(sessions_dir))
            .with_preview_config(allow_current_uid(current_uid)),
    );
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    let req = preview_request(current_uid, 0x55);
    let buf = exchange(&mut client, &req).await;
    let preview: PreviewResponse =
        decode_preview(&buf).expect("Authorized peer must receive a PreviewResponse");
    assert_eq!(preview.version, CURRENT_VERSION);
    assert_eq!(preview.width, 320);
    assert_eq!(preview.height, 240);
    assert_eq!(preview.format, 0, "Mock camera streams RGB24");
    assert_eq!(preview.data.len(), 320 * 240 * 3);
    camera.stop();
}

#[tokio::test]
async fn test_preview_frame_rate_limited_per_peer_uid() {
    let current_uid = nix::unistd::getuid().as_raw();
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("preview_ratelimit.sock");
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(), health).with_preview_config(PreviewConfig {
            enabled: true,
            allowed_uids: vec![current_uid],
            max_requests_per_sec: 2,
            remote_view: false,
        }),
    );
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    for nonce in 1u8..=2u8 {
        let req = preview_request(current_uid, nonce);
        let buf = exchange(&mut client, &req).await;
        let preview: PreviewResponse =
            decode_preview(&buf).expect("Requests within quota must be served");
        assert_eq!(preview.version, CURRENT_VERSION);
    }
    let req = preview_request(current_uid, 0x66);
    let buf = exchange(&mut client, &req).await;
    assert_denied(&buf, [0x66; 32], ReasonClass::RateLimited);
}

/// Spy camera recording whether the dispatcher called `notify_activity()`.
struct SpyCamera {
    notified: std::sync::atomic::AtomicBool,
}

impl CameraManager for SpyCamera {
    fn latest_frame(&self) -> Option<Arc<soos_camera_v4l::Frame>> {
        None
    }
    fn is_ready(&self) -> bool {
        false
    }
    fn notify_activity(&self) {
        self.notified
            .store(true, std::sync::atomic::Ordering::Release);
    }
    fn stop(&self) {}
}

#[tokio::test]
async fn test_preview_frame_denied_peer_does_not_wake_camera() {
    let current_uid = nix::unistd::getuid().as_raw();
    if current_uid == 0 {
        return;
    }
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("preview_nowake.sock");
    let (components, camera) = mock_pipeline(dir.path()).await;
    camera.stop();
    let spy = Arc::new(SpyCamera {
        notified: std::sync::atomic::AtomicBool::new(false),
    });
    let components = PipelineComponents {
        camera: spy.clone(),
        ..components
    };
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
        dispatcher_config(),
        health,
        components,
    ));
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    let started = std::time::Instant::now();
    let req = preview_request(current_uid, 0x77);
    let buf = exchange(&mut client, &req).await;
    assert_denied(&buf, [0x77; 32], ReasonClass::UidMismatch);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "Denied request must not enter the camera wake loop"
    );
    assert!(
        !spy.notified.load(std::sync::atomic::Ordering::Acquire),
        "Denied request must never call notify_activity() on the camera"
    );
}

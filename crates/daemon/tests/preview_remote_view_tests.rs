//! Contractual tests for the daemon side of the live camera view in `soos-remote`
//! (GitHub #345, ADR 2026-10-07 "Live Camera View in `soos-remote` Through the Daemon
//! Preview Channel", spec `AI/architect_spec_remote_live_camera.md` §9, §13.1, §13.7).
//!
//! Contract:
//! - The preview wire format codes are single-sourced in `soos-protocol` (RLC11 support).
//! - A preview peer is classified from its `/proc/<pid>/cgroup` (systemd hierarchies only):
//!   the `soos-remote.service` user unit of the peer's own UID is `RemoteCompanion`, an
//!   inconsistent or unreadable cgroup refuses the preview (fail closed), anything else is
//!   `Local` (RLC4).
//! - A `RemoteCompanion` peer is refused unless `[preview] remote_view = true` (default
//!   `false`); `Local` peers are unaffected by the key (RLC1, RLC4).
//! - Every unprivileged preview peer needs a local seat session, i.e. exactly the root `Auth`
//!   predicate `check_local_seat_session_of` (`CLASS=user`, `SEAT`, `REMOTE=0`, active; an
//!   absent or malformed `REMOTE` refuses) (RLC3).
//! - One `info` line without pixel data per remote connection on its first non-empty frame
//!   (RLC5).
//! - A persistent view connection leaves the second per-UID daemon connection for `Auth`
//!   (RLC4, ADR item 1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::useless_format,
    clippy::print_stderr,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
use soos_daemon::limits::PeerLimitsConfig;
use soos_daemon::pipeline::PipelineComponents;
use soos_daemon::preview::PreviewConfig;
use soos_daemon::preview_peer::{
    classify_preview_peer_cgroup, PreviewPeerError, PreviewPeerOrigin, REMOTE_COMPANION_UNIT,
};
use soos_daemon::session::SessionValidator;
use soos_daemon::session_policy::{LogindError, LogindSource, SessionRecord};
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, decode_preview, encode};
use soos_protocol::types::{
    PreviewResponse, ReasonClass, Request, RequestKind, Response, StatusResponse, Verdict,
    CURRENT_VERSION, MAX_MESSAGE_SIZE,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

/// Fixed `info` message of the first frame served to a remote connection (spec §9.3 step 6).
const FIRST_FRAME_MESSAGE: &str =
    "Remote camera view: first preview frame served to soos-remote on this connection";

/// Upper bound of an encoded `Response` frame (see `preview_authorization_tests.rs`).
const MAX_DENIAL_REPLY_LEN: usize = 4 + 1 + 32 + 1 + 1 + 10 + 10;

/// Upper bound for "the daemon closed the connection promptly".
const CLOSE_DEADLINE: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn current_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

fn dispatcher_config() -> DispatcherConfig {
    DispatcherConfig {
        max_concurrent_connections: 8,
        connection_timeout: Duration::from_millis(2000),
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
        service: "soos-remote".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
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
        service: "swaylock".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

/// Preview enabled for `uid` with an explicit `remote_view` value.
fn preview_config(uid: u32, remote_view: bool) -> PreviewConfig {
    PreviewConfig {
        enabled: true,
        allowed_uids: vec![uid],
        remote_view,
        ..PreviewConfig::default()
    }
}

/// `/proc/<pid>/cgroup` of a process inside the `soos-remote.service` user unit of `uid`.
fn remote_cgroup(uid: u32) -> String {
    format!("0::/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/soos-remote.service\n")
}

/// `/proc/<pid>/cgroup` of a process inside a local session scope of `uid`.
fn local_cgroup(uid: u32) -> String {
    format!("0::/user.slice/user-{uid}.slice/session-4.scope\n")
}

/// Mock logind source answering only `cgroup_of_pid` (spec §9.3 test hook).
#[derive(Debug)]
struct CgroupSource {
    cgroup: Result<Option<String>, LogindError>,
}

impl CgroupSource {
    fn with(content: String) -> Arc<dyn LogindSource> {
        Arc::new(Self {
            cgroup: Ok(Some(content)),
        })
    }
}

impl LogindSource for CgroupSource {
    fn session_id_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
        Ok(None)
    }
    fn session(&self, _session_id: &str) -> Result<Option<SessionRecord>, LogindError> {
        Ok(None)
    }
    fn sessions(&self) -> Result<Vec<SessionRecord>, LogindError> {
        Ok(Vec::new())
    }
    fn cgroup_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
        self.cgroup.clone()
    }
}

async fn spawn_server(dispatcher: Arc<ConnectionDispatcher>, sock_path: &Path) {
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

/// Asserts a bounded `ProtocolError`/`UidMismatch` denial carrying zero pixel bytes.
fn assert_refused(buf: &[u8], request_id: [u8; 32], what: &str) {
    assert!(
        buf.len() <= MAX_MESSAGE_SIZE + 4,
        "{what}: a refusal must be bounded by MAX_MESSAGE_SIZE ({} bytes)",
        buf.len()
    );
    let resp: Response = decode(buf)
        .unwrap_or_else(|e| panic!("{what}: a refused preview must be a standard Response: {e:?}"));
    assert_eq!(resp.request_id, request_id, "{what}: nonce must be echoed");
    assert_eq!(resp.verdict, Verdict::ProtocolError, "{what}");
    assert_eq!(resp.reason_class, ReasonClass::UidMismatch, "{what}");
    assert!(
        buf.len() <= MAX_DENIAL_REPLY_LEN,
        "{what}: a refused peer must never receive pixel bytes ({} bytes)",
        buf.len()
    );
}

/// Asserts the reply is a served `PreviewResponse`.
fn assert_served(buf: &[u8], what: &str) -> PreviewResponse {
    let resp: Result<PreviewResponse, _> = decode_preview(buf);
    match resp {
        Ok(preview) => {
            assert_eq!(preview.version, CURRENT_VERSION, "{what}");
            preview
        }
        Err(e) => {
            let refusal: Option<Response> = decode(buf).ok();
            panic!(
                "{what}: expected a PreviewResponse, got {:?} (decode error {e:?})",
                refusal.map(|r| (r.verdict, r.reason_class))
            )
        }
    }
}

/// Returns true when the daemon closed the stream (EOF or reset) within `CLOSE_DEADLINE`.
async fn closed_promptly(client: &mut UnixStream) -> bool {
    let mut byte = [0u8; 1];
    match tokio::time::timeout(CLOSE_DEADLINE, client.read(&mut byte)).await {
        Ok(Ok(0)) => true,
        Ok(Ok(n)) => panic!("A refused connection must not receive a payload, got {n} byte(s)"),
        Ok(Err(e)) => matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
        ),
        Err(_) => false,
    }
}

/// Builds a pipeline whose only relevant component is a live mock camera (320x240 RGB24).
async fn mock_pipeline(temp: &Path) -> (PipelineComponents, Arc<MockCameraManager>) {
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

/// Writes one logind session record into `dir` under the file name `id`.
fn write_session(dir: &Path, id: &str, content: &str) {
    std::fs::create_dir_all(dir).expect("sessions dir");
    std::fs::write(dir.join(id), content).expect("session file");
}

/// A realistic local seat session record of `uid`.
fn seat_session(uid: u32) -> String {
    format!("UID={uid}\nACTIVE=1\nSTATE=active\nREMOTE=0\nSEAT=seat0\nCLASS=user\n")
}

// ---------------------------------------------------------------------------
// Tracing capture (test 45)
// ---------------------------------------------------------------------------

/// One captured event: level, `message` text and the names of every other field.
#[derive(Debug, Clone)]
struct CapturedEvent {
    level: tracing::Level,
    message: String,
    fields: Vec<String>,
}

#[derive(Default)]
struct EventRecorder(Mutex<Vec<CapturedEvent>>);

struct FieldVisitor<'a> {
    message: &'a mut String,
    fields: &'a mut Vec<String>,
}

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message.push_str(&format!("{value:?}"));
        } else {
            self.fields.push(field.name().to_string());
        }
    }
}

impl tracing::Subscriber for EventRecorder {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut message = String::new();
        let mut fields = Vec::new();
        event.record(&mut FieldVisitor {
            message: &mut message,
            fields: &mut fields,
        });
        self.0.lock().expect("recorder").push(CapturedEvent {
            level: *event.metadata().level(),
            message,
            fields,
        });
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

impl EventRecorder {
    fn first_frame_events(&self) -> Vec<CapturedEvent> {
        self.0
            .lock()
            .expect("recorder")
            .iter()
            .filter(|e| e.message.contains(FIRST_FRAME_MESSAGE))
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// 49. Preview format codes (spec §3.1, §13.1)
// ---------------------------------------------------------------------------

#[test]
fn test_rlc_preview_format_codes() {
    use soos_protocol::types::{
        PREVIEW_FORMAT_EMPTY, PREVIEW_FORMAT_GREY, PREVIEW_FORMAT_MJPEG, PREVIEW_FORMAT_NV12,
        PREVIEW_FORMAT_RGB24, PREVIEW_FORMAT_YUYV,
    };
    assert_eq!(PREVIEW_FORMAT_RGB24, 0);
    assert_eq!(PREVIEW_FORMAT_GREY, 1);
    assert_eq!(PREVIEW_FORMAT_YUYV, 2);
    assert_eq!(PREVIEW_FORMAT_NV12, 3);
    assert_eq!(PREVIEW_FORMAT_MJPEG, 4);
    assert_eq!(PREVIEW_FORMAT_EMPTY, 255);
    assert_eq!(
        soos_daemon::preview::PREVIEW_FORMAT_EMPTY,
        PREVIEW_FORMAT_EMPTY,
        "The daemon's PREVIEW_FORMAT_EMPTY must alias the protocol constant"
    );
    assert_eq!(
        soos_daemon::preview_image::PREVIEW_FORMAT_EMPTY,
        PREVIEW_FORMAT_EMPTY
    );
}

// ---------------------------------------------------------------------------
// 41. Pure cgroup classification (spec §9.2)
// ---------------------------------------------------------------------------

#[test]
fn test_rlc_classify_preview_peer_cgroup() {
    use PreviewPeerError::Malformed;
    use PreviewPeerOrigin::{Local, RemoteCompanion};

    assert_eq!(REMOTE_COMPANION_UNIT, "soos-remote.service");

    let base = "/user.slice/user-1000.slice/user@1000.service";
    let cases: Vec<(String, u32, Result<PreviewPeerOrigin, PreviewPeerError>)> = vec![
        // The companion unit of the peer's own UID.
        (
            format!("0::{base}/app.slice/soos-remote.service\n"),
            1000,
            Ok(RemoteCompanion),
        ),
        // Other user units, session scopes and the root cgroup are local.
        (
            format!("0::{base}/app.slice/soos-gui.service\n"),
            1000,
            Ok(Local),
        ),
        (format!("0::{base}/session-4.scope\n"), 1000, Ok(Local)),
        (
            "0::/user.slice/user-1000.slice/session-4.scope\n".to_string(),
            1000,
            Ok(Local),
        ),
        ("0::/\n".to_string(), 1000, Ok(Local)),
        // Exact unit name only.
        (
            format!("0::{base}/app.slice/soos-remote@x.service\n"),
            1000,
            Ok(Local),
        ),
        (
            format!("0::{base}/app.slice/soos-remote.service.d\n"),
            1000,
            Ok(Local),
        ),
        // Directly below the manager, without an `app.slice`.
        (
            format!("0::{base}/soos-remote.service\n"),
            1000,
            Ok(RemoteCompanion),
        ),
        // Inconsistent manager UID.
        (
            "0::/user.slice/user-1000.slice/user@1001.service/app.slice/soos-remote.service\n"
                .to_string(),
            1000,
            Err(Malformed),
        ),
        (
            "0::/user.slice/user-1000.slice/user@1001.service/app.slice/soos-gui.service\n"
                .to_string(),
            1000,
            Err(Malformed),
        ),
        // F11: the companion unit of another UID than the peer.
        (
            format!("0::{base}/app.slice/soos-remote.service\n"),
            1001,
            Err(Malformed),
        ),
        // cgroup v1 `name=systemd` hierarchy is considered.
        (
            format!("1:name=systemd:{base}/app.slice/soos-remote.service\n"),
            1000,
            Ok(RemoteCompanion),
        ),
        (
            format!("1:name=systemd:{base}/app.slice/soos-gui.service\n"),
            1000,
            Ok(Local),
        ),
        // Other controller hierarchies are ignored.
        (
            format!(
                "5:memory:{base}/app.slice/soos-remote.service\n0::/user.slice/user-1000.slice/session-4.scope\n"
            ),
            1000,
            Ok(Local),
        ),
        (
            format!("5:memory:{base}/app.slice/soos-remote.service\n"),
            1000,
            Ok(Local),
        ),
        (
            format!(
                "3:cpu,cpuacct:/elsewhere\n0::{base}/app.slice/soos-remote.service\n"
            ),
            1000,
            Ok(RemoteCompanion),
        ),
        // Considered lines that agree.
        (
            format!(
                "1:name=systemd:{base}/app.slice/soos-remote.service\n0::{base}/app.slice/soos-remote.service\n"
            ),
            1000,
            Ok(RemoteCompanion),
        ),
        // Considered lines that disagree.
        (
            format!(
                "1:name=systemd:/user.slice/user-1000.slice/session-4.scope\n0::{base}/app.slice/soos-remote.service\n"
            ),
            1000,
            Err(Malformed),
        ),
        // B3 / C29: the companion is the first non-`.slice` component after the manager, at
        // any slice depth (never a fixed `app.slice/` prefix) ...
        (
            format!("0::{base}/app.slice/app-soos.slice/soos-remote.service\n"),
            1000,
            Ok(RemoteCompanion),
        ),
        (
            format!("0::{base}/a.slice/b.slice/c.slice/soos-remote.service\n"),
            1000,
            Ok(RemoteCompanion),
        ),
        // ... including a sub-cgroup below the unit (never an `ends_with` match) ...
        (
            format!("0::{base}/app.slice/soos-remote.service/child\n"),
            1000,
            Ok(RemoteCompanion),
        ),
        (
            format!("0::{base}/app.slice/soos-remote.service/a/b\n"),
            1000,
            Ok(RemoteCompanion),
        ),
        // ... and only the FIRST non-`.slice` component counts: the unit name deeper below
        // another unit or scope is not the companion.
        (
            format!("0::{base}/app.slice/other.service/soos-remote.service\n"),
            1000,
            Ok(Local),
        ),
        (
            format!("0::{base}/app.slice/app-x.scope/soos-remote.service\n"),
            1000,
            Ok(Local),
        ),
        (
            "0::/user.slice/user-1000.slice/session-4.scope/soos-remote.service\n".to_string(),
            1000,
            Ok(Local),
        ),
        // Two disagreeing `0::` lines.
        (
            format!(
                "0::{base}/app.slice/soos-remote.service\n0::/user.slice/user-1000.slice/session-4.scope\n"
            ),
            1000,
            Err(Malformed),
        ),
        (
            format!(
                "0::/user.slice/user-1000.slice/session-4.scope\n0::{base}/app.slice/soos-remote.service\n"
            ),
            1000,
            Err(Malformed),
        ),
        // C29: a `..` or an empty component fails closed (never `Local`).
        (
            format!("0::{base}/app.slice/../soos-remote.service\n"),
            1000,
            Err(Malformed),
        ),
        (
            format!("0::{base}/../user@1000.service/app.slice/soos-remote.service\n"),
            1000,
            Err(Malformed),
        ),
        (
            "0::/user.slice/user-1000.slice/../session-4.scope\n".to_string(),
            1000,
            Err(Malformed),
        ),
        (
            format!("0::{base}//app.slice/soos-remote.service\n"),
            1000,
            Err(Malformed),
        ),
        (
            format!("0::{base}/app.slice//soos-remote.service\n"),
            1000,
            Err(Malformed),
        ),
        (
            format!("1:name=systemd:{base}/app.slice/../soos-remote.service\n"),
            1000,
            Err(Malformed),
        ),
        // No considered line at all.
        (String::new(), 1000, Ok(Local)),
        ("7:pids:/whatever\n".to_string(), 1000, Ok(Local)),
    ];

    for (content, peer_uid, expected) in cases {
        assert_eq!(
            classify_preview_peer_cgroup(&content, peer_uid),
            expected,
            "classification of {content:?} for peer uid {peer_uid}"
        );
    }

    // Malformed (non-strict) UIDs in the manager path.
    for bad in [
        "0::/user.slice/user-01000.slice/user@01000.service/app.slice/soos-remote.service\n",
        "0::/user.slice/user-+1000.slice/user@+1000.service/app.slice/soos-remote.service\n",
        "0::/user.slice/user-abc.slice/user@abc.service/app.slice/soos-remote.service\n",
        "0::/user.slice/user-99999999999.slice/user@99999999999.service/app.slice/soos-remote.service\n",
    ] {
        assert_eq!(
            classify_preview_peer_cgroup(bad, 1000),
            Err(Malformed),
            "malformed manager UID must fail closed: {bad:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 42. `remote_view` gate (spec §9.3 step 3)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rlc_remote_view_refused_unless_enabled() {
    let uid = current_uid();
    if uid == 0 {
        // Root peers skip the origin and seat checks; the gate cannot be exercised as root.
        return;
    }
    assert!(
        !PreviewConfig::default().remote_view,
        "remote_view must default to false"
    );

    for (remote_view, nonce) in [(false, 0x42u8), (true, 0x43u8)] {
        let dir = tempdir().expect("tempdir");
        let sock_path = dir.path().join("remote_view.sock");
        let health = Arc::new(HealthState::new());
        let dispatcher = Arc::new(
            ConnectionDispatcher::new(dispatcher_config(), health)
                .with_session_validator(SessionValidator::disabled())
                .with_preview_config(preview_config(uid, remote_view))
                .with_preview_peer_source(CgroupSource::with(remote_cgroup(uid))),
        );
        spawn_server(dispatcher, &sock_path).await;

        let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
        let buf = exchange(&mut client, &preview_request(uid, nonce)).await;
        if remote_view {
            assert_served(&buf, "remote_view = true serves the companion");
        } else {
            assert_refused(
                &buf,
                [nonce; 32],
                "remote_view = false refuses the companion",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 43. Unreadable cgroup refuses (spec S-6, §9.3 step 2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rlc_unreadable_peer_cgroup_refuses_preview() {
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    let sources: Vec<(&str, Arc<dyn LogindSource>)> = vec![
        ("absent cgroup", Arc::new(CgroupSource { cgroup: Ok(None) })),
        (
            "unreadable cgroup",
            Arc::new(CgroupSource {
                cgroup: Err(LogindError("cgroup unreadable".into())),
            }),
        ),
        (
            "malformed cgroup",
            CgroupSource::with(
                "0::/user.slice/user-1000.slice/user@1001.service/app.slice/soos-remote.service\n"
                    .to_string(),
            ),
        ),
    ];
    for (i, (what, source)) in sources.into_iter().enumerate() {
        let dir = tempdir().expect("tempdir");
        let sock_path = dir.path().join("cgroup.sock");
        let health = Arc::new(HealthState::new());
        // Even with `remote_view = true`, an unclassifiable peer is refused.
        let dispatcher = Arc::new(
            ConnectionDispatcher::new(dispatcher_config(), health)
                .with_session_validator(SessionValidator::disabled())
                .with_preview_config(preview_config(uid, true))
                .with_preview_peer_source(source),
        );
        spawn_server(dispatcher, &sock_path).await;
        let nonce = 0x50 + i as u8;
        let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
        let buf = exchange(&mut client, &preview_request(uid, nonce)).await;
        assert_refused(&buf, [nonce; 32], what);
    }
}

// ---------------------------------------------------------------------------
// 44. Local seat session (spec S-7, §9.1, §9.3 step 4, F1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rlc_preview_requires_local_seat_session() {
    const U: u32 = 1000;
    // (name, record content, expected served)
    let fixtures: Vec<(&str, String, bool)> = vec![
        (
            "manager session without seat",
            format!("UID={U}\nACTIVE=1\nSTATE=active\nREMOTE=0\nCLASS=manager\n"),
            false,
        ),
        ("local seat session", seat_session(U), true),
        (
            "local seat session, ACTIVE only",
            format!("UID={U}\nACTIVE=1\nREMOTE=0\nSEAT=seat0\nCLASS=user\n"),
            true,
        ),
        (
            "STATE=active without ACTIVE",
            format!("UID={U}\nSTATE=active\nREMOTE=0\nSEAT=seat0\nCLASS=user\n"),
            true,
        ),
        (
            "remote session",
            format!("UID={U}\nACTIVE=1\nSTATE=active\nREMOTE=1\nSEAT=seat0\nCLASS=user\n"),
            false,
        ),
        (
            "REMOTE key absent",
            format!("UID={U}\nACTIVE=1\nSTATE=active\nSEAT=seat0\nCLASS=user\n"),
            false,
        ),
        (
            "REMOTE malformed",
            format!("UID={U}\nACTIVE=1\nSTATE=active\nREMOTE=yes\nSEAT=seat0\nCLASS=user\n"),
            false,
        ),
        (
            "inactive session",
            format!("UID={U}\nACTIVE=0\nSTATE=online\nREMOTE=0\nSEAT=seat0\nCLASS=user\n"),
            false,
        ),
        (
            "empty SEAT",
            format!("UID={U}\nACTIVE=1\nSTATE=active\nREMOTE=0\nSEAT=\nCLASS=user\n"),
            false,
        ),
        (
            "greeter class",
            format!("UID={U}\nACTIVE=1\nSTATE=active\nREMOTE=0\nSEAT=seat0\nCLASS=greeter\n"),
            false,
        ),
        (
            "foreign UID",
            format!("UID=1001\nACTIVE=1\nSTATE=active\nREMOTE=0\nSEAT=seat0\nCLASS=user\n"),
            false,
        ),
    ];

    // Pure predicate: exactly the root `Auth` predicate.
    for (name, content, expected) in &fixtures {
        let record = SessionRecord::parse(content);
        assert_eq!(
            record.is_local_seat_session_of(U),
            record.check_local_seat_session_of(U).is_ok(),
            "{name}: is_local_seat_session_of must equal check_local_seat_session_of().is_ok()"
        );
        assert_eq!(record.is_local_seat_session_of(U), *expected, "{name}");
    }

    // Directory scan through the validator.
    for (name, content, expected) in &fixtures {
        let dir = tempdir().expect("tempdir");
        let sessions = dir.path().join("sessions");
        write_session(&sessions, "4", content);
        let validator = SessionValidator::with_sessions_dir(sessions);
        assert_eq!(validator.has_local_seat_session(U), *expected, "{name}");
    }
    // A seatless manager record next to a remote one never qualifies.
    {
        let dir = tempdir().expect("tempdir");
        let sessions = dir.path().join("sessions");
        write_session(&sessions, "c1", &fixtures[0].1);
        write_session(&sessions, "7", &fixtures[4].1);
        assert!(!SessionValidator::with_sessions_dir(sessions.clone()).has_local_seat_session(U));
        // Adding a real seat session makes it qualify.
        write_session(&sessions, "2", &seat_session(U));
        assert!(SessionValidator::with_sessions_dir(sessions).has_local_seat_session(U));
    }
    // Absent sessions directory fails closed; disabled enforcement permits.
    assert!(
        !SessionValidator::with_sessions_dir(PathBuf::from("/nonexistent/soos/sessions"))
            .has_local_seat_session(U)
    );
    assert!(SessionValidator::disabled().has_local_seat_session(U));

    // End to end through the dispatcher for the unprivileged test peer.
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    let end_to_end: Vec<(&str, Option<String>, bool)> = vec![
        (
            "manager session without seat",
            Some(format!(
                "UID={uid}\nACTIVE=1\nSTATE=active\nREMOTE=0\nCLASS=manager\n"
            )),
            false,
        ),
        ("local seat session", Some(seat_session(uid)), true),
        (
            "remote session",
            Some(format!(
                "UID={uid}\nACTIVE=1\nSTATE=active\nREMOTE=1\nSEAT=seat0\nCLASS=user\n"
            )),
            false,
        ),
        (
            "REMOTE key absent",
            Some(format!(
                "UID={uid}\nACTIVE=1\nSTATE=active\nSEAT=seat0\nCLASS=user\n"
            )),
            false,
        ),
        (
            "REMOTE malformed",
            Some(format!(
                "UID={uid}\nACTIVE=1\nSTATE=active\nREMOTE=yes\nSEAT=seat0\nCLASS=user\n"
            )),
            false,
        ),
        (
            "STATE=active without ACTIVE",
            Some(format!(
                "UID={uid}\nSTATE=active\nREMOTE=0\nSEAT=seat0\nCLASS=user\n"
            )),
            true,
        ),
        // `None`: enforcement disabled.
        ("disabled enforcement", None, true),
    ];
    for (i, (name, content, expected)) in end_to_end.into_iter().enumerate() {
        let dir = tempdir().expect("tempdir");
        let sock_path = dir.path().join("seat.sock");
        let validator = match content {
            Some(content) => {
                let sessions = dir.path().join("sessions");
                write_session(&sessions, "4", &content);
                SessionValidator::with_sessions_dir(sessions)
            }
            None => SessionValidator::disabled(),
        };
        let health = Arc::new(HealthState::new());
        let dispatcher = Arc::new(
            ConnectionDispatcher::new(dispatcher_config(), health)
                .with_session_validator(validator)
                .with_preview_config(preview_config(uid, false))
                .with_preview_peer_source(CgroupSource::with(local_cgroup(uid))),
        );
        spawn_server(dispatcher, &sock_path).await;
        let nonce = 0x60 + i as u8;
        let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
        let buf = exchange(&mut client, &preview_request(uid, nonce)).await;
        if expected {
            assert_served(&buf, name);
        } else {
            assert_refused(&buf, [nonce; 32], name);
        }
    }
}

// ---------------------------------------------------------------------------
// 45. One `info` line per remote connection (spec §9.3 step 6)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn test_rlc_remote_first_frame_logged_once_per_connection() {
    let uid = current_uid();
    if uid == 0 {
        // Root peers are never classified; the remote announcement cannot be exercised.
        return;
    }
    let recorder = Arc::new(EventRecorder::default());
    // Current-thread runtime: the dispatcher tasks run on this thread, under this default.
    let _guard = tracing::subscriber::set_default(Arc::clone(&recorder));

    let dir = tempdir().expect("tempdir");
    let (components, camera) = mock_pipeline(dir.path()).await;

    // Remote companion with a live camera: two non-empty frames on one connection.
    let sock_path = dir.path().join("remote_frames.sock");
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(dispatcher_config(), health, components)
            .with_session_validator(SessionValidator::disabled())
            .with_preview_config(preview_config(uid, true))
            .with_preview_peer_source(CgroupSource::with(remote_cgroup(uid))),
    );
    spawn_server(dispatcher, &sock_path).await;

    let mut first = UnixStream::connect(&sock_path).await.expect("Connect");
    for nonce in [0x71u8, 0x72u8] {
        let buf = exchange(&mut first, &preview_request(uid, nonce)).await;
        let preview = assert_served(&buf, "remote frame");
        assert!(
            !preview.data.is_empty(),
            "the mock camera must yield a non-empty frame"
        );
    }
    assert_eq!(
        recorder.first_frame_events().len(),
        1,
        "exactly one first-frame line per remote connection"
    );

    // A second connection announces again, once.
    let mut second = UnixStream::connect(&sock_path).await.expect("Connect");
    for nonce in [0x73u8, 0x74u8] {
        let buf = exchange(&mut second, &preview_request(uid, nonce)).await;
        assert_served(&buf, "remote frame on a second connection");
    }
    drop(first);
    drop(second);
    camera.stop();

    let events = recorder.first_frame_events();
    assert_eq!(
        events.len(),
        2,
        "one first-frame line per connection, two connections"
    );
    for event in &events {
        assert_eq!(event.level, tracing::Level::INFO);
        assert_eq!(
            event.fields,
            vec!["peer_uid".to_string()],
            "the first-frame line carries only peer_uid"
        );
    }
    // No event of the run names a pixel property.
    for event in recorder.0.lock().expect("recorder").iter() {
        for field in &event.fields {
            for forbidden in ["bytes", "width", "height", "sequence", "data"] {
                assert!(
                    !field.contains(forbidden),
                    "event {:?} carries a pixel field {field}",
                    event.message
                );
            }
        }
    }

    // Empty frames (no pipeline) on a remote connection: no announcement.
    let empty_path = dir.path().join("remote_empty.sock");
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(), health)
            .with_session_validator(SessionValidator::disabled())
            .with_preview_config(preview_config(uid, true))
            .with_preview_peer_source(CgroupSource::with(remote_cgroup(uid))),
    );
    spawn_server(dispatcher, &empty_path).await;
    let mut empty = UnixStream::connect(&empty_path).await.expect("Connect");
    for nonce in [0x75u8, 0x76u8] {
        let buf = exchange(&mut empty, &preview_request(uid, nonce)).await;
        let preview = assert_served(&buf, "empty remote frame");
        assert!(preview.data.is_empty());
    }
    assert_eq!(
        recorder.first_frame_events().len(),
        2,
        "empty frames are never announced"
    );

    // A local peer with real frames: no announcement.
    let local_dir = tempdir().expect("tempdir");
    let (components, camera) = mock_pipeline(local_dir.path()).await;
    let local_path = local_dir.path().join("local_frames.sock");
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(dispatcher_config(), health, components)
            .with_session_validator(SessionValidator::disabled())
            .with_preview_config(preview_config(uid, true))
            .with_preview_peer_source(CgroupSource::with(local_cgroup(uid))),
    );
    spawn_server(dispatcher, &local_path).await;
    let mut local = UnixStream::connect(&local_path).await.expect("Connect");
    for nonce in [0x77u8, 0x78u8] {
        let buf = exchange(&mut local, &preview_request(uid, nonce)).await;
        let preview = assert_served(&buf, "local frame");
        assert!(!preview.data.is_empty());
    }
    camera.stop();
    assert_eq!(
        recorder.first_frame_events().len(),
        2,
        "a local peer is never announced as soos-remote"
    );
}

// ---------------------------------------------------------------------------
// 46. `[preview] remote_view` configuration key (spec §4.2)
// ---------------------------------------------------------------------------

#[test]
fn test_rlc_remote_view_config_key() {
    assert!(!DaemonConfig::default().preview.remote_view);

    let absent = DaemonConfig::from_toml_str("[preview]\nenabled = true\nallowed_uids = [1000]\n")
        .expect("valid [preview] without remote_view");
    assert!(
        !absent.preview.remote_view,
        "remote_view must default to false"
    );

    let on = DaemonConfig::from_toml_str(
        "[preview]\nenabled = true\nallowed_uids = [1000]\nremote_view = true\n",
    )
    .expect("remote_view = true parses");
    assert!(on.preview.remote_view);

    let off = DaemonConfig::from_toml_str("[preview]\nremote_view = false\n")
        .expect("remote_view = false parses");
    assert!(!off.preview.remote_view);

    // Inert but accepted without `enabled`.
    let inert = DaemonConfig::from_toml_str("[preview]\nremote_view = true\n")
        .expect("remote_view = true with enabled = false is accepted");
    assert!(inert.preview.remote_view);
    assert!(!inert.preview.enabled);

    for bad in [
        "[preview]\nremote_view = \"yes\"\n",
        "[preview]\nremote_view = 1\n",
    ] {
        assert!(
            DaemonConfig::from_toml_str(bad).is_err(),
            "a non-boolean remote_view must be a configuration error: {bad:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 47. Local peers unaffected by `remote_view` (spec §9.3)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rlc_local_peer_unaffected_by_remote_view() {
    let uid = current_uid();
    let dir = tempdir().expect("tempdir");
    let sessions = dir.path().join("sessions");
    write_session(&sessions, "3", &seat_session(uid));
    let sock_path = dir.path().join("local_peer.sock");
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(), health)
            .with_session_validator(SessionValidator::with_sessions_dir(sessions))
            .with_preview_config(preview_config(uid, false))
            .with_preview_peer_source(CgroupSource::with(local_cgroup(uid))),
    );
    spawn_server(dispatcher, &sock_path).await;

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    for nonce in [0x81u8, 0x82u8] {
        let buf = exchange(&mut client, &preview_request(uid, nonce)).await;
        assert_served(
            &buf,
            "local peer with a seat session and remote_view = false",
        );
    }

    // A local peer under the user manager (e.g. soos-gui started as a user unit) is local too.
    let dir2 = tempdir().expect("tempdir");
    let sessions2 = dir2.path().join("sessions");
    write_session(&sessions2, "3", &seat_session(uid));
    let sock_path2 = dir2.path().join("local_unit.sock");
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(), health)
            .with_session_validator(SessionValidator::with_sessions_dir(sessions2))
            .with_preview_config(preview_config(uid, false))
            .with_preview_peer_source(CgroupSource::with(format!(
                "0::/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/soos-gui.service\n"
            ))),
    );
    spawn_server(dispatcher, &sock_path2).await;
    let mut client = UnixStream::connect(&sock_path2).await.expect("Connect");
    let buf = exchange(&mut client, &preview_request(uid, 0x83)).await;
    assert_served(&buf, "local user unit with remote_view = false");
}

// ---------------------------------------------------------------------------
// 63. A view connection leaves room for `Auth` (spec §13.7, ADR item 1, R-3)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rlc_view_connection_leaves_room_for_auth() {
    let uid = current_uid();
    if uid == 0 {
        // Root is exempt from the per-UID connection limit.
        return;
    }
    let limits = PeerLimitsConfig::default();
    assert_eq!(limits.max_connections_per_uid, 2);

    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("view_room.sock");
    let health = Arc::new(HealthState::new());
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(), health)
            .with_peer_limits(limits)
            .with_session_validator(SessionValidator::disabled())
            .with_preview_config(preview_config(uid, true))
            .with_preview_peer_source(CgroupSource::with(remote_cgroup(uid))),
    );
    spawn_server(dispatcher, &sock_path).await;

    // The persistent view connection, already served a frame.
    let mut view = UnixStream::connect(&sock_path).await.expect("Connect view");
    let buf = exchange(&mut view, &preview_request(uid, 0x91)).await;
    assert_served(&buf, "view connection");

    // A second connection of the same UID is admitted and answered.
    let mut second = UnixStream::connect(&sock_path)
        .await
        .expect("Connect second");
    let buf = exchange(&mut second, &status_request(uid, 0x92)).await;
    let status: StatusResponse = decode(&buf).expect("Status answered on the second connection");
    assert_eq!(status.version, CURRENT_VERSION);

    // A third concurrent connection is refused at admission (R-3 with soos-gui open).
    let mut third = UnixStream::connect(&sock_path)
        .await
        .expect("Connect third");
    assert!(
        closed_promptly(&mut third).await,
        "a third concurrent connection of the UID must be refused at admission"
    );

    // The view connection is still usable.
    let buf = exchange(&mut view, &preview_request(uid, 0x93)).await;
    assert_served(&buf, "view connection after the refusal");

    // An `Auth` request on the admitted second connection is processed (any verdict),
    // i.e. it was not refused at admission.
    let buf = exchange(&mut second, &auth_request(uid)).await;
    let resp: Response = decode(&buf).expect("Auth answered with a standard Response");
    assert_eq!(resp.request_id, [0x5A; 32]);

    // A fresh connection for `Auth` while the view is open is admitted too.
    let _ = closed_promptly(&mut second).await;
    drop(second);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut auth_conn = UnixStream::connect(&sock_path).await.expect("Connect auth");
    let buf = exchange(&mut auth_conn, &auth_request(uid)).await;
    let resp: Response = decode(&buf).expect("Auth admitted next to a view connection");
    assert_eq!(resp.request_id, [0x5A; 32]);
}

// ---------------------------------------------------------------------------
// B2 (auditor, C30). The production default peer source, a procfs-backed source and the
// number of cgroup reads.
// ---------------------------------------------------------------------------

/// B2 (a), RLC4: without `with_preview_peer_source`, the dispatcher classifies the peer
/// from the real `/proc/<pid>/cgroup` (`SystemLogind::with_paths(.., DEFAULT_PROC_ROOT)`),
/// for both values of `enforce_active_session`. The peer is this test process, so the
/// expected verdict is computed from `/proc/self/cgroup` with the pure classifier: a
/// trait-default source (`Ok(None)`, refuses every peer), a source built only when
/// enforcement is on, or one that skips `/proc` all disagree with it.
#[tokio::test]
async fn test_rlc_default_peer_source_reads_proc_cgroup() {
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    let own = std::fs::read_to_string("/proc/self/cgroup").expect("/proc/self/cgroup");
    let expected = classify_preview_peer_cgroup(&own, uid);
    if expected == Ok(PreviewPeerOrigin::RemoteCompanion) {
        eprintln!("skipping: the test process runs inside soos-remote.service");
        return;
    }
    for (enforce, nonce) in [(false, 0xB1u8), (true, 0xB2u8)] {
        let dir = tempdir().expect("tempdir");
        let sock_path = dir.path().join("default_source.sock");
        let health = Arc::new(HealthState::new());
        let config = DispatcherConfig {
            enforce_active_session: enforce,
            logind_sessions_dir: dir.path().join("sessions"),
            ..dispatcher_config()
        };
        // No `with_preview_peer_source`: the production default is exercised. The seat
        // check is disabled separately so that only the peer classification decides.
        let dispatcher = Arc::new(
            ConnectionDispatcher::new(config, health)
                .with_session_validator(SessionValidator::disabled())
                .with_preview_config(preview_config(uid, false)),
        );
        spawn_server(dispatcher, &sock_path).await;
        let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
        let buf = exchange(&mut client, &preview_request(uid, nonce)).await;
        match expected {
            Ok(PreviewPeerOrigin::Local) => {
                assert_served(
                    &buf,
                    &format!(
                        "default peer source (enforce_active_session = {enforce}) must read \
                         /proc and classify this Local process as Local"
                    ),
                );
            }
            _ => assert_refused(
                &buf,
                [nonce; 32],
                "an unclassifiable own cgroup must be refused by the default source",
            ),
        }
    }
}

/// B2 (a'), C30: static pin of the default constructor (the companion verdict cannot be
/// produced for the test process itself): `dispatcher.rs` builds the default preview peer
/// source as `SystemLogind::with_paths(.., DEFAULT_PROC_ROOT)`.
#[test]
fn test_rlc_default_peer_source_is_system_procfs() {
    let src =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dispatcher.rs"))
            .expect("read dispatcher.rs");
    assert!(
        src.contains("with_preview_peer_source"),
        "dispatcher.rs must define the preview peer source hook"
    );
    let compact: String = src.split_whitespace().collect();
    assert!(
        compact.contains("SystemLogind::with_paths(") && compact.contains("DEFAULT_PROC_ROOT"),
        "the default preview peer source must be SystemLogind::with_paths(.., DEFAULT_PROC_ROOT)"
    );
}

/// B2 (b), RLC4: a procfs-backed `SystemLogind` over a temporary proc root, holding a fake
/// `<pid>/cgroup` for this process: the companion path is refused with `remote_view =
/// false` and served with `true`; a cgroup file of at least `MAX_CGROUP_FILE_SIZE` bytes
/// is refused (bounded read, fail closed) even though its content would classify `Local`.
#[tokio::test]
async fn test_rlc_procfs_peer_source_classifies_companion() {
    use soos_daemon::session_policy::{SystemLogind, MAX_CGROUP_FILE_SIZE};
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    let pid = std::process::id();
    let mut oversize = local_cgroup(uid);
    while (oversize.len() as u64) < MAX_CGROUP_FILE_SIZE + 64 {
        oversize.push_str("7:pids:/padding\n");
    }
    // Sanity: the oversize content classifies Local when read whole.
    assert_eq!(
        classify_preview_peer_cgroup(&oversize, uid),
        Ok(PreviewPeerOrigin::Local)
    );
    let cases: Vec<(&str, String, bool, bool)> = vec![
        (
            "companion, remote_view = false",
            remote_cgroup(uid),
            false,
            false,
        ),
        (
            "companion, remote_view = true",
            remote_cgroup(uid),
            true,
            true,
        ),
        (
            "local scope, remote_view = false",
            local_cgroup(uid),
            false,
            true,
        ),
        ("oversize cgroup file", oversize, true, false),
    ];
    for (i, (what, content, remote_view, served)) in cases.into_iter().enumerate() {
        let dir = tempdir().expect("tempdir");
        let proc_root = dir.path().join("proc");
        std::fs::create_dir_all(proc_root.join(pid.to_string())).expect("proc dir");
        std::fs::write(proc_root.join(pid.to_string()).join("cgroup"), content)
            .expect("cgroup file");
        let source: Arc<dyn LogindSource> = Arc::new(SystemLogind::with_paths(
            dir.path().join("sessions"),
            proc_root,
        ));
        let sock_path = dir.path().join("procfs.sock");
        let health = Arc::new(HealthState::new());
        let dispatcher = Arc::new(
            ConnectionDispatcher::new(dispatcher_config(), health)
                .with_session_validator(SessionValidator::disabled())
                .with_preview_config(preview_config(uid, remote_view))
                .with_preview_peer_source(source),
        );
        spawn_server(dispatcher, &sock_path).await;
        let nonce = 0xC0 + i as u8;
        let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
        let buf = exchange(&mut client, &preview_request(uid, nonce)).await;
        if served {
            assert_served(&buf, what);
        } else {
            assert_refused(&buf, [nonce; 32], what);
        }
    }
}

/// Counting wrapper over a fixed cgroup answer.
#[derive(Debug)]
struct CountingSource {
    content: String,
    calls: std::sync::atomic::AtomicUsize,
}

impl LogindSource for CountingSource {
    fn session_id_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
        Ok(None)
    }
    fn session(&self, _session_id: &str) -> Result<Option<SessionRecord>, LogindError> {
        Ok(None)
    }
    fn sessions(&self) -> Result<Vec<SessionRecord>, LogindError> {
        Ok(Vec::new())
    }
    fn cgroup_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Some(self.content.clone()))
    }
}

/// B2 (c), C30, §11: the cgroup is read only for `PreviewFrame`, once per connection
/// (cached origin): zero reads for `Status` and `Auth`, one read for a connection serving
/// two preview frames, one more for a second preview connection.
#[tokio::test]
async fn test_rlc_peer_cgroup_read_once_per_preview_connection() {
    use std::sync::atomic::Ordering;
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    let counter = Arc::new(CountingSource {
        content: remote_cgroup(uid),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("counting.sock");
    let health = Arc::new(HealthState::new());
    let source: Arc<dyn LogindSource> = counter.clone();
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(), health)
            .with_session_validator(SessionValidator::disabled())
            .with_preview_config(preview_config(uid, true))
            .with_preview_peer_source(source),
    );
    spawn_server(dispatcher, &sock_path).await;

    // Status and Auth never touch the cgroup.
    let mut other = UnixStream::connect(&sock_path).await.expect("Connect");
    let buf = exchange(&mut other, &status_request(uid, 0xD1)).await;
    let _: StatusResponse = decode(&buf).expect("Status answered");
    let buf = exchange(&mut other, &auth_request(uid)).await;
    let _: Response = decode(&buf).expect("Auth answered");
    drop(other);
    assert_eq!(
        counter.calls.load(Ordering::SeqCst),
        0,
        "Status/Auth must never read the peer cgroup"
    );

    // One read per preview connection, cached across its frames.
    let mut view = UnixStream::connect(&sock_path).await.expect("Connect view");
    for nonce in [0xD2u8, 0xD3] {
        let buf = exchange(&mut view, &preview_request(uid, nonce)).await;
        assert_served(&buf, "preview on the counted connection");
    }
    assert_eq!(
        counter.calls.load(Ordering::SeqCst),
        1,
        "one read per connection"
    );
    drop(view);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut second = UnixStream::connect(&sock_path)
        .await
        .expect("Connect second");
    let buf = exchange(&mut second, &preview_request(uid, 0xD4)).await;
    assert_served(&buf, "second preview connection");
    assert_eq!(
        counter.calls.load(Ordering::SeqCst),
        2,
        "a new connection reads again"
    );
}

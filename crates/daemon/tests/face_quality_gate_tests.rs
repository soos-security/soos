//! Contractual daemon tests for the pre-PAD face quality gate (GitHub #218 / PAD-13).
//!
//! A face rejected by the vision quality gate (too small or blurred) is an unusable
//! capture: the request must end in `Deny` / `NoFace`, never `Allow`, and never be
//! reported as an internal error.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::PipelineComponents;
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{
    BoundingBox, FaceDetection, MockEmbeddingExtractor, MockFaceDetector, MockPadDetector,
};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

struct Fixture {
    dispatcher: Arc<ConnectionDispatcher>,
    camera: Arc<MockCameraManager>,
    detector: Arc<MockFaceDetector>,
    pad: Arc<MockPadDetector>,
    sock_path: std::path::PathBuf,
    uid: u32,
    _temp_dir: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.camera.stop();
    }
}

async fn fixture() -> Fixture {
    let temp_dir = tempdir().expect("tempdir");
    let sock_path = temp_dir.path().join("quality.sock");
    let bio_store = Arc::new(
        BiometricStore::new(
            temp_dir.path().join("biometrics"),
            BioMasterKey::generate().expect("bio key"),
        )
        .expect("bio store"),
    );
    let ev_dir = temp_dir.path().join("evidence");
    std::fs::create_dir_all(&ev_dir).expect("ev dir");
    let evidence_store = Arc::new(EvidenceStore::new(
        EvidenceConfig {
            enabled: false,
            base_dir: ev_dir,
            retention_days: 7,
            daily_cap_per_uid: 3,
            key_path: temp_dir.path().join("evidence.key"),
        },
        EvMasterKey::generate().expect("ev key"),
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
    let mut frame_opt = None;
    for _ in 0..100 {
        if let Some(f) = camera.latest_frame() {
            frame_opt = Some(f);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let frame = frame_opt.expect("mock camera frame");

    // Enrol with a large, well-sized face so that the template matches the mock output.
    let detector = Arc::new(MockFaceDetector::new_centered_face(320, 240, 0.95));
    let pad = Arc::new(MockPadDetector::new_live());
    let vision = Arc::new(VisionPipeline::new(
        detector.clone(),
        pad.clone(),
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig {
            match_threshold: 0.45,
            pad_threshold: 0.80,
            ..Default::default()
        },
    ));
    let output = vision.process_frame(&frame).expect("enrollment frame");
    let uid = nix::unistd::getuid().as_raw();
    let template = BiometricTemplate::new(
        uid,
        "mock-model".into(),
        "1.0".into(),
        1,
        zeroize::Zeroizing::new(output.embedding.as_slice().to_vec()),
    )
    .expect("template");
    bio_store.enroll(&template).expect("enroll");

    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        ThresholdConfig::default(),
        RateLimiter::new(RateLimitConfig::new(50, 60_000_000_000)),
    )));
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    health.set_camera_ready(camera.is_ready());
    health.set_models_verified(true);

    let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
        DispatcherConfig {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_secs(5),
            enforce_active_session: false,
            logind_sessions_dir: std::path::PathBuf::from("/run/systemd/sessions"),
        },
        health,
        PipelineComponents {
            camera: camera.clone(),
            vision,
            biometric_store: bio_store,
            evidence_store,
            policy,
        },
    ));

    Fixture {
        dispatcher,
        camera,
        detector,
        pad,
        sock_path,
        uid,
        _temp_dir: temp_dir,
    }
}

async fn authenticate(fixture: &Fixture, tag: u8) -> Response {
    let listener = UnixListener::bind(&fixture.sock_path).expect("bind");
    let disp = fixture.dispatcher.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = disp.handle_connection(stream).await;
        }
    });

    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [tag; 32],
        uid_hint: fixture.uid,
        service: "sudo".into(),
        deadline_monotonic_ns: u64::MAX,
    };
    let mut client = UnixStream::connect(&fixture.sock_path)
        .await
        .expect("connect");
    client
        .write_all(&encode(&req).expect("encode"))
        .await
        .expect("write");
    client.flush().await.expect("flush");
    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.expect("len");
    let resp_len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + resp_len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("body");
    decode::<Response>(&buf).expect("decode")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_218_too_small_face_returns_deny_no_face() {
    let fixture = fixture().await;
    // A 20 px face: far below DEFAULT_MIN_FACE_WIDTH_PX.
    let box_ = BoundingBox::new(150.0, 110.0, 170.0, 130.0);
    let landmarks = MockFaceDetector::canonical_landmarks_for_box(&box_);
    fixture
        .detector
        .set_detections(vec![FaceDetection::with_landmarks(box_, 0.95, landmarks)]);

    let pad_calls_before = fixture.pad.call_count();
    let resp = authenticate(&fixture, 218).await;
    assert_eq!(
        resp.verdict,
        Verdict::Deny,
        "too-small face must never allow"
    );
    assert_eq!(resp.reason_class, ReasonClass::NoFace);
    assert_eq!(
        fixture.pad.call_count(),
        pad_calls_before,
        "PAD must not score a tiny face"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_218_well_sized_face_still_allows() {
    let fixture = fixture().await;
    let resp = authenticate(&fixture, 219).await;
    assert_eq!(resp.verdict, Verdict::Allow);
    assert_eq!(resp.reason_class, ReasonClass::FaceMatch);
}

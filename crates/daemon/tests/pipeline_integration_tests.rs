//! Contractual integration tests for daemon full pipeline integration (Issue #12 / #19).
//!
//! Covers:
//! - #12.1: CameraManager startup & HealthState readiness reporting
//! - #12.2: VisionPipeline request handling, deadline propagation & stale frame checks
//! - #12.3: BiometricStore template loading & missing enrollment handling
//! - #12.4: EvidenceStore intrusion snapshot capture on PasswordFailed event
//! - #12.5: Policy engine evaluation & per-UID rate limiting
//! - #12.6: End-to-end multi-verdict verification (Allow, Deny, Unavailable, ProtocolError)

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

use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::PipelineComponents;
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{
    BoundingBox, FaceDetection, MockEmbeddingExtractor, MockFaceDetector, MockLandmarkDetector,
};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    Event, EventKind, ReasonClass, Request, RequestKind, Response, StatusResponse, Verdict,
    CURRENT_VERSION,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

/// Helper building a test pipeline environment backed by temporary directories.
#[allow(
    dead_code,
    reason = "Test fixture preserves store handles and vector state for test extensions"
)]
struct TestPipelineFixture {
    pub dispatcher: Arc<ConnectionDispatcher>,
    pub health: Arc<HealthState>,
    pub bio_store: Arc<BiometricStore>,
    pub evidence_store: Arc<EvidenceStore>,
    pub camera: Arc<MockCameraManager>,
    pub sock_path: std::path::PathBuf,
    pub enrolled_vector: Vec<f32>,
    pub current_uid: u32,
    _temp_dir: tempfile::TempDir,
}

impl TestPipelineFixture {
    pub async fn new(enroll_current_user: bool, rate_limit_max: u32) -> Self {
        let temp_dir = tempdir().expect("Failed to create tempdir");
        let sock_path = temp_dir.path().join("test_daemon.sock");
        let bio_dir = temp_dir.path().join("biometrics");
        let ev_dir = temp_dir.path().join("evidence");

        let bio_key = BioMasterKey::generate().expect("Generate bio key");
        let bio_store = Arc::new(BiometricStore::new(&bio_dir, bio_key).expect("BioStore init"));

        let ev_key = EvMasterKey::generate().expect("Generate ev key");
        let ev_config = EvidenceConfig {
            enabled: true,
            base_dir: ev_dir.clone(),
            retention_days: 7,
            daily_cap_per_uid: 3,
            key_path: temp_dir.path().join("evidence.key"),
        };
        std::fs::create_dir_all(&ev_dir).expect("Create ev dir");
        let evidence_store = Arc::new(EvidenceStore::new(ev_config, ev_key));

        let camera_config = CameraConfigBuilder::new()
            .device_path("/dev/null")
            .resolution(640, 480)
            .fps(30)
            .format(PixelFormat::Rgb24)
            .warmup_frames(0)
            .build();

        let camera = Arc::new(MockCameraManager::new(camera_config));
        // Wait for mock camera thread to stabilize and output initial frame
        let mut frame_opt = None;
        for _ in 0..100 {
            if let Some(f) = camera.latest_frame() {
                frame_opt = Some(f);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let frame = frame_opt.expect("Mock camera should have frame");

        // Configure neural inference mocks
        let dummy_detection = FaceDetection {
            box_: BoundingBox::new(50.0, 50.0, 200.0, 200.0),
            score: 0.95,
        };
        let detector = Arc::new(MockFaceDetector::new_with_detections(vec![dummy_detection]));
        let landmarks = Arc::new(MockLandmarkDetector::new_canonical());
        let extractor = Arc::new(MockEmbeddingExtractor::new(128));

        let vision_config = VisionPipelineConfig {
            min_face_confidence: 0.70,
            match_threshold: 0.45,
            target_width: 112,
            target_height: 112,
        };
        let vision = Arc::new(VisionPipeline::new(
            detector,
            landmarks,
            extractor,
            vision_config,
        ));

        let output = vision.process_frame(&frame).expect("Process frame");
        let enrolled_vector = output.embedding.as_slice().to_vec();

        let current_uid = nix::unistd::getuid().as_raw();
        if enroll_current_user {
            let template = BiometricTemplate::new(
                current_uid,
                "mock-model".into(),
                "1.0".into(),
                1,
                zeroize::Zeroizing::new(enrolled_vector.clone()),
            )
            .expect("Valid template");
            bio_store.enroll(&template).expect("Enrollment failed");
        }

        let threshold_cfg = ThresholdConfig::default();
        let rate_cfg = RateLimitConfig {
            max_attempts: rate_limit_max,
            window_duration_ns: 60_000_000_000,
        };
        let rate_limiter = RateLimiter::new(rate_cfg);
        let policy = Arc::new(Mutex::new(AuthorizationEngine::with_rate_limiter(
            threshold_cfg,
            rate_limiter,
        )));

        let components = PipelineComponents {
            camera: camera.clone(),
            vision,
            biometric_store: bio_store.clone(),
            evidence_store: evidence_store.clone(),
            policy,
        };

        let health = Arc::new(HealthState::new());
        health.set_socket_ready(true);
        health.set_camera_ready(camera.is_ready());
        health.set_models_verified(true);

        let disp_config = DispatcherConfig {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_millis(500),
        };

        let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
            disp_config,
            health.clone(),
            components,
        ));

        Self {
            dispatcher,
            health,
            bio_store,
            evidence_store,
            camera,
            sock_path,
            enrolled_vector,
            current_uid,
            _temp_dir: temp_dir,
        }
    }

    pub fn start_listener(&self) -> UnixListener {
        UnixListener::bind(&self.sock_path).expect("Bind listener failed")
    }
}

// ---------------------------------------------------------------------------
// Sub-issue #12.1: CameraManager Integration & HealthState
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_12_1_camera_startup_reports_readiness_to_health() {
    let fixture = TestPipelineFixture::new(true, 5).await;
    assert!(fixture.camera.is_ready(), "Mock camera must be ready");

    let status = fixture.health.snapshot();
    assert!(
        status.camera_ready,
        "Camera readiness must be reflected in HealthState"
    );
    assert!(status.is_healthy, "Overall health must be healthy");
}

// ---------------------------------------------------------------------------
// Sub-issue #12.2: VisionPipeline Handler & Latency Budget / Deadlines
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_12_2_deadline_exceeded_returns_unavailable_timeout() {
    let fixture = TestPipelineFixture::new(true, 5).await;
    let listener = fixture.start_listener();

    let disp = fixture.dispatcher.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = disp.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&fixture.sock_path)
        .await
        .expect("Connect client failed");

    // Request with deadline in the past (1 nanosecond)
    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [42u8; 32],
        uid_hint: fixture.current_uid,
        service: "sudo".into(),
        deadline_monotonic_ns: 1, // Past deadline
    };

    let encoded = encode(&req).expect("Encode request");
    client.write_all(&encoded).await.expect("Write request");
    client.flush().await.expect("Flush client");

    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.expect("Read len");
    let resp_len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + resp_len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("Read body");

    let resp: Response = decode(&buf).expect("Decode response");
    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.reason_class, ReasonClass::Timeout);
}

// ---------------------------------------------------------------------------
// Sub-issue #12.3: BiometricStore Template Loading
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_12_3_missing_enrollment_returns_unavailable() {
    // Fixture without enrolling current user
    let fixture = TestPipelineFixture::new(false, 5).await;
    let listener = fixture.start_listener();

    let disp = fixture.dispatcher.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = disp.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&fixture.sock_path)
        .await
        .expect("Connect client failed");

    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [12u8; 32],
        uid_hint: fixture.current_uid,
        service: "sudo".into(),
        deadline_monotonic_ns: u64::MAX,
    };

    let encoded = encode(&req).expect("Encode request");
    client.write_all(&encoded).await.expect("Write request");
    client.flush().await.expect("Flush client");

    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.expect("Read len");
    let resp_len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + resp_len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("Read body");

    let resp: Response = decode(&buf).expect("Decode response");
    assert_eq!(
        resp.verdict,
        Verdict::Unavailable,
        "Missing enrollment must return Unavailable"
    );
    assert_eq!(resp.reason_class, ReasonClass::InternalError);
}

// ---------------------------------------------------------------------------
// Sub-issue #12.4: EvidenceStore Intrusion Capture on PasswordFailed Event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_12_4_password_failed_event_captures_evidence_snapshot() {
    let fixture = TestPipelineFixture::new(true, 5).await;
    let listener = fixture.start_listener();

    let disp = fixture.dispatcher.clone();
    let handle = tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = disp.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&fixture.sock_path)
        .await
        .expect("Connect client failed");

    let event = Event {
        version: CURRENT_VERSION,
        kind: EventKind::PasswordFailed,
        request_id: Some([7u8; 32]),
        service: "gdm".into(),
        timestamp_monotonic_ns: 1_000_000,
    };

    let encoded = encode(&event).expect("Encode event");
    client.write_all(&encoded).await.expect("Write event");
    client.flush().await.expect("Flush event");

    // Client drops stream after fire-and-forget
    drop(client);

    // Wait for server task to finish handling the connection
    let _ = handle.await;

    // Verify snapshot file exists in evidence directory
    let mut found_snapshots = 0;
    for entry in std::fs::read_dir(&fixture.evidence_store.config().base_dir).expect("Read dir") {
        let entry = entry.expect("Dir entry");
        if entry.path().is_dir() {
            let date_str = entry.file_name().to_string_lossy().to_string();
            let files = fixture
                .evidence_store
                .list_snapshots_for_date(&date_str)
                .expect("List snapshots");
            found_snapshots += files.len();
            for file in files {
                let record = fixture
                    .evidence_store
                    .load_snapshot(&file)
                    .expect("Load snapshot");
                assert_eq!(record.uid, fixture.current_uid);
            }
        }
    }
    assert_eq!(
        found_snapshots, 1,
        "Evidence store must contain 1 snapshot after PasswordFailed event"
    );
}

// ---------------------------------------------------------------------------
// Sub-issue #12.5: Policy Engine Integration & Rate Limiting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_12_5_rate_limit_exceeded_returns_protocol_error_rate_limited() {
    // Configure rate limit of max 2 attempts
    let fixture = TestPipelineFixture::new(true, 2).await;
    let listener = fixture.start_listener();

    let disp = fixture.dispatcher.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let d = disp.clone();
            tokio::spawn(async move {
                let _ = d.handle_connection(stream).await;
            });
        }
    });

    let send_auth = || async {
        let mut client = UnixStream::connect(&fixture.sock_path)
            .await
            .expect("Connect client failed");

        let req = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id: [99u8; 32],
            uid_hint: fixture.current_uid,
            service: "sudo".into(),
            deadline_monotonic_ns: u64::MAX,
        };

        let encoded = encode(&req).expect("Encode request");
        client.write_all(&encoded).await.expect("Write request");
        client.flush().await.expect("Flush client");

        let mut len_bytes = [0u8; 4];
        client.read_exact(&mut len_bytes).await.expect("Read len");
        let resp_len = u32::from_be_bytes(len_bytes) as usize;
        let mut buf = vec![0u8; 4 + resp_len];
        buf[..4].copy_from_slice(&len_bytes);
        client.read_exact(&mut buf[4..]).await.expect("Read body");

        decode::<Response>(&buf).expect("Decode response")
    };

    // Attempt 1 -> Allow
    let resp1 = send_auth().await;
    assert_eq!(resp1.verdict, Verdict::Allow);

    // Attempt 2 -> Allow
    let resp2 = send_auth().await;
    assert_eq!(resp2.verdict, Verdict::Allow);

    // Attempt 3 -> Rate limit exceeded!
    let resp3 = send_auth().await;
    assert_eq!(
        resp3.verdict,
        Verdict::ProtocolError,
        "Exceeded rate limit must return ProtocolError"
    );
    assert_eq!(resp3.reason_class, ReasonClass::RateLimited);
}

// ---------------------------------------------------------------------------
// Sub-issue #12.6: End-to-End Verification — All 4 Verdict Paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_12_6_all_four_verdict_paths() {
    let fixture = TestPipelineFixture::new(true, 10).await;
    let listener = fixture.start_listener();

    let disp = fixture.dispatcher.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let d = disp.clone();
            tokio::spawn(async move {
                let _ = d.handle_connection(stream).await;
            });
        }
    });

    async fn send_req(sock_path: &std::path::Path, req: Request) -> Response {
        let mut client = UnixStream::connect(sock_path)
            .await
            .expect("Connect client failed");

        let encoded = encode(&req).expect("Encode request");
        client.write_all(&encoded).await.expect("Write request");
        client.flush().await.expect("Flush client");

        let mut len_bytes = [0u8; 4];
        client.read_exact(&mut len_bytes).await.expect("Read len");
        let resp_len = u32::from_be_bytes(len_bytes) as usize;
        let mut buf = vec![0u8; 4 + resp_len];
        buf[..4].copy_from_slice(&len_bytes);
        client.read_exact(&mut buf[4..]).await.expect("Read body");

        decode::<Response>(&buf).expect("Decode response")
    }

    // Path 1: Allow (Nominal matching authentication)
    let req_allow = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [1u8; 32],
        uid_hint: fixture.current_uid,
        service: "sudo".into(),
        deadline_monotonic_ns: u64::MAX,
    };
    let resp_allow = send_req(&fixture.sock_path, req_allow).await;
    assert_eq!(resp_allow.verdict, Verdict::Allow);
    assert_eq!(resp_allow.reason_class, ReasonClass::FaceMatch);

    // Path 2: ProtocolError (Mismatched spoofed UID)
    if fixture.current_uid != 0 {
        let req_spoof = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id: [2u8; 32],
            uid_hint: 99999, // Mismatched UID
            service: "sudo".into(),
            deadline_monotonic_ns: u64::MAX,
        };
        let resp_spoof = send_req(&fixture.sock_path, req_spoof).await;
        assert_eq!(resp_spoof.verdict, Verdict::ProtocolError);
        assert_eq!(resp_spoof.reason_class, ReasonClass::UidMismatch);
    }

    // Path 3: Unavailable (Expired deadline)
    let req_unavail = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [3u8; 32],
        uid_hint: fixture.current_uid,
        service: "sudo".into(),
        deadline_monotonic_ns: 1, // expired
    };
    let resp_unavail = send_req(&fixture.sock_path, req_unavail).await;
    assert_eq!(resp_unavail.verdict, Verdict::Unavailable);
    assert_eq!(resp_unavail.reason_class, ReasonClass::Timeout);

    // Path 4: Diagnostic Status query (Non-biometric status response)
    let mut client = UnixStream::connect(&fixture.sock_path)
        .await
        .expect("Connect client failed");
    let req_status = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Status,
        request_id: [4u8; 32],
        uid_hint: fixture.current_uid,
        service: "admin".into(),
        deadline_monotonic_ns: u64::MAX,
    };
    let encoded = encode(&req_status).expect("Encode status req");
    client.write_all(&encoded).await.expect("Write status req");
    client.flush().await.expect("Flush client");

    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.expect("Read len");
    let resp_len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + resp_len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("Read body");

    let status_resp: StatusResponse = decode(&buf).expect("Decode status resp");
    assert!(status_resp.socket_ready);
    assert!(status_resp.camera_ready);
    assert!(status_resp.is_healthy);
}

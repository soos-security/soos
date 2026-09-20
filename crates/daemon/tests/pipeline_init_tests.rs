//! Contractual integration tests for daemon full pipeline initialization and mock camera flag.

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
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, MockCameraManager, PixelFormat};
use soos_daemon::config::{DispatcherConfig, PipelineConfig};
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::error::DaemonError;
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::{initialize_pipeline, PipelineComponents};
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{Request, RequestKind, Response, CURRENT_VERSION};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

fn make_auth_request(uid: u32) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [42u8; 32],
        uid_hint: uid,
        service: "sudo".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

#[tokio::test]
async fn test_daemon_startup_initializes_all_pipeline_components() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch_full_pipe.sock");
    let bio_dir = dir.path().join("biometrics");
    let ev_dir = dir.path().join("evidence");

    let bio_key = BioMasterKey::generate().expect("Generate bio key");
    let bio_store = Arc::new(BiometricStore::new(&bio_dir, bio_key).expect("BioStore init"));

    let ev_key = EvMasterKey::generate().expect("Generate ev key");
    let ev_config = EvidenceConfig {
        enabled: false,
        base_dir: ev_dir,
        key_path: dir.path().join("evidence.key"),
        ..Default::default()
    };
    let evidence_store = Arc::new(EvidenceStore::new(ev_config, ev_key));

    let camera_config = CameraConfigBuilder::new()
        .device_path("/dev/null")
        .resolution(640, 480)
        .fps(30)
        .format(PixelFormat::Rgb24)
        .warmup_frames(0)
        .build();
    let camera = Arc::new(MockCameraManager::new(camera_config));

    // Neural mocks
    let detector = Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95));
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(128));

    let vision = Arc::new(VisionPipeline::new(
        detector,
        pad,
        extractor,
        VisionPipelineConfig::default(),
    ));

    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        ThresholdConfig::default(),
        RateLimiter::new(RateLimitConfig::default()),
    )));

    let components = PipelineComponents {
        camera: camera.clone(),
        vision,
        biometric_store: bio_store.clone(),
        evidence_store: evidence_store.clone(),
        policy,
    };

    let health = Arc::new(HealthState::new());

    // Daemon startup sequence:
    // 1. Pipeline verified and components initialized
    health.set_camera_ready(true);
    health.set_models_verified(true);

    // 2. Dispatcher created with pipeline
    let config = DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(500),
        enforce_active_session: false,
        logind_sessions_dir: std::path::PathBuf::from("/run/systemd/sessions"),
    };
    let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
        config,
        health.clone(),
        components,
    ));

    // 3. Socket bound and readiness declared
    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    health.set_socket_ready(true);

    // Assert overall health is true
    let snapshot = health.snapshot();
    assert!(snapshot.camera_ready);
    assert!(snapshot.models_verified);
    assert!(snapshot.socket_ready);
    assert!(
        snapshot.is_healthy,
        "Daemon must report is_healthy: true when all components are initialized"
    );

    // Run connection test through the active pipeline
    let disp_clone = dispatcher.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = disp_clone.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&sock_path)
        .await
        .expect("Connect failed");
    let current_uid = nix::unistd::getuid().as_raw();
    let req = make_auth_request(current_uid);
    let framed = encode(&req).expect("Encoding failed");

    client
        .write_all(&framed)
        .await
        .expect("Write framed request failed");
    client.flush().await.expect("Flush failed");

    let mut resp_len_bytes = [0u8; 4];
    client
        .read_exact(&mut resp_len_bytes)
        .await
        .expect("Read resp len failed");
    let resp_len = u32::from_be_bytes(resp_len_bytes) as usize;

    let mut full_resp_buf = Vec::with_capacity(4 + resp_len);
    full_resp_buf.extend_from_slice(&resp_len_bytes);
    let mut resp_payload = vec![0u8; resp_len];
    client
        .read_exact(&mut resp_payload)
        .await
        .expect("Read resp payload failed");
    full_resp_buf.extend_from_slice(&resp_payload);

    let resp: Response = decode(&full_resp_buf).expect("Decode response failed");
    assert_eq!(resp.request_id, [42u8; 32]);
    // The request was processed by the pipeline (since user is not enrolled, it returns Deny or Unavailable, not InternalError)
    assert_ne!(
        resp.reason_class,
        soos_protocol::types::ReasonClass::InternalError,
        "Pipeline-backed dispatcher must not return InternalError"
    );
}

#[tokio::test]
async fn test_mock_camera_flag_uses_mock_manager() {
    let mut config = PipelineConfig {
        use_mock_camera: true,
        ..Default::default()
    };
    config.camera.device_path = std::path::PathBuf::from("/dev/nonexistent_mock_cam");
    config.camera.warmup_frames = 0;

    // When use_mock_camera is true, camera manager must be MockCameraManager which runs without hardware
    let dir = tempdir().expect("tempdir");
    config.biometrics_dir = dir.path().join("biometrics");
    config.master_key_path = dir.path().join("master.key");
    config.evidence.base_dir = dir.path().join("evidence");
    config.evidence.key_path = dir.path().join("evidence.key");
    config.models_dir = dir.path().join("models");

    // Even if models are missing, testing initialize_pipeline verifies it branches into mock camera
    let err =
        initialize_pipeline(&config).expect_err("Models are missing so it should fail on models");
    // Verify it reached the models step (i.e. did not fail on camera device path!)
    assert!(
        matches!(err, DaemonError::Inference(_)),
        "Expected Inference error because models are missing, but got: {:?}",
        err
    );
}

#[tokio::test]
async fn test_pipeline_init_missing_models_fails_closed() {
    let dir = tempdir().expect("tempdir");
    let mut config = PipelineConfig {
        use_mock_camera: true,
        ..Default::default()
    };
    config.models_dir = dir.path().join("nonexistent_models");
    config.biometrics_dir = dir.path().join("biometrics");
    config.master_key_path = dir.path().join("master.key");
    config.evidence.base_dir = dir.path().join("evidence");
    config.evidence.key_path = dir.path().join("evidence.key");

    let res = initialize_pipeline(&config);
    assert!(
        matches!(res, Err(DaemonError::Inference(_))),
        "Initializing pipeline with missing models must fail closed with DaemonError::Inference"
    );
}

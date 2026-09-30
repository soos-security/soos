//! Contractual tests binding enrolled templates to the loaded embedding model
//! (GitHub #182 / STO-09): the daemon must refuse a template whose recorded
//! `model_id` differs from the embedding extractor it loaded, returning a
//! non-`Allow` verdict so that PAM falls back (`PAM_IGNORE`).

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
use soos_daemon::pipeline::{PipelineComponents, EMBEDDING_MODEL_ID};
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{
    MockEmbeddingExtractor, MockFaceDetector, MockPadDetector, ModelManifest,
};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

struct Fixture {
    dispatcher: Arc<ConnectionDispatcher>,
    camera: Arc<MockCameraManager>,
    sock_path: PathBuf,
    uid: u32,
    _temp: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.camera.stop();
    }
}

/// Builds a dispatcher bound to `expected_model` whose store holds a template for the
/// current UID recorded with `template_model`.
async fn fixture(template_model: &str, expected_model: Option<&str>) -> Fixture {
    let temp = tempdir().unwrap();
    let sock_path = temp.path().join("daemon.sock");
    let bio_store = Arc::new(
        BiometricStore::new(
            temp.path().join("biometrics"),
            BioMasterKey::generate().unwrap(),
        )
        .unwrap(),
    );
    let ev_dir = temp.path().join("evidence");
    std::fs::create_dir_all(&ev_dir).unwrap();
    let evidence_store = Arc::new(EvidenceStore::new(
        EvidenceConfig {
            enabled: false,
            base_dir: ev_dir,
            retention_days: 7,
            daily_cap_per_uid: 3,
            key_path: temp.path().join("evidence.key"),
        },
        EvMasterKey::generate().unwrap(),
    ));

    let camera = Arc::new(MockCameraManager::new(
        CameraConfigBuilder::new()
            .device_path("/dev/null")
            .resolution(320, 240)
            .fps(30)
            .idle_timeout(Duration::from_secs(60))
            .format(PixelFormat::Rgb24)
            .warmup_frames(0)
            .build(),
    ));
    let mut frame = None;
    for _ in 0..100 {
        if let Some(f) = camera.latest_frame() {
            frame = Some(f);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let frame = frame.expect("mock camera frame");

    let vision = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95)),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig {
            min_face_confidence: 0.70,
            match_threshold: 0.45,
            pad_threshold: 0.80,
            target_width: 112,
            target_height: 112,
            ..Default::default()
        },
    ));
    let enrolled = vision
        .process_frame(&frame)
        .unwrap()
        .embedding
        .as_slice()
        .to_vec();

    let uid = nix::unistd::getuid().as_raw();
    let template = BiometricTemplate::new(
        uid,
        template_model.into(),
        "2.0.0".into(),
        1,
        zeroize::Zeroizing::new(enrolled),
    )
    .unwrap();
    bio_store.enroll(&template).unwrap();

    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        ThresholdConfig::default(),
        RateLimiter::new(RateLimitConfig::new(10, 60_000_000_000)),
    )));
    let components =
        PipelineComponents::new(camera.clone(), vision, bio_store, evidence_store, policy);

    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    health.set_camera_ready(camera.is_ready());
    health.set_models_verified(true);

    let mut dispatcher = ConnectionDispatcher::with_pipeline(
        DispatcherConfig {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_millis(500),
            enforce_active_session: false,
            logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
        },
        health,
        components,
    );
    if let Some(model) = expected_model {
        dispatcher = dispatcher.with_expected_embedding_model(model);
    }

    Fixture {
        dispatcher: Arc::new(dispatcher),
        camera,
        sock_path,
        uid,
        _temp: temp,
    }
}

async fn auth(fx: &Fixture) -> Response {
    let listener = UnixListener::bind(&fx.sock_path).unwrap();
    let disp = fx.dispatcher.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = disp.handle_connection(stream).await;
        }
    });

    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [7u8; 32],
        uid_hint: fx.uid,
        service: "sudo".into(),
        deadline_monotonic_ns: u64::MAX,
    };
    let mut client = UnixStream::connect(&fx.sock_path).await.unwrap();
    client.write_all(&encode(&req).unwrap()).await.unwrap();
    client.flush().await.unwrap();
    let mut len = [0u8; 4];
    client.read_exact(&mut len).await.unwrap();
    let body_len = u32::from_be_bytes(len) as usize;
    let mut buf = vec![0u8; 4 + body_len];
    buf[..4].copy_from_slice(&len);
    client.read_exact(&mut buf[4..]).await.unwrap();
    decode::<Response>(&buf).unwrap()
}

#[test]
fn test_embedding_model_id_is_attested_by_manifest() {
    assert_eq!(EMBEDDING_MODEL_ID, "arcface_w600k_mbf");
    let manifest = ModelManifest::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml"),
    )
    .unwrap();
    assert!(manifest.get_model(EMBEDDING_MODEL_ID).is_some());
}

#[tokio::test]
async fn test_template_with_foreign_model_id_is_refused() {
    let fx = fixture("mobilefacenet", Some(EMBEDDING_MODEL_ID)).await;
    let resp = auth(&fx).await;
    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.reason_class, ReasonClass::ModelUnavailable);
}

#[tokio::test]
async fn test_template_with_matching_model_id_is_evaluated() {
    let fx = fixture(EMBEDDING_MODEL_ID, Some(EMBEDDING_MODEL_ID)).await;
    let resp = auth(&fx).await;
    assert_eq!(resp.verdict, Verdict::Allow);
    assert_eq!(resp.reason_class, ReasonClass::FaceMatch);
}

#[test]
fn test_production_wiring_binds_templates_to_loaded_embedding_model() {
    let main_rs = include_str!("../src/main.rs");
    assert!(
        main_rs.contains(".with_expected_embedding_model(EMBEDDING_MODEL_ID)"),
        "soos-daemon main must bind template model ids to the loaded embedding model"
    );
    let pipeline_rs = include_str!("../src/pipeline.rs");
    assert!(
        pipeline_rs.contains("get_or_load_session(EMBEDDING_MODEL_ID)"),
        "the embedding session must be loaded through EMBEDDING_MODEL_ID"
    );
}

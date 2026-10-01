//! Contractual tests of the SFace template binding (GitHub #278, owner decision 2026-10-01,
//! matrix SFC7 / SFC9).
//!
//! Once `sface_2021dec` (128-D) is the loaded embedding model, a template recorded with the
//! retired ArcFace model (`arcface_w600k_mbf`, 512-D) or a current-id template whose vector has
//! another length is refused with `Unavailable` / `ModelUnavailable` before any embedding
//! inference runs, so PAM returns `PAM_IGNORE` and the password module runs. Embeddings of two
//! models are never compared.

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
use std::sync::atomic::{AtomicUsize, Ordering};
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
use soos_daemon::pipeline::{
    classify_template, PipelineComponents, TemplateModelBinding, EMBEDDING_MODEL_ID,
};
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{
    BiometricEmbedding, EmbeddingExtractor, InferenceError, MockEmbeddingExtractor,
    MockFaceDetector, MockPadDetector, EMBEDDING_DIMENSION,
};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

/// Mock extractor of the shipped dimension that counts its inferences (spy).
struct CountingExtractor {
    inner: MockEmbeddingExtractor,
    calls: Arc<AtomicUsize>,
}

impl EmbeddingExtractor for CountingExtractor {
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner
            .extract_embedding(aligned_crop_rgb, width, height)
    }

    fn output_dimension(&self) -> Option<usize> {
        self.inner.output_dimension()
    }
}

struct Fixture {
    dispatcher: Arc<ConnectionDispatcher>,
    camera: Arc<MockCameraManager>,
    calls: Arc<AtomicUsize>,
    sock_path: PathBuf,
    uid: u32,
    _temp: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.camera.stop();
    }
}

/// What the stored template holds.
enum Enrolled {
    /// The embedding of the mock camera frame (same identity, shipped dimension).
    LiveIdentity,
    /// An arbitrary unit vector of the given length.
    Vector(usize),
}

async fn fixture(template_model: &str, template_version: &str, enrolled: Enrolled) -> Fixture {
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

    let calls = Arc::new(AtomicUsize::new(0));
    let vision = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95)),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(CountingExtractor {
            inner: MockEmbeddingExtractor::new(EMBEDDING_DIMENSION),
            calls: Arc::clone(&calls),
        }),
        VisionPipelineConfig {
            min_face_confidence: 0.70,
            match_threshold: 0.45,
            pad_threshold: 0.80,
            target_width: 112,
            target_height: 112,
            ..Default::default()
        },
    ));
    let vector = match enrolled {
        Enrolled::LiveIdentity => vision
            .process_frame(&frame)
            .unwrap()
            .embedding
            .as_slice()
            .to_vec(),
        Enrolled::Vector(len) => {
            let mut v = vec![0.0f32; len];
            v[0] = 1.0;
            v
        }
    };
    calls.store(0, Ordering::SeqCst);

    let uid = nix::unistd::getuid().as_raw();
    let template = BiometricTemplate::new(
        uid,
        template_model.into(),
        template_version.into(),
        1,
        zeroize::Zeroizing::new(vector),
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

    let dispatcher = ConnectionDispatcher::with_pipeline(
        DispatcherConfig {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_secs(5),
            enforce_active_session: false,
            logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
        },
        health,
        components,
    )
    .with_expected_embedding_model(EMBEDDING_MODEL_ID);

    Fixture {
        dispatcher: Arc::new(dispatcher),
        camera,
        calls,
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
        request_id: [9u8; 32],
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
fn test_classify_template_binds_model_id_and_dimension() {
    assert_eq!(
        classify_template(EMBEDDING_MODEL_ID, Some(128), EMBEDDING_MODEL_ID, 128),
        TemplateModelBinding::Current
    );
    assert_eq!(
        classify_template(EMBEDDING_MODEL_ID, Some(128), EMBEDDING_MODEL_ID, 512),
        TemplateModelBinding::Foreign
    );
    assert_eq!(
        classify_template(EMBEDDING_MODEL_ID, Some(128), "arcface_w600k_mbf", 128),
        TemplateModelBinding::Foreign
    );
    assert_eq!(
        classify_template(EMBEDDING_MODEL_ID, Some(128), "arcface_w600k_mbf", 512),
        TemplateModelBinding::Foreign
    );
    // An extractor that reports no dimension binds by id only.
    assert_eq!(
        classify_template(EMBEDDING_MODEL_ID, None, EMBEDDING_MODEL_ID, 512),
        TemplateModelBinding::Current
    );
}

/// SFC7: a retired ArcFace template is refused before any inference.
#[tokio::test]
async fn test_arcface_template_is_refused_after_the_sface_switch() {
    let fx = fixture("arcface_w600k_mbf", "2.0.0", Enrolled::Vector(512)).await;
    let resp = auth(&fx).await;
    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.reason_class, ReasonClass::ModelUnavailable);
    assert_eq!(
        fx.calls.load(Ordering::SeqCst),
        0,
        "no embedding inference may run for a foreign template"
    );
}

/// SFC9: a current-id template whose vector length is not the loaded dimension is refused as
/// `Unavailable` (not scored as a `Deny`), before any inference.
#[tokio::test]
async fn test_template_with_wrong_dimension_is_refused() {
    let fx = fixture(EMBEDDING_MODEL_ID, "2.0.0", Enrolled::Vector(512)).await;
    let resp = auth(&fx).await;
    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.reason_class, ReasonClass::ModelUnavailable);
    assert_eq!(fx.calls.load(Ordering::SeqCst), 0);
}

/// Control: a current SFace template of the shipped dimension is matched and authorized.
#[tokio::test]
async fn test_current_sface_template_is_evaluated() {
    let fx = fixture(EMBEDDING_MODEL_ID, "2.0.0", Enrolled::LiveIdentity).await;
    let resp = auth(&fx).await;
    assert_eq!(resp.verdict, Verdict::Allow);
    assert_eq!(resp.reason_class, ReasonClass::FaceMatch);
    assert!(fx.calls.load(Ordering::SeqCst) > 0);
}

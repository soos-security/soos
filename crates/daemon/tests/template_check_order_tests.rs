//! Contract tests of GitHub #298 (follow-ups of the SFace switch review, matrix TCO1-TCO8):
//! the enrolled template is fetched and bound to the loaded embedding model BEFORE the camera
//! is woken and before any capture, and a template that can never authenticate (foreign
//! model id or dimension) does not consume a rate-limit attempt.
//!
//! Before the fix the dispatcher reserved the attempt, woke the camera (LED on, up to the
//! wake budget for a cold sensor) and only then classified the template, so a user with a
//! pre-SFace template waited for the camera and lost one attempt for a request that was
//! certain to be refused.
//!
//! Preserved contracts (GitHub #200 / DMN-10): a CURRENT template, a missing template and a
//! store error still reserve exactly one attempt atomically under the policy write lock
//! before any camera or vision work; a rate-limited request never wakes the camera.

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
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, Frame, MockCameraManager, PixelFormat};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::{current_monotonic_nanos, PipelineComponents, EMBEDDING_MODEL_ID};
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{
    BiometricEmbedding, EmbeddingExtractor, InferenceError, MockEmbeddingExtractor,
    MockFaceDetector, MockPadDetector, EMBEDDING_DIMENSION,
};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

/// Sentinel stored by the spy when it could not observe the limiter (lock held by a writer)
/// or has not observed it yet.
const NOT_OBSERVED: usize = usize::MAX;

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

/// Spy camera wrapping the mock camera: counts wakes (`notify_activity`) and capture reads
/// (`latest_frame`) and records the limiter state seen at the first of each.
struct SpyCamera {
    inner: Arc<MockCameraManager>,
    policy: Arc<RwLock<AuthorizationEngine>>,
    uid: u32,
    /// Reports a camera that never becomes ready (cold sensor).
    never_ready: bool,
    wakes: AtomicUsize,
    captures: AtomicUsize,
    /// Attempts remaining for `uid` observed at the first wake.
    remaining_at_first_wake: AtomicUsize,
    /// Attempts remaining for `uid` observed at the first capture read.
    remaining_at_first_capture: AtomicUsize,
}

impl SpyCamera {
    fn observe_remaining(&self) -> usize {
        let Ok(engine) = self.policy.try_read() else {
            return NOT_OBSERVED;
        };
        let Some(limiter) = engine.rate_limiter() else {
            return NOT_OBSERVED;
        };
        let now = current_monotonic_nanos().expect("monotonic clock");
        limiter.remaining_attempts(self.uid, now) as usize
    }

    fn reset_counters(&self) {
        self.wakes.store(0, Ordering::SeqCst);
        self.captures.store(0, Ordering::SeqCst);
        self.remaining_at_first_wake
            .store(NOT_OBSERVED, Ordering::SeqCst);
        self.remaining_at_first_capture
            .store(NOT_OBSERVED, Ordering::SeqCst);
    }
}

impl CameraManager for SpyCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        if self.captures.fetch_add(1, Ordering::SeqCst) == 0 {
            let remaining = self.observe_remaining();
            self.remaining_at_first_capture
                .store(remaining, Ordering::SeqCst);
        }
        if self.never_ready {
            return None;
        }
        self.inner.latest_frame()
    }

    fn is_ready(&self) -> bool {
        !self.never_ready && self.inner.is_ready()
    }

    fn notify_activity(&self) {
        if self.wakes.fetch_add(1, Ordering::SeqCst) == 0 {
            let remaining = self.observe_remaining();
            self.remaining_at_first_wake
                .store(remaining, Ordering::SeqCst);
        }
        self.inner.notify_activity();
    }

    fn stop(&self) {
        self.inner.stop();
    }
}

/// What the store holds for the requesting UID.
enum Enrolled {
    /// No template at all.
    Nothing,
    /// A retired ArcFace template (`arcface_w600k_mbf`, 512-D): `Foreign`.
    RetiredArcFace,
    /// A current-id template of another vector length: `Foreign`.
    WrongDimension,
    /// The embedding of the mock camera frame under the loaded model id: `Current`.
    CurrentLiveIdentity,
}

struct Fixture {
    camera: Arc<SpyCamera>,
    policy: Arc<RwLock<AuthorizationEngine>>,
    inferences: Arc<AtomicUsize>,
    sock_path: PathBuf,
    uid: u32,
    server: tokio::task::JoinHandle<()>,
    _temp: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        self.camera.stop();
    }
}

impl Fixture {
    fn remaining_attempts(&self) -> u32 {
        let engine = self.policy.try_read().expect("policy lock free");
        let limiter = engine.rate_limiter().expect("rate limiter configured");
        limiter.remaining_attempts(self.uid, current_monotonic_nanos().expect("clock"))
    }

    fn tracked_uids(&self) -> usize {
        let engine = self.policy.try_read().expect("policy lock free");
        engine
            .rate_limiter()
            .expect("rate limiter configured")
            .tracked_uids()
    }
}

async fn fixture(enrolled: Enrolled, max_attempts: u32, never_ready: bool) -> Fixture {
    let temp = tempdir().unwrap();
    let sock_path = temp.path().join("template_check_order.sock");
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

    let mock = Arc::new(MockCameraManager::new(
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
    for _ in 0..200 {
        if let Some(f) = mock.latest_frame() {
            frame = Some(f);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let frame = frame.expect("mock camera frame");

    let inferences = Arc::new(AtomicUsize::new(0));
    let vision = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95)),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(CountingExtractor {
            inner: MockEmbeddingExtractor::new(EMBEDDING_DIMENSION),
            calls: Arc::clone(&inferences),
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

    let uid = nix::unistd::getuid().as_raw();
    let unit_vector = |len: usize| {
        let mut v = vec![0.0f32; len];
        v[0] = 1.0;
        v
    };
    let template = match enrolled {
        Enrolled::Nothing => None,
        Enrolled::RetiredArcFace => Some(("arcface_w600k_mbf", unit_vector(512))),
        Enrolled::WrongDimension => Some((EMBEDDING_MODEL_ID, unit_vector(512))),
        Enrolled::CurrentLiveIdentity => Some((
            EMBEDDING_MODEL_ID,
            vision
                .process_frame(&frame)
                .unwrap()
                .embedding
                .as_slice()
                .to_vec(),
        )),
    };
    if let Some((model_id, vector)) = template {
        let template = BiometricTemplate::new(
            uid,
            model_id.into(),
            "2.0.0".into(),
            1,
            zeroize::Zeroizing::new(vector),
        )
        .unwrap();
        bio_store.enroll(&template).unwrap();
    }
    inferences.store(0, Ordering::SeqCst);

    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        ThresholdConfig::default(),
        RateLimiter::new(RateLimitConfig::new(max_attempts, 60_000_000_000)),
    )));
    let camera = Arc::new(SpyCamera {
        inner: mock,
        policy: Arc::clone(&policy),
        uid,
        never_ready,
        wakes: AtomicUsize::new(0),
        captures: AtomicUsize::new(0),
        remaining_at_first_wake: AtomicUsize::new(NOT_OBSERVED),
        remaining_at_first_capture: AtomicUsize::new(NOT_OBSERVED),
    });
    let components = PipelineComponents::new(
        camera.clone(),
        vision,
        bio_store,
        evidence_store,
        Arc::clone(&policy),
    );

    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    health.set_camera_ready(true);
    health.set_models_verified(true);

    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(
            DispatcherConfig {
                max_concurrent_connections: 8,
                connection_timeout: Duration::from_secs(5),
                enforce_active_session: false,
                logind_sessions_dir: temp.path().to_path_buf(),
            },
            health,
            components,
        )
        .with_expected_embedding_model(EMBEDDING_MODEL_ID),
    );

    let listener = UnixListener::bind(&sock_path).unwrap();
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let disp = dispatcher.clone();
            tokio::spawn(async move {
                let _ = disp.handle_connection(stream).await;
            });
        }
    });

    camera.reset_counters();
    Fixture {
        camera,
        policy,
        inferences,
        sock_path,
        uid,
        server,
        _temp: temp,
    }
}

async fn send_auth(sock_path: PathBuf, uid: u32, tag: u8) -> Response {
    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [tag; 32],
        uid_hint: uid,
        service: "sudo".into(),
        deadline_monotonic_ns: u64::MAX,
    };
    let mut client = UnixStream::connect(&sock_path).await.unwrap();
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

async fn auth(fx: &Fixture, tag: u8) -> Response {
    send_auth(fx.sock_path.clone(), fx.uid, tag).await
}

fn verdict(resp: &Response) -> (Verdict, ReasonClass) {
    (resp.verdict, resp.reason_class)
}

const MODEL_UNAVAILABLE: (Verdict, ReasonClass) =
    (Verdict::Unavailable, ReasonClass::ModelUnavailable);
const RATE_LIMITED: (Verdict, ReasonClass) = (Verdict::ProtocolError, ReasonClass::RateLimited);

/// TCO1: a retired ArcFace template is refused without waking the camera or reading a
/// capture, and the verdict is unchanged (`Unavailable` / `ModelUnavailable`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_foreign_template_never_wakes_the_camera() {
    let fx = fixture(Enrolled::RetiredArcFace, 5, false).await;
    let resp = auth(&fx, 1).await;
    assert_eq!(verdict(&resp), MODEL_UNAVAILABLE);
    assert_eq!(
        fx.camera.wakes.load(Ordering::SeqCst),
        0,
        "a foreign template must never wake the camera"
    );
    assert_eq!(
        fx.camera.captures.load(Ordering::SeqCst),
        0,
        "a foreign template must never read a capture"
    );
    assert_eq!(fx.inferences.load(Ordering::SeqCst), 0);
}

/// TCO1: a current-id template of another vector length is refused the same way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_wrong_dimension_template_never_wakes_the_camera() {
    let fx = fixture(Enrolled::WrongDimension, 5, false).await;
    let resp = auth(&fx, 2).await;
    assert_eq!(verdict(&resp), MODEL_UNAVAILABLE);
    assert_eq!(fx.camera.wakes.load(Ordering::SeqCst), 0);
    assert_eq!(fx.camera.captures.load(Ordering::SeqCst), 0);
    assert_eq!(fx.inferences.load(Ordering::SeqCst), 0);
}

/// TCO2: with a cold camera that never becomes ready, a foreign template is answered
/// `ModelUnavailable` at once instead of waiting for the wake budget and answering
/// `CameraUnavailable`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_foreign_template_does_not_wait_for_a_cold_camera() {
    let fx = fixture(Enrolled::RetiredArcFace, 5, true).await;
    let started = Instant::now();
    let resp = auth(&fx, 3).await;
    assert_eq!(verdict(&resp), MODEL_UNAVAILABLE);
    assert_eq!(fx.camera.wakes.load(Ordering::SeqCst), 0);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "a foreign template must not wait for the camera wake budget"
    );
}

/// TCO3: a current SFace template still wakes the camera, reads captures and authenticates.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_current_template_still_wakes_the_camera_and_authenticates() {
    let fx = fixture(Enrolled::CurrentLiveIdentity, 5, false).await;
    let resp = auth(&fx, 4).await;
    assert_eq!(verdict(&resp), (Verdict::Allow, ReasonClass::FaceMatch));
    assert!(fx.camera.wakes.load(Ordering::SeqCst) >= 1);
    assert!(fx.camera.captures.load(Ordering::SeqCst) >= 1);
    assert!(fx.inferences.load(Ordering::SeqCst) > 0);
}

/// TCO4: a UID without a template is refused with the existing verdict
/// (`Unavailable` / `InternalError`) before the camera wake, and the attempt is still
/// recorded (GitHub #200 contract).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_missing_template_is_refused_before_the_camera_wake_and_still_counts() {
    let fx = fixture(Enrolled::Nothing, 5, false).await;
    let resp = auth(&fx, 5).await;
    assert_eq!(
        verdict(&resp),
        (Verdict::Unavailable, ReasonClass::InternalError)
    );
    assert_eq!(fx.camera.wakes.load(Ordering::SeqCst), 0);
    assert_eq!(fx.camera.captures.load(Ordering::SeqCst), 0);
    assert_eq!(fx.remaining_attempts(), 4, "the attempt is still recorded");
}

/// TCO5: a foreign template consumes no rate-limit attempt: with a single attempt, repeated
/// foreign requests all answer `ModelUnavailable` and the limiter tracks nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_foreign_template_consumes_no_rate_limit_attempt() {
    let fx = fixture(Enrolled::RetiredArcFace, 1, false).await;
    for tag in 10..13u8 {
        let resp = auth(&fx, tag).await;
        assert_eq!(
            verdict(&resp),
            MODEL_UNAVAILABLE,
            "request {tag}: a foreign template is never rate limited"
        );
    }
    assert_eq!(fx.tracked_uids(), 0, "no attempt was recorded");
    assert_eq!(fx.remaining_attempts(), 1);
}

/// TCO5: concurrent foreign requests leave the limiter untouched too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_foreign_template_requests_reserve_nothing() {
    let fx = fixture(Enrolled::RetiredArcFace, 1, false).await;
    let a = tokio::spawn(send_auth(fx.sock_path.clone(), fx.uid, 20));
    let b = tokio::spawn(send_auth(fx.sock_path.clone(), fx.uid, 21));
    assert_eq!(verdict(&a.await.unwrap()), MODEL_UNAVAILABLE);
    assert_eq!(verdict(&b.await.unwrap()), MODEL_UNAVAILABLE);
    assert_eq!(fx.tracked_uids(), 0);
    assert_eq!(fx.camera.wakes.load(Ordering::SeqCst), 0);
}

/// TCO6: for a current template the attempt is already reserved when the camera is woken
/// and when the first capture is read (reservation before any capture or inference).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_current_template_reserves_the_attempt_before_wake_and_capture() {
    let fx = fixture(Enrolled::CurrentLiveIdentity, 5, false).await;
    let resp = auth(&fx, 30).await;
    assert_eq!(resp.verdict, Verdict::Allow);
    assert_eq!(
        fx.camera.remaining_at_first_wake.load(Ordering::SeqCst),
        4,
        "the attempt is reserved before the camera wake"
    );
    assert_eq!(
        fx.camera.remaining_at_first_capture.load(Ordering::SeqCst),
        4,
        "the attempt is reserved before the first capture"
    );
    assert_eq!(
        fx.remaining_attempts(),
        4,
        "exactly one attempt per request"
    );
}

/// TCO7: a rate-limited request with a current template is refused without waking the
/// camera (unchanged GitHub #200 fast path).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rate_limited_current_template_never_wakes_the_camera() {
    let fx = fixture(Enrolled::CurrentLiveIdentity, 1, false).await;
    let first = auth(&fx, 40).await;
    assert_eq!(first.verdict, Verdict::Allow);
    fx.camera.reset_counters();
    let second = auth(&fx, 41).await;
    assert_eq!(verdict(&second), RATE_LIMITED);
    assert_eq!(fx.camera.wakes.load(Ordering::SeqCst), 0);
    assert_eq!(fx.camera.captures.load(Ordering::SeqCst), 0);
}

/// TCO8: concurrent requests with a current template still share the limit atomically:
/// with one attempt, exactly one request is evaluated and the other is `RateLimited`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_current_template_reservations_stay_atomic() {
    let fx = fixture(Enrolled::CurrentLiveIdentity, 1, false).await;
    let a = tokio::spawn(send_auth(fx.sock_path.clone(), fx.uid, 50));
    let b = tokio::spawn(send_auth(fx.sock_path.clone(), fx.uid, 51));
    let responses = [a.await.unwrap(), b.await.unwrap()];
    let limited = responses
        .iter()
        .filter(|r| verdict(r) == RATE_LIMITED)
        .count();
    let allowed = responses
        .iter()
        .filter(|r| r.verdict == Verdict::Allow)
        .count();
    assert_eq!(
        limited, 1,
        "the request beyond the single attempt is RateLimited"
    );
    assert!(allowed <= 1, "at most one Allow, got {allowed}");
    assert_eq!(fx.remaining_attempts(), 0);
}

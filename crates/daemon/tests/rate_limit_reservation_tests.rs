//! Contract tests for GitHub #200 (review finding DMN-10): the per-UID `Auth` rate limit
//! must be checked and recorded atomically, before any camera or vision work.
//!
//! Before the fix the dispatcher checked the limiter under a read lock at the start of the
//! request and recorded the attempt only after the consensus loop: concurrent requests all
//! passed the check, and requests that ended early (not enrolled, camera unavailable,
//! cancelled) were never counted.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contract tests use assertions, unwrap and indexing"
)]

use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::PipelineComponents;
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

/// Which face the mock detector reports on every capture.
#[derive(Clone, Copy)]
enum Scene {
    /// A single centered live face matching the enrolled template (consensus Allow).
    MatchingFace,
    /// No face at all: the consensus never completes and the request ends `Deny`.
    Empty,
}

struct Fixture {
    policy: Arc<RwLock<AuthorizationEngine>>,
    camera: Arc<MockCameraManager>,
    sock_path: std::path::PathBuf,
    uid: u32,
    _temp_dir: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.camera.stop();
    }
}

async fn fixture(scene: Scene, enroll: bool, rate_limit_max: u32) -> Fixture {
    let temp_dir = tempdir().expect("tempdir");
    let sock_path = temp_dir.path().join("rate_limit_reservation.sock");

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

    let camera = Arc::new(MockCameraManager::new(
        CameraConfigBuilder::new()
            .device_path("/dev/null")
            .resolution(320, 240)
            .fps(30)
            .idle_timeout(Duration::from_secs(60))
            .warmup_frames(0)
            .build(),
    ));
    let mut frame = None;
    for _ in 0..200 {
        frame = camera.latest_frame();
        if frame.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let frame = frame.expect("mock camera frame");

    let vision_config = VisionPipelineConfig {
        min_face_confidence: 0.70,
        match_threshold: 0.45,
        pad_threshold: 0.80,
        target_width: 112,
        target_height: 112,
        ..Default::default()
    };
    // Enrollment always uses a matching face so the template exists when requested.
    let enroll_vision = VisionPipeline::new(
        Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95)),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        vision_config.clone(),
    );
    let uid = nix::unistd::getuid().as_raw();
    if enroll {
        let output = enroll_vision
            .process_frame(&frame)
            .expect("enrollment frame");
        let template = BiometricTemplate::new(
            uid,
            "mock-model".into(),
            "1.0".into(),
            1,
            zeroize::Zeroizing::new(output.embedding.as_slice().to_vec()),
        )
        .expect("template");
        bio_store.enroll(&template).expect("enroll");
    }

    let detector = match scene {
        Scene::MatchingFace => MockFaceDetector::new_centered_face(640, 480, 0.95),
        Scene::Empty => MockFaceDetector::new_empty(),
    };
    let vision = Arc::new(VisionPipeline::new(
        Arc::new(detector),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        vision_config,
    ));

    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        ThresholdConfig::default(),
        RateLimiter::new(RateLimitConfig::new(rate_limit_max, 60_000_000_000)),
    )));

    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    health.set_camera_ready(true);
    health.set_models_verified(true);

    let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
        DispatcherConfig {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_secs(5),
            enforce_active_session: false,
            logind_sessions_dir: temp_dir.path().to_path_buf(),
        },
        health,
        PipelineComponents::new(
            camera.clone(),
            vision,
            bio_store,
            evidence_store,
            policy.clone(),
        ),
    ));

    let listener = UnixListener::bind(&sock_path).expect("bind");
    let disp = dispatcher.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let disp = disp.clone();
            tokio::spawn(async move {
                let _ = disp.handle_connection(stream).await;
            });
        }
    });

    Fixture {
        policy,
        camera,
        sock_path,
        uid,
        _temp_dir: temp_dir,
    }
}

fn auth_request(uid: u32, tag: u8) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [tag; 32],
        uid_hint: uid,
        service: "sudo".into(),
        deadline_monotonic_ns: u64::MAX,
    }
}

async fn send_auth(sock_path: std::path::PathBuf, req: Request) -> Response {
    let mut client = UnixStream::connect(&sock_path).await.expect("connect");
    client
        .write_all(&encode(&req).expect("encode"))
        .await
        .expect("write");
    client.flush().await.expect("flush");
    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.expect("read len");
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("read body");
    decode::<Response>(&buf).expect("decode response")
}

async fn send_concurrently(fixture: &Fixture, count: u8) -> Vec<Response> {
    let handles: Vec<_> = (0..count)
        .map(|i| {
            let path = fixture.sock_path.clone();
            let req = auth_request(fixture.uid, 0x40 + i);
            tokio::spawn(send_auth(path, req))
        })
        .collect();
    let mut responses = Vec::new();
    for handle in handles {
        responses.push(handle.await.expect("client task"));
    }
    responses
}

fn is_rate_limited(resp: &Response) -> bool {
    resp.verdict == Verdict::ProtocolError && resp.reason_class == ReasonClass::RateLimited
}

/// DMN-10 acceptance test: two simultaneous `Auth` requests with `max_attempts = 1`
/// must yield at most one non-`RateLimited` verdict, whatever the biometric outcome.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_200_concurrent_denied_auths_with_one_attempt_yield_one_evaluation() {
    let fixture = fixture(Scene::Empty, true, 1).await;
    let responses = send_concurrently(&fixture, 2).await;
    let evaluated = responses.iter().filter(|r| !is_rate_limited(r)).count();
    assert_eq!(
        evaluated,
        1,
        "exactly one request may consume the single attempt, got {:?}",
        responses
            .iter()
            .map(|r| (r.verdict, r.reason_class))
            .collect::<Vec<_>>()
    );
}

/// Concurrent requests on a matching face: never more than one `Allow` for one attempt.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_200_concurrent_matching_auths_with_one_attempt_allow_at_most_once() {
    let fixture = fixture(Scene::MatchingFace, true, 1).await;
    // Two is the default per-UID connection cap (`DEFAULT_MAX_CONNECTIONS_PER_UID`).
    let responses = send_concurrently(&fixture, 2).await;
    let allowed = responses
        .iter()
        .filter(|r| r.verdict == Verdict::Allow)
        .count();
    let rate_limited = responses.iter().filter(|r| is_rate_limited(r)).count();
    assert!(allowed <= 1, "at most one Allow, got {allowed}");
    assert_eq!(
        rate_limited, 1,
        "the request beyond the single attempt is RateLimited"
    );
}

/// An attempt that ends before the consensus loop (UID not enrolled) is still recorded,
/// so it cannot be replayed without limit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_200_attempt_ending_before_vision_work_is_recorded() {
    let fixture = fixture(Scene::MatchingFace, false, 1).await;
    let first = send_auth(fixture.sock_path.clone(), auth_request(fixture.uid, 1)).await;
    assert_eq!(
        (first.verdict, first.reason_class),
        (Verdict::Unavailable, ReasonClass::InternalError),
        "not enrolled"
    );
    let second = send_auth(fixture.sock_path.clone(), auth_request(fixture.uid, 2)).await;
    assert!(
        is_rate_limited(&second),
        "the early-ending attempt consumed the quota, got {:?}/{:?}",
        second.verdict,
        second.reason_class
    );
}

/// The attempt is reserved before the camera and vision work, under the policy write lock:
/// the limiter already holds it while the request is still being evaluated.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_200_attempt_is_reserved_before_vision_work() {
    let fixture = fixture(Scene::Empty, true, 5).await;
    let path = fixture.sock_path.clone();
    let req = auth_request(fixture.uid, 9);
    let pending = tokio::spawn(send_auth(path, req));

    // The empty scene keeps the consensus loop running for the whole decision budget.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !pending.is_finished(),
        "request still in the consensus loop"
    );
    let engine = fixture.policy.read().await;
    let limiter = engine.rate_limiter().expect("rate limiter configured");
    assert_eq!(
        limiter.tracked_uids(),
        1,
        "the in-flight attempt is already recorded"
    );
    drop(engine);

    let resp = pending.await.expect("client task");
    assert_ne!(resp.verdict, Verdict::Allow, "no face never authorizes");
}

/// A rate-limited request is answered without waking the camera pipeline: sequential
/// requests beyond the quota are refused immediately.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_200_allow_then_rate_limited_with_one_attempt() {
    let fixture = fixture(Scene::MatchingFace, true, 1).await;
    let first = send_auth(fixture.sock_path.clone(), auth_request(fixture.uid, 1)).await;
    assert_eq!(first.verdict, Verdict::Allow);
    let started = std::time::Instant::now();
    let second = send_auth(fixture.sock_path.clone(), auth_request(fixture.uid, 2)).await;
    assert!(is_rate_limited(&second));
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "a rate-limited request never runs the consensus loop"
    );
}

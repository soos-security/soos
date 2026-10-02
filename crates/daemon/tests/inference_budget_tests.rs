//! Contractual tests for GitHub #158 (DMN-05) and #159 (DMN-06):
//! CPU inference runs on a bounded blocking pool guarded by an inference semaphore, and the
//! multi-frame consensus loop honors a single request deadline computed once at request start,
//! never starting an inference that cannot finish within the remaining budget.
//!
//! Timing strategy: the pure deadline and estimator logic is tested with explicit `Instant`
//! arithmetic (no sleeps). Dispatcher tests use a PAD mock blocked on a condition variable that
//! only the test releases (with a generous safety timeout), so their assertions depend on
//! ordering, never on how fast the machine is.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic_in_result_fn,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::inference::{
    InferenceEstimator, InferenceGate, RequestDeadline, DEFAULT_INFERENCE_ESTIMATE_MS,
    MAX_CONCURRENT_INFERENCES, MAX_INFERENCE_ESTIMATE_MS, RESPONSE_WRITE_MARGIN_MS,
};
use soos_daemon::pipeline::{current_monotonic_nanos, PipelineComponents, DECISION_BUDGET_MS};
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{
    InferenceError, MockEmbeddingExtractor, MockFaceDetector, PadDetector, PadResult,
};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    ReasonClass, Request, RequestKind, Response, StatusResponse, Verdict, CURRENT_VERSION,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

const MS: u64 = 1_000_000;

/// Safety bound of the gated PAD mock: a test that never releases the gate still terminates.
const GATE_SAFETY_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Pure deadline arithmetic (#159)
// ---------------------------------------------------------------------------

#[test]
fn test_159_write_margin_and_estimate_constants() {
    assert_eq!(RESPONSE_WRITE_MARGIN_MS, 50);
    assert_eq!(DEFAULT_INFERENCE_ESTIMATE_MS, 80);
    assert_eq!(MAX_CONCURRENT_INFERENCES, 1);
    const { assert!(MAX_INFERENCE_ESTIMATE_MS >= DEFAULT_INFERENCE_ESTIMATE_MS) };
}

#[test]
fn test_159_client_deadline_reserves_write_margin() {
    let started = Instant::now();
    let now_ns = 10_000 * MS;
    let deadline = RequestDeadline::compute(
        now_ns,
        now_ns + 300 * MS,
        started,
        Duration::from_millis(1000),
    );
    assert_eq!(
        deadline.remaining_at(now_ns, started),
        Duration::from_millis(300 - RESPONSE_WRITE_MARGIN_MS)
    );
    assert_eq!(
        deadline.remaining_at(now_ns + 100 * MS, started),
        Duration::from_millis(200 - RESPONSE_WRITE_MARGIN_MS)
    );
}

#[test]
fn test_159_time_spent_before_the_loop_counts_against_outer_timeout() {
    // The request started 600ms ago (slow client write, peer checks, camera wake): only
    // connection_timeout - 600ms - margin remains, whatever the client deadline says.
    let started = Instant::now();
    let now_ns = 10_000 * MS;
    let deadline = RequestDeadline::compute(
        now_ns,
        now_ns + 5_000 * MS,
        started,
        Duration::from_millis(1000),
    );
    let later = started + Duration::from_millis(600);
    assert_eq!(
        deadline.remaining_at(now_ns, later),
        Duration::from_millis(400 - RESPONSE_WRITE_MARGIN_MS)
    );
    let past_outer = started + Duration::from_millis(1000);
    assert_eq!(deadline.remaining_at(now_ns, past_outer), Duration::ZERO);
    assert!(deadline.is_expired_at(now_ns, past_outer));
}

#[test]
fn test_159_missing_client_deadline_uses_decision_budget() {
    let started = Instant::now();
    let now_ns = 10_000 * MS;
    for client in [0, u64::MAX] {
        let deadline =
            RequestDeadline::compute(now_ns, client, started, Duration::from_millis(5000));
        assert_eq!(
            deadline.remaining_at(now_ns, started),
            Duration::from_millis(DECISION_BUDGET_MS),
            "client deadline {client} must fall back to DECISION_BUDGET_MS"
        );
    }
    // Still bounded by the outer connection timeout.
    let deadline = RequestDeadline::compute(now_ns, 0, started, Duration::from_millis(500));
    assert_eq!(
        deadline.remaining_at(now_ns, started),
        Duration::from_millis(500 - RESPONSE_WRITE_MARGIN_MS)
    );
}

#[test]
fn test_159_expired_or_margin_only_client_deadline_has_no_budget() {
    let started = Instant::now();
    let now_ns = 10_000 * MS;
    let expired = RequestDeadline::compute(now_ns, now_ns - MS, started, Duration::from_secs(1));
    assert_eq!(expired.remaining_at(now_ns, started), Duration::ZERO);
    let within_margin = RequestDeadline::compute(
        now_ns,
        now_ns + (RESPONSE_WRITE_MARGIN_MS - 1) * MS,
        started,
        Duration::from_secs(1),
    );
    assert_eq!(within_margin.remaining_at(now_ns, started), Duration::ZERO);
    assert!(within_margin.is_expired_at(now_ns, started));
}

#[test]
fn test_159_inference_starts_only_when_estimate_fits_remaining_budget() {
    let started = Instant::now();
    let now_ns = 10_000 * MS;
    // 150ms client budget - 50ms margin = 100ms remaining.
    let deadline =
        RequestDeadline::compute(now_ns, now_ns + 150 * MS, started, Duration::from_secs(1));
    assert!(deadline.can_start_at(now_ns, started, Duration::from_millis(80)));
    assert!(deadline.can_start_at(now_ns, started, Duration::from_millis(100)));
    assert!(!deadline.can_start_at(now_ns, started, Duration::from_millis(101)));
    assert!(!deadline.can_start_at(now_ns + 30 * MS, started, Duration::from_millis(80)));
    assert!(!deadline.can_start_at(now_ns + 200 * MS, started, Duration::ZERO));
}

#[test]
fn test_159_estimator_tracks_measured_latency_with_bounded_ema() {
    let estimator = InferenceEstimator::new(Duration::from_millis(80));
    assert_eq!(estimator.estimate(), Duration::from_millis(80));
    estimator.record(Duration::from_millis(160));
    assert_eq!(estimator.estimate(), Duration::from_millis(100));
    estimator.record(Duration::from_millis(20));
    assert_eq!(estimator.estimate(), Duration::from_millis(80));
    for _ in 0..64 {
        estimator.record(Duration::from_secs(3600));
    }
    assert_eq!(
        estimator.estimate(),
        Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS),
        "the estimate is clamped so one pathological measurement cannot disable inference forever"
    );
    let clamped_initial = InferenceEstimator::new(Duration::from_secs(3600));
    assert_eq!(
        clamped_initial.estimate(),
        Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS)
    );
}

#[test]
fn test_158_inference_gate_is_bounded_and_never_zero_sized() {
    let gate = InferenceGate::default();
    assert_eq!(gate.max_concurrent(), MAX_CONCURRENT_INFERENCES);
    assert_eq!(gate.available_permits(), MAX_CONCURRENT_INFERENCES);
    assert_eq!(
        gate.estimate(),
        Duration::from_millis(DEFAULT_INFERENCE_ESTIMATE_MS)
    );
    let clamped = InferenceGate::new(0, Duration::from_millis(10));
    assert_eq!(
        clamped.max_concurrent(),
        1,
        "a zero-sized gate would deadlock"
    );
}

// ---------------------------------------------------------------------------
// Dispatcher fixture with a gated PAD mock
// ---------------------------------------------------------------------------

/// PAD mock whose evaluations block until the test opens the gate (or the safety timeout).
#[derive(Default)]
struct GatedPad {
    armed: AtomicBool,
    panic_when_armed: AtomicBool,
    entered: AtomicUsize,
    calls: AtomicUsize,
    gate_timed_out: AtomicBool,
    open: Mutex<bool>,
    cv: Condvar,
}

impl GatedPad {
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.cv.notify_all();
    }

    fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn gate_timed_out(&self) -> bool {
        self.gate_timed_out.load(Ordering::SeqCst)
    }
}

impl PadDetector for GatedPad {
    fn evaluate_liveness(
        &self,
        _rgb: &[u8],
        _width: u32,
        _height: u32,
    ) -> Result<PadResult, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.armed.load(Ordering::SeqCst) {
            return Ok(PadResult::live(0.99));
        }
        if self.panic_when_armed.load(Ordering::SeqCst) {
            panic!("simulated inference panic");
        }
        self.entered.fetch_add(1, Ordering::SeqCst);
        let guard = self.open.lock().unwrap();
        let (_guard, wait) = self
            .cv
            .wait_timeout_while(guard, GATE_SAFETY_TIMEOUT, |open| !*open)
            .unwrap();
        if wait.timed_out() {
            self.gate_timed_out.store(true, Ordering::SeqCst);
        }
        Ok(PadResult::live(0.99))
    }
}

struct Fixture {
    dispatcher: Arc<ConnectionDispatcher>,
    policy: Arc<RwLock<AuthorizationEngine>>,
    camera: Arc<MockCameraManager>,
    pad: Arc<GatedPad>,
    sock_path: std::path::PathBuf,
    uid: u32,
    _temp_dir: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.pad.release();
        self.camera.stop();
    }
}

async fn fixture(
    connection_timeout: Duration,
    rate_limit_max: u32,
    gate: Option<InferenceGate>,
) -> Fixture {
    let temp_dir = tempdir().expect("tempdir");
    let sock_path = temp_dir.path().join("inference_budget.sock");

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

    let pad = Arc::new(GatedPad::default());
    let vision = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95)),
        pad.clone(),
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

    // Enroll with the unarmed PAD, then reset the call counter.
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
    pad.calls.store(0, Ordering::SeqCst);

    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        ThresholdConfig::default(),
        RateLimiter::new(RateLimitConfig::new(rate_limit_max, 60_000_000_000)),
    )));

    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    health.set_camera_ready(true);
    health.set_models_verified(true);

    let mut dispatcher = ConnectionDispatcher::with_pipeline(
        DispatcherConfig {
            max_concurrent_connections: 8,
            connection_timeout,
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
    );
    if let Some(gate) = gate {
        dispatcher = dispatcher.with_inference_gate(gate);
    }
    let dispatcher = Arc::new(dispatcher);

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
        dispatcher,
        policy,
        camera,
        pad,
        sock_path,
        uid,
        _temp_dir: temp_dir,
    }
}

fn request(kind: RequestKind, uid: u32, tag: u8, deadline_monotonic_ns: u64) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind,
        request_id: [tag; 32],
        uid_hint: uid,
        service: "sudo".into(),
        deadline_monotonic_ns,
    }
}

/// Sends one request and returns the raw response frame, or `None` on EOF / I/O failure.
async fn exchange(sock_path: &std::path::Path, req: &Request) -> Option<Vec<u8>> {
    let mut client = UnixStream::connect(sock_path).await.ok()?;
    client.write_all(&encode(req).ok()?).await.ok()?;
    client.flush().await.ok()?;
    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.ok()?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    assert!(len <= 4096);
    let mut buf = vec![0u8; 4 + len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.ok()?;
    Some(buf)
}

async fn auth(sock_path: &std::path::Path, req: &Request) -> Option<Response> {
    exchange(sock_path, req)
        .await
        .map(|buf| decode::<Response>(&buf).expect("decode response"))
}

async fn wait_until(mut predicate: impl FnMut() -> bool) {
    let started = Instant::now();
    while !predicate() && started.elapsed() < GATE_SAFETY_TIMEOUT * 2 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// ---------------------------------------------------------------------------
// #158: inference off the async runtime, bounded by the inference gate
// ---------------------------------------------------------------------------

/// Single-threaded runtime: if inference ran inline on the runtime thread, no Status request
/// could be served until the blocked inference finished.
#[tokio::test]
async fn test_158_status_request_served_while_inference_is_blocked() {
    let fx = fixture(Duration::from_secs(10), 10, None).await;
    fx.pad.arm();

    let sock = fx.sock_path.clone();
    let auth_req = request(RequestKind::Auth, fx.uid, 158, u64::MAX);
    let pending_auth = tokio::spawn(async move { auth(&sock, &auth_req).await });

    wait_until(|| fx.pad.entered() >= 1).await;
    assert_eq!(fx.pad.entered(), 1, "the auth request must reach inference");

    let status_req = request(RequestKind::Status, fx.uid, 1, u64::MAX);
    let status_buf = exchange(&fx.sock_path, &status_req)
        .await
        .expect("Status response must be written while inference is running");
    let status: StatusResponse = decode(&status_buf).expect("decode status");
    assert!(status.is_healthy);
    assert!(
        !fx.pad.gate_timed_out(),
        "Status was only served after the blocked inference gave up: inference blocks the runtime"
    );

    fx.pad.release();
    let resp = pending_auth
        .await
        .expect("auth task")
        .expect("auth response");
    assert_eq!(
        resp.verdict,
        Verdict::Allow,
        "reason={:?}",
        resp.reason_class
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_158_second_auth_never_queues_behind_busy_inference_gate() {
    let fx = fixture(Duration::from_secs(10), 10, None).await;
    fx.pad.arm();

    let sock = fx.sock_path.clone();
    let first_req = request(RequestKind::Auth, fx.uid, 1, u64::MAX);
    let first = tokio::spawn(async move { auth(&sock, &first_req).await });
    wait_until(|| fx.pad.entered() >= 1).await;
    assert_eq!(fx.pad.entered(), 1);

    // Second request with a 300ms client deadline while the only inference slot is busy.
    let now_ns = current_monotonic_nanos().expect("clock");
    let second_req = request(RequestKind::Auth, fx.uid, 2, now_ns + 300 * MS);
    let second = auth(&fx.sock_path, &second_req)
        .await
        .expect("second request must receive a decoded response, never EOF");

    assert_eq!(second.verdict, Verdict::Unavailable);
    assert_eq!(second.reason_class, ReasonClass::Timeout);
    assert!(
        second.verdict.should_ignore(),
        "timeout must fall back to password"
    );
    assert_eq!(
        fx.pad.entered(),
        1,
        "the second request must not run a concurrent inference"
    );
    assert!(!fx.pad.gate_timed_out());
    assert_eq!(
        fx.dispatcher.inference_gate().available_permits(),
        0,
        "the first request still owns the only inference slot"
    );

    fx.pad.release();
    let first_resp = first.await.expect("first task").expect("first response");
    assert_eq!(first_resp.verdict, Verdict::Allow);
    assert_eq!(
        fx.dispatcher.inference_gate().available_permits(),
        MAX_CONCURRENT_INFERENCES,
        "no background inference may stay queued once requests are answered"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_158_panicking_inference_returns_internal_error_not_eof() {
    let fx = fixture(Duration::from_secs(2), 10, None).await;
    fx.pad.panic_when_armed.store(true, Ordering::SeqCst);
    fx.pad.arm();

    let resp = auth(
        &fx.sock_path,
        &request(RequestKind::Auth, fx.uid, 3, u64::MAX),
    )
    .await
    .expect("a panicking inference must still produce a well-formed response");
    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.reason_class, ReasonClass::InternalError);
    assert_eq!(
        fx.dispatcher.inference_gate().available_permits(),
        MAX_CONCURRENT_INFERENCES
    );
}

// ---------------------------------------------------------------------------
// #159: never start an inference that cannot finish within the remaining budget
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_159_client_deadline_shorter_than_estimate_skips_inference() {
    let fx = fixture(Duration::from_secs(1), 10, None).await;

    // 100ms client budget - 50ms write margin = 50ms < 80ms default inference estimate.
    let now_ns = current_monotonic_nanos().expect("clock");
    let resp = auth(
        &fx.sock_path,
        &request(RequestKind::Auth, fx.uid, 4, now_ns + 100 * MS),
    )
    .await
    .expect("decoded response");

    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.reason_class, ReasonClass::Timeout);
    assert_eq!(
        fx.pad.calls(),
        0,
        "no inference may start when it cannot finish before the deadline"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_159_estimate_exceeding_budget_times_out_and_records_attempt() {
    let fx = fixture(
        Duration::from_millis(500),
        1,
        Some(InferenceGate::new(1, Duration::from_secs(10))),
    )
    .await;

    let resp = auth(
        &fx.sock_path,
        &request(RequestKind::Auth, fx.uid, 5, u64::MAX),
    )
    .await
    .expect("decoded response, never EOF");

    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.reason_class, ReasonClass::Timeout);
    assert_eq!(fx.pad.calls(), 0);

    let now_ns = current_monotonic_nanos().expect("clock");
    assert!(
        fx.policy
            .read()
            .await
            .check_allowed(fx.uid, now_ns)
            .is_err(),
        "a timed-out attempt must still be recorded by the rate limiter"
    );
}

// ---------------------------------------------------------------------------
// GitHub #315 (DMN-NEW-2, matrix DRM): a saturated estimate never disables face auth
// ---------------------------------------------------------------------------

/// PAM console/sudo client budget: the module default `timeout_ms` (1000ms).
const PAM_DEFAULT_CLIENT_BUDGET_MS: u64 = 1000;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_drm_estimate_seeded_at_max_recovers_for_1000ms_requests() {
    // Warm-up (or one pathological measurement) left the estimate at its upper bound, which
    // exceeds the 950ms budget of every 1000ms PAM request.
    let fx = fixture(
        Duration::from_millis(2500),
        20,
        Some(InferenceGate::new(
            1,
            Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS),
        )),
    )
    .await;
    assert_eq!(
        fx.dispatcher.inference_gate().estimate(),
        Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS)
    );

    let mut verdicts = Vec::new();
    for tag in 0..5u8 {
        let now_ns = current_monotonic_nanos().expect("clock");
        let resp = auth(
            &fx.sock_path,
            &request(
                RequestKind::Auth,
                fx.uid,
                100 + tag,
                now_ns + PAM_DEFAULT_CLIENT_BUDGET_MS * MS,
            ),
        )
        .await
        .expect("decoded response, never EOF");
        if verdicts.is_empty() {
            // Fail closed per request: the gated request itself never runs an inference.
            assert_eq!(resp.verdict, Verdict::Unavailable);
            assert_eq!(resp.reason_class, ReasonClass::Timeout);
            assert_eq!(
                fx.pad.calls(),
                0,
                "the gated request must not start an inference"
            );
            assert!(
                fx.dispatcher.inference_gate().estimate()
                    < Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS),
                "a request finalized by the estimate gate with zero frames evaluated must \
                 decay the estimate"
            );
        }
        verdicts.push(resp.verdict);
        if resp.verdict == Verdict::Allow {
            break;
        }
    }

    assert_eq!(
        verdicts.last(),
        Some(&Verdict::Allow),
        "a saturated estimate must not disable face authentication for 1000ms stacks \
         (verdicts: {verdicts:?})"
    );
    assert!(
        verdicts.len() <= 3,
        "recovery must take at most a couple of requests (verdicts: {verdicts:?})"
    );
    assert!(fx.pad.calls() > 0);
}

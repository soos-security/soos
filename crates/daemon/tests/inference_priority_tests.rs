//! Contract tests of GitHub #323 for PAM priority over the single inference slot
//! (matrix PAU12) and the 40-attempt default seen through the dispatcher (PAU2).
//!
//! - `InferenceGate` clones share one semaphore, estimator and interactive-demand counter;
//! - `try_acquire_background` never waits and yields to any live `InteractiveDemandGuard`;
//! - the dispatcher holds a guard for every `Auth` request in Step 8 and drops it on every
//!   return path;
//! - an `Auth` request started while a presence job holds the slot still renders `Allow`
//!   within its deadline.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use common::{build_pipeline, Enrollment, PipelineOptions, PipelineParts};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::inference::{InferenceGate, InferencePriority, InteractiveDemandGuard};
use soos_daemon::pipeline::{current_monotonic_nanos, EMBEDDING_MODEL_ID};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};

/// Sentinel: the wake hook has not observed the gate yet.
const NOT_OBSERVED: usize = usize::MAX;

// ---------------------------------------------------------------------------------------
// Gate unit contracts
// ---------------------------------------------------------------------------------------

/// PAU12: the two priorities exist and are distinct.
#[test]
fn test_pau_inference_priority_variants() {
    assert_ne!(
        InferencePriority::Interactive,
        InferencePriority::Background
    );
}

/// PAU12: guards are counted while alive and released on drop (saturating).
#[test]
fn test_pau_interactive_demand_counts_live_guards() {
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    assert_eq!(gate.interactive_demand(), 0);
    let a: InteractiveDemandGuard = gate.register_interactive();
    let b = gate.register_interactive();
    assert_eq!(gate.interactive_demand(), 2);
    drop(a);
    assert_eq!(gate.interactive_demand(), 1);
    drop(b);
    assert_eq!(gate.interactive_demand(), 0);
    assert_eq!(InferenceGate::default().interactive_demand(), 0);
}

/// PAU12: clones share the semaphore and the demand counter.
#[tokio::test]
async fn test_pau_gate_clones_share_permits_and_demand() {
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let clone = gate.clone();
    let guard = clone.register_interactive();
    assert_eq!(
        gate.interactive_demand(),
        1,
        "demand is shared across clones"
    );
    drop(guard);
    let permit = gate
        .acquire_within(Duration::ZERO)
        .await
        .expect("free slot");
    assert_eq!(clone.available_permits(), 0, "the semaphore is shared");
    assert!(clone.try_acquire_background().is_none());
    drop(permit);
    assert_eq!(clone.available_permits(), 1);
}

/// PAU12: with a live interactive guard the background acquisition is refused, even with a
/// free slot; without one it succeeds immediately.
#[tokio::test]
async fn test_pau_background_acquisition_yields_to_interactive_demand() {
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let guard = gate.register_interactive();
    assert_eq!(gate.available_permits(), 1);
    assert!(
        gate.try_acquire_background().is_none(),
        "a live InteractiveDemandGuard must refuse background acquisition"
    );
    assert_eq!(
        gate.available_permits(),
        1,
        "a refused attempt holds nothing"
    );
    drop(guard);
    let permit = gate
        .try_acquire_background()
        .expect("no demand and a free slot");
    assert_eq!(gate.available_permits(), 0);
    drop(permit);
}

/// PAU12: background acquisition never waits for a busy slot.
#[tokio::test]
async fn test_pau_background_acquisition_never_waits() {
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let held = gate.acquire_within(Duration::ZERO).await.unwrap();
    let started = Instant::now();
    assert!(gate.try_acquire_background().is_none());
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "try_acquire_background must return at once"
    );
    drop(held);
}

/// PAU12: an interactive request still acquires while a background permit is held, as
/// soon as it is released (FIFO acquire_within unchanged).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_interactive_acquires_after_a_background_job() {
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let background = gate.try_acquire_background().unwrap();
    let releaser = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(background);
    });
    let _demand = gate.register_interactive();
    let permit = gate.acquire_within(Duration::from_millis(900)).await;
    assert!(permit.is_some(), "the interactive request gets the slot");
    releaser.await.unwrap();
}

// ---------------------------------------------------------------------------------------
// Dispatcher wiring
// ---------------------------------------------------------------------------------------

struct DispatcherFixture {
    parts: PipelineParts,
    gate: InferenceGate,
    demand_at_wake: Arc<AtomicUsize>,
    sock_path: PathBuf,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for DispatcherFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn dispatcher_fixture(
    enrolled: Vec<(u32, Enrollment)>,
    max_attempts: u32,
) -> DispatcherFixture {
    let mut options = PipelineOptions::new(current_monotonic_nanos);
    options.enrolled = enrolled;
    options.max_attempts = max_attempts;
    let parts = build_pipeline(options).await;
    let gate = InferenceGate::new(1, Duration::from_millis(80));

    let demand_at_wake = Arc::new(AtomicUsize::new(NOT_OBSERVED));
    {
        let observer = gate.clone();
        let slot = Arc::clone(&demand_at_wake);
        *parts.camera.on_wake.lock().unwrap() = Some(Box::new(move || {
            slot.store(observer.interactive_demand(), Ordering::SeqCst);
        }));
    }

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
                logind_sessions_dir: parts.temp.path().to_path_buf(),
            },
            health,
            parts.components.clone(),
        )
        .with_inference_gate(gate.clone())
        .with_expected_embedding_model(EMBEDDING_MODEL_ID),
    );
    let sock_path = parts.temp.path().join("priority.sock");
    let listener = UnixListener::bind(&sock_path).unwrap();
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let disp = dispatcher.clone();
            tokio::spawn(async move {
                let _ = disp.handle_connection(stream).await;
            });
        }
    });
    DispatcherFixture {
        parts,
        gate,
        demand_at_wake,
        sock_path,
        server,
    }
}

async fn send_auth(sock_path: PathBuf, uid: u32, tag: u8, deadline_ns: u64) -> Response {
    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [tag; 32],
        uid_hint: uid,
        service: "sudo".into(),
        deadline_monotonic_ns: deadline_ns,
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

/// The test process UID (the dispatcher harness mode binds `peer.uid == uid_hint`).
fn own_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

/// PAU12: an `Auth` request holds an interactive demand while it wakes the camera and runs
/// the consensus, and releases it once the verdict is rendered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_dispatcher_holds_an_interactive_guard_for_every_auth_request() {
    let uid = own_uid();
    let fx = dispatcher_fixture(vec![(uid, Enrollment::LiveIdentity)], 40).await;
    let resp = send_auth(fx.sock_path.clone(), uid, 1, u64::MAX).await;
    assert_eq!(resp.verdict, Verdict::Allow);
    assert_eq!(
        fx.demand_at_wake.load(Ordering::SeqCst),
        1,
        "the Auth request must be registered as interactive demand during Step 8"
    );
    assert_eq!(
        fx.gate.interactive_demand(),
        0,
        "the guard is dropped once the request returns"
    );
}

/// `(label, enrolled templates, max_attempts)` of one dispatcher return path.
type ReturnPathCase = (&'static str, Vec<(u32, Enrollment)>, u32);

/// PAU12: every early return of Step 8 drops the guard (not enrolled, foreign template,
/// store error, rate limited, camera not ready, spoof veto).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_dispatcher_drops_the_guard_on_every_return_path() {
    let uid = own_uid();
    let cases: Vec<ReturnPathCase> = vec![
        ("not enrolled", vec![], 40),
        (
            "foreign template",
            vec![(uid, Enrollment::RetiredArcFace)],
            40,
        ),
        ("store error", vec![(uid, Enrollment::Corrupt)], 40),
        ("rate limited", vec![(uid, Enrollment::LiveIdentity)], 1),
        (
            "camera not ready",
            vec![(uid, Enrollment::LiveIdentity)],
            40,
        ),
        ("spoof veto", vec![(uid, Enrollment::LiveIdentity)], 40),
        ("no match", vec![(uid, Enrollment::Similarity(0.10))], 40),
    ];
    for (label, enrolled, max_attempts) in cases {
        let fx = dispatcher_fixture(enrolled, max_attempts).await;
        match label {
            "rate limited" => {
                let first = send_auth(fx.sock_path.clone(), uid, 1, u64::MAX).await;
                assert_eq!(first.verdict, Verdict::Allow, "{label}: first request");
            }
            "camera not ready" => fx.parts.camera.never_ready.store(true, Ordering::SeqCst),
            "spoof veto" => fx
                .parts
                .pad
                .set_result(soos_inference_ort::PadResult::spoof(
                    0.99,
                    soos_inference_ort::AttackType::PrintPhoto,
                )),
            _ => {}
        }
        let resp = send_auth(fx.sock_path.clone(), uid, 2, u64::MAX).await;
        assert_ne!(resp.verdict, Verdict::Allow, "{label}: must not authorize");
        assert_eq!(
            fx.gate.interactive_demand(),
            0,
            "{label}: the interactive guard must be dropped on this return path"
        );
    }
}

/// PAU12: an `Auth` request started while a presence (background) job holds the inference
/// slot still renders `Allow` within its deadline.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_auth_during_a_presence_job_still_allows_within_its_deadline() {
    let uid = own_uid();
    let fx = dispatcher_fixture(vec![(uid, Enrollment::LiveIdentity)], 40).await;
    let background = fx
        .gate
        .try_acquire_background()
        .expect("no interactive demand yet");
    let holder = tokio::spawn(async move {
        // One presence inference job (well above the typical SFace pass).
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(background);
    });
    let started = Instant::now();
    let now = current_monotonic_nanos().unwrap();
    let deadline = now + 1_000_000_000;
    let resp = send_auth(fx.sock_path.clone(), uid, 3, deadline).await;
    let elapsed = started.elapsed();
    assert_eq!(
        (resp.verdict, resp.reason_class),
        (Verdict::Allow, ReasonClass::FaceMatch),
        "PAM keeps priority over a presence job"
    );
    assert!(
        elapsed < Duration::from_millis(1000),
        "the Allow must be rendered within the 1000 ms client deadline, took {elapsed:?}"
    );
    holder.await.unwrap();
    assert_eq!(fx.gate.interactive_demand(), 0);
}

// ---------------------------------------------------------------------------------------
// PAU2 through the dispatcher
// ---------------------------------------------------------------------------------------

/// PAU2: under the default rate limit the 40th `Auth` attempt of one UID inside 60 s is
/// evaluated and the 41st is `ProtocolError` / `RateLimited`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_forty_first_auth_attempt_is_rate_limited_by_default() {
    let uid = own_uid();
    let fx = dispatcher_fixture(vec![], soos_policy::RateLimitConfig::default().max_attempts).await;
    for tag in 0..40u8 {
        let resp = send_auth(fx.sock_path.clone(), uid, tag, u64::MAX).await;
        assert_eq!(
            (resp.verdict, resp.reason_class),
            (Verdict::Unavailable, ReasonClass::InternalError),
            "attempt {} (not enrolled) must be evaluated, not rate limited",
            u32::from(tag) + 1
        );
    }
    let resp = send_auth(fx.sock_path.clone(), uid, 40, u64::MAX).await;
    assert_eq!(
        (resp.verdict, resp.reason_class),
        (Verdict::ProtocolError, ReasonClass::RateLimited),
        "the 41st attempt inside the window must be rate limited"
    );
    assert_eq!(fx.parts.remaining_attempts(uid), 0);
}

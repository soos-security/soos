//! Contract tests for GitHub #310 (review finding DMN-NEW-1): the `PasswordFailed` evidence
//! write runs on the blocking pool, tracked in `ConnectionDispatcher::evidence_writes` like
//! the spoof evidence write, and never on a Tokio worker.
//!
//! The evidence base-directory `flock` is held by the test for the whole scenario, so the
//! retention rotation that follows the snapshot write cannot complete. The event handler must
//! still return, a `Status` request on another connection must still be answered within
//! `connection_timeout`, and the shutdown drain must stay bounded by its budget.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract test suite uses direct assertions, unwrap and indexing"
)]

use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, FlockArg};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::PipelineComponents;
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    Event, EventKind, Request, RequestKind, StatusResponse, CURRENT_VERSION,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

/// `connection_timeout` of the dispatcher under test.
const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1500);

/// Shutdown drain budget used by the bounded-drain assertion.
const DRAIN_BUDGET: Duration = Duration::from_millis(200);

/// Outer watchdog: the whole scenario must finish well within this bound.
const WATCHDOG: Duration = Duration::from_secs(10);

/// What the scenario observed, reported from the runtime thread to the test thread.
#[derive(Debug)]
struct Outcome {
    event_handler_returned: bool,
    status_within_timeout: bool,
    tracked_writes: usize,
    drain_aborted: usize,
    drain_elapsed: Duration,
}

async fn scenario(held_base: PathBuf, temp_root: PathBuf) -> Outcome {
    let sock_path = temp_root.join("offload.sock");
    let bio_store = Arc::new(
        BiometricStore::new(
            temp_root.join("biometrics"),
            BioMasterKey::generate().expect("bio key"),
        )
        .expect("biometric store"),
    );
    let evidence_store = Arc::new(EvidenceStore::new(
        EvidenceConfig {
            enabled: true,
            base_dir: held_base,
            retention_days: 7,
            daily_cap_per_uid: 5,
            key_path: temp_root.join("evidence.key"),
        },
        EvMasterKey::generate().expect("evidence key"),
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
    for _ in 0..200 {
        if camera.latest_frame().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(camera.latest_frame().is_some(), "mock camera frame");

    let vision = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95)),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig::default(),
    ));
    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        ThresholdConfig::default(),
        RateLimiter::new(RateLimitConfig::new(10, 60_000_000_000)),
    )));
    let components = PipelineComponents {
        camera: camera.clone(),
        vision,
        biometric_store: bio_store,
        evidence_store,
        policy,
    };
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    health.set_camera_ready(camera.is_ready());
    health.set_models_verified(true);
    let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
        DispatcherConfig {
            max_concurrent_connections: 8,
            connection_timeout: CONNECTION_TIMEOUT,
            enforce_active_session: false,
            logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
        },
        health,
        components,
    ));

    let listener = UnixListener::bind(&sock_path).expect("bind");
    let (event_done_tx, mut event_done_rx) = tokio::sync::mpsc::channel::<()>(1);
    let server = dispatcher.clone();
    tokio::spawn(async move {
        // First connection: the PasswordFailed event; its completion is reported.
        if let Ok((stream, _)) = listener.accept().await {
            let d = server.clone();
            tokio::spawn(async move {
                let _ = d.handle_connection(stream).await;
                let _ = event_done_tx.send(()).await;
            });
        }
        while let Ok((stream, _)) = listener.accept().await {
            let d = server.clone();
            tokio::spawn(async move {
                let _ = d.handle_connection(stream).await;
            });
        }
    });

    let uid = nix::unistd::getuid().as_raw();
    let mut event_client = UnixStream::connect(&sock_path)
        .await
        .expect("connect event");
    let event = Event {
        version: CURRENT_VERSION,
        kind: EventKind::PasswordFailed,
        request_id: Some([0x31; 32]),
        uid: Some(uid),
        service: "sudo".into(),
        timestamp_monotonic_ns: 1_000_000,
    };
    event_client
        .write_all(&encode(&event).expect("encode event"))
        .await
        .expect("write event");
    event_client.flush().await.expect("flush event");
    drop(event_client);

    // Wall-clock bounds: with a single worker, a blocked worker also stalls the timer driver,
    // so `tokio::time::timeout` alone cannot observe the stall.
    let event_started = Instant::now();
    let event_handler_returned = tokio::time::timeout(CONNECTION_TIMEOUT, event_done_rx.recv())
        .await
        .is_ok_and(|v| v.is_some())
        && event_started.elapsed() < CONNECTION_TIMEOUT;

    let status_started = Instant::now();
    let status_within_timeout = tokio::time::timeout(CONNECTION_TIMEOUT, async {
        let mut client = UnixStream::connect(&sock_path)
            .await
            .expect("connect status");
        let req = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Status,
            request_id: [0x32; 32],
            uid_hint: uid,
            service: "soos-admin".into(),
            deadline_monotonic_ns: u64::MAX,
        };
        client
            .write_all(&encode(&req).expect("encode status"))
            .await
            .expect("write status");
        client.flush().await.expect("flush status");
        let mut len_bytes = [0u8; 4];
        client.read_exact(&mut len_bytes).await.expect("read len");
        let len = u32::from_be_bytes(len_bytes) as usize;
        let mut buf = vec![0u8; 4 + len];
        buf[..4].copy_from_slice(&len_bytes);
        client.read_exact(&mut buf[4..]).await.expect("read body");
        let status: StatusResponse = decode(&buf).expect("decode status");
        status.version == CURRENT_VERSION
    })
    .await
    .unwrap_or(false)
        && status_started.elapsed() < CONNECTION_TIMEOUT;

    let tracked_writes = dispatcher.evidence_writes().in_flight();
    let drain_started = Instant::now();
    let report = dispatcher.evidence_writes().drain(DRAIN_BUDGET).await;
    Outcome {
        event_handler_returned,
        status_within_timeout,
        tracked_writes,
        drain_aborted: report.aborted,
        drain_elapsed: drain_started.elapsed(),
    }
}

#[test]
fn test_310_password_failed_evidence_write_never_blocks_a_worker() {
    let temp = tempdir().expect("tempdir");
    let base = temp.path().join("evidence");
    std::fs::create_dir(&base).expect("evidence dir");
    // Held for the whole scenario, like a long-running `soos-enroll migrate --evidence`.
    let held = Flock::lock(
        std::fs::File::open(&base).expect("open evidence dir"),
        FlockArg::LockExclusiveNonblock,
    )
    .expect("hold evidence lock");

    let (tx, rx) = mpsc::channel();
    let root = temp.path().to_path_buf();
    std::thread::spawn(move || {
        // A single worker: an inline blocking write would freeze every connection.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("runtime");
        let outcome = runtime.block_on(scenario(base, root));
        let _ = tx.send(outcome);
        runtime.shutdown_timeout(Duration::from_millis(100));
    });

    let outcome = rx
        .recv_timeout(WATCHDOG)
        .expect("the daemon froze while the PasswordFailed evidence write was blocked");
    drop(held);

    assert!(
        outcome.event_handler_returned,
        "the PasswordFailed handler must return while the evidence write is blocked: {outcome:?}"
    );
    assert!(
        outcome.status_within_timeout,
        "a Status request must be answered within connection_timeout: {outcome:?}"
    );
    assert_eq!(
        outcome.tracked_writes, 1,
        "the PasswordFailed write must be tracked in evidence_writes: {outcome:?}"
    );
    assert_eq!(
        outcome.drain_aborted, 1,
        "the blocked write is abandoned by the bounded drain: {outcome:?}"
    );
    assert!(
        outcome.drain_elapsed < DRAIN_BUDGET + Duration::from_secs(1),
        "the shutdown drain must stay bounded by its budget: {outcome:?}"
    );
}

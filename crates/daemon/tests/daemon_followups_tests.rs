//! Daemon follow-ups of GitHub #287 (walkthrough 147).
//!
//! - DFU2: `accept()` errors (`EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM`, ...) back off with a
//!   bounded exponential delay reset on success; the loop never stops and shutdown stays
//!   responsive during a backoff.
//! - DFU3: spoof evidence writes are tracked and drained within the shutdown budget.
//! - DFU4: a clock failure on the response path downgrades `Allow` (never rendered without
//!   a clock), exercised through `stamp_response`, the function `build_response` uses.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::{
    stamp_response, ConnectionDispatcher, PAD_FAILED_EVIDENCE_REASON, RESPONSE_VALIDITY_NS,
};
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::PipelineComponents;
use soos_daemon::shutdown::{
    accept_with_backoff, AcceptBackoff, BlockingTasks, ConnectionTasks, ACCEPT_BACKOFF_INITIAL,
    ACCEPT_BACKOFF_MAX,
};
use soos_daemon::DaemonError;
use soos_evidence_store::{
    EvidenceConfig, EvidenceRecord, EvidenceStore, MasterKey as EvMasterKey,
};
use soos_inference_ort::{
    AttackType, MockEmbeddingExtractor, MockFaceDetector, MockPadDetector, PadResult,
};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::message::encode_request;
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

type AcceptFuture = Pin<Box<dyn Future<Output = io::Result<UnixStream>> + Send>>;

fn dispatcher() -> Arc<ConnectionDispatcher> {
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    Arc::new(ConnectionDispatcher::new(
        DispatcherConfig {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_millis(500),
            enforce_active_session: false,
            logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
        },
        health,
    ))
}

fn emfile() -> io::Error {
    io::Error::from_raw_os_error(nix::errno::Errno::EMFILE as i32)
}

// ---------------------------------------------------------------------------
// DFU2: bounded accept() backoff
// ---------------------------------------------------------------------------

#[test]
fn test_accept_backoff_doubles_from_initial_and_caps_at_max() {
    let mut backoff = AcceptBackoff::new(Duration::from_millis(10), Duration::from_millis(80));
    let delays: Vec<Duration> = (0..6).map(|_| backoff.on_error()).collect();
    assert_eq!(
        delays,
        [10, 20, 40, 80, 80, 80].map(Duration::from_millis).to_vec(),
        "the delay doubles from the initial value and is capped at the maximum"
    );
    backoff.on_success();
    assert_eq!(
        backoff.on_error(),
        Duration::from_millis(10),
        "a successful accept resets the backoff"
    );
}

#[test]
fn test_accept_backoff_defaults_and_degenerate_bounds() {
    assert!(ACCEPT_BACKOFF_INITIAL > Duration::ZERO);
    assert!(ACCEPT_BACKOFF_INITIAL <= ACCEPT_BACKOFF_MAX);
    assert!(
        ACCEPT_BACKOFF_MAX <= Duration::from_secs(1),
        "the backoff is capped at about one second"
    );
    let mut default = AcceptBackoff::default();
    assert_eq!(default.on_error(), ACCEPT_BACKOFF_INITIAL);

    // A zero configuration never degenerates into a busy loop.
    let mut zero = AcceptBackoff::new(Duration::ZERO, Duration::ZERO);
    let first = zero.on_error();
    assert!(first > Duration::ZERO, "a backoff delay is never zero");
    // A maximum below the initial value is raised to the initial value.
    let mut inverted = AcceptBackoff::new(Duration::from_millis(50), Duration::from_millis(5));
    assert_eq!(inverted.on_error(), Duration::from_millis(50));
    assert_eq!(inverted.on_error(), Duration::from_millis(50));
    // Many consecutive errors never overflow or exceed the cap.
    let mut long = AcceptBackoff::default();
    for _ in 0..10_000 {
        assert!(long.on_error() <= ACCEPT_BACKOFF_MAX);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_accept_errors_back_off_without_stopping_the_loop() {
    const FAILURES: usize = 4;
    let calls: Arc<Mutex<Vec<Instant>>> = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::new(tokio::sync::Notify::new());
    let clients: Arc<Mutex<Vec<UnixStream>>> = Arc::new(Mutex::new(Vec::new()));

    let accept = {
        let calls = Arc::clone(&calls);
        let served = Arc::clone(&served);
        let clients = Arc::clone(&clients);
        move || -> AcceptFuture {
            let n = {
                let mut calls = calls.lock().unwrap();
                calls.push(Instant::now());
                calls.len()
            };
            if n <= FAILURES {
                Box::pin(std::future::ready(Err(emfile())))
            } else if n == FAILURES + 1 {
                let (server, client) = UnixStream::pair().unwrap();
                clients.lock().unwrap().push(client);
                served.notify_one();
                Box::pin(std::future::ready(Ok(server)))
            } else {
                Box::pin(std::future::pending())
            }
        }
    };

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let mut tasks = ConnectionTasks::new();
    let backoff = AcceptBackoff::new(Duration::from_millis(20), Duration::from_millis(40));
    let loop_dispatcher = dispatcher();
    let waiter = {
        let served = Arc::clone(&served);
        tokio::spawn(async move {
            served.notified().await;
            // Let the loop spawn the handler before the shutdown signal.
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = stop_tx.send(());
        })
    };
    tokio::time::timeout(
        Duration::from_secs(5),
        accept_with_backoff(accept, loop_dispatcher, &mut tasks, backoff, async {
            let _ = stop_rx.await;
        }),
    )
    .await
    .expect("the accept loop must return on shutdown");
    waiter.await.unwrap();

    let calls = calls.lock().unwrap().clone();
    assert!(
        calls.len() > FAILURES,
        "the loop must keep accepting after errors, got {} calls",
        calls.len()
    );
    let expected = [20u64, 40, 40, 40];
    for (i, min_ms) in expected.iter().enumerate() {
        let gap = calls[i + 1].duration_since(calls[i]);
        assert!(
            gap >= Duration::from_millis(*min_ms),
            "accept call {} followed error {} after {gap:?}, expected a backoff >= {min_ms} ms",
            i + 2,
            i + 1
        );
    }
    assert_eq!(
        tasks.len(),
        1,
        "the connection accepted after the errors must be served"
    );
    clients.lock().unwrap().clear();
    let report = tasks.drain(Duration::from_secs(2)).await;
    assert_eq!(report.completed, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shutdown_interrupts_accept_backoff_promptly() {
    let first_error = Arc::new(tokio::sync::Notify::new());
    let accept = {
        let first_error = Arc::clone(&first_error);
        move || -> AcceptFuture {
            first_error.notify_one();
            Box::pin(std::future::ready(Err(emfile())))
        }
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let signalled_at = Arc::new(Mutex::new(None::<Instant>));
    let signaller = {
        let first_error = Arc::clone(&first_error);
        let signalled_at = Arc::clone(&signalled_at);
        tokio::spawn(async move {
            first_error.notified().await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            *signalled_at.lock().unwrap() = Some(Instant::now());
            let _ = stop_tx.send(());
        })
    };
    let mut tasks = ConnectionTasks::new();
    // A 1 s backoff is in progress when the shutdown signal arrives.
    let backoff = AcceptBackoff::new(Duration::from_secs(1), Duration::from_secs(1));
    tokio::time::timeout(
        Duration::from_secs(5),
        accept_with_backoff(accept, dispatcher(), &mut tasks, backoff, async {
            let _ = stop_rx.await;
        }),
    )
    .await
    .expect("the accept loop must return on shutdown");
    let returned_at = Instant::now();
    signaller.await.unwrap();
    let signalled = signalled_at.lock().unwrap().expect("shutdown signalled");
    let latency = returned_at.duration_since(signalled);
    assert!(
        latency < Duration::from_millis(500),
        "shutdown must interrupt a backoff sleep, took {latency:?}"
    );
    assert!(tasks.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_accept_until_shutdown_still_serves_a_real_listener() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("dfu2.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let mut tasks = ConnectionTasks::new();
    let client = tokio::spawn({
        let sock = sock.clone();
        async move {
            let request = Request {
                version: CURRENT_VERSION,
                kind: RequestKind::Status,
                request_id: [0x5B; 32],
                uid_hint: nix::unistd::getuid().as_raw(),
                service: "soos-admin".into(),
                deadline_monotonic_ns: 0,
            };
            let mut stream = UnixStream::connect(&sock).await.unwrap();
            stream
                .write_all(&encode_request(&request).unwrap())
                .await
                .unwrap();
            let mut len_bytes = [0u8; 4];
            let answered =
                tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut len_bytes))
                    .await;
            let _ = stop_tx.send(());
            answered.is_ok_and(|r| r.is_ok()) && u32::from_be_bytes(len_bytes) > 0
        }
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        soos_daemon::shutdown::accept_until_shutdown(&listener, dispatcher(), &mut tasks, async {
            let _ = stop_rx.await;
        }),
    )
    .await
    .expect("accept loop returns on shutdown");
    assert!(
        client.await.unwrap(),
        "the production accept loop must still serve a real listener"
    );
    let _ = tasks.drain(Duration::from_secs(2)).await;
}

// ---------------------------------------------------------------------------
// DFU3: tracked blocking evidence writes
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_blocking_tasks_drain_waits_for_in_flight_write() {
    let tasks = BlockingTasks::new();
    let done = Arc::new(AtomicBool::new(false));
    {
        let done = Arc::clone(&done);
        tasks.spawn_blocking(move || {
            std::thread::sleep(Duration::from_millis(150));
            done.store(true, Ordering::SeqCst);
        });
    }
    assert_eq!(tasks.in_flight(), 1);
    let report = tasks.drain(Duration::from_secs(5)).await;
    assert!(
        done.load(Ordering::SeqCst),
        "drain must wait for the in-flight write"
    );
    assert_eq!(report.completed, 1);
    assert!(report.is_clean());
    assert_eq!(tasks.in_flight(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_blocking_tasks_drain_is_bounded_by_budget() {
    let tasks = BlockingTasks::new();
    tasks.spawn_blocking(|| std::thread::sleep(Duration::from_millis(1500)));
    tasks.spawn_blocking(|| {});
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = Instant::now();
    let report = tasks.drain(Duration::from_millis(100)).await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(900),
        "drain must be bounded by its budget, took {elapsed:?}"
    );
    assert_eq!(report.completed, 1);
    assert_eq!(
        report.aborted, 1,
        "a write still running at the deadline is reported as abandoned"
    );
    assert!(!report.is_clean());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_blocking_tasks_reap_finished_jobs_on_spawn() {
    let tasks = BlockingTasks::new();
    let ran = Arc::new(AtomicUsize::new(0));
    for _ in 0..16 {
        let ran = Arc::clone(&ran);
        tasks.spawn_blocking(move || {
            ran.fetch_add(1, Ordering::SeqCst);
        });
    }
    let started = Instant::now();
    while ran.load(Ordering::SeqCst) < 16 && started.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    tasks.spawn_blocking(|| std::thread::sleep(Duration::from_millis(100)));
    assert_eq!(
        tasks.in_flight(),
        1,
        "finished writes are reaped, so the tracked set stays bounded"
    );
    let report = tasks.drain(Duration::from_secs(5)).await;
    assert_eq!(report.completed, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_blocking_task_panic_is_counted_not_propagated() {
    let tasks = BlockingTasks::new();
    tasks.spawn_blocking(|| panic!("DFU3-SENSITIVE-PAYLOAD"));
    let report = tasks.drain(Duration::from_secs(5)).await;
    assert_eq!(report.panicked, 1);
    assert!(!report.is_clean());
}

/// Minimal pipeline fixture whose PAD detector reports a spoof (see `pad_evidence_tests`).
struct SpoofFixture {
    dispatcher: Arc<ConnectionDispatcher>,
    evidence_store: Arc<EvidenceStore>,
    camera: Arc<MockCameraManager>,
    sock_path: PathBuf,
    current_uid: u32,
    _temp_dir: tempfile::TempDir,
}

impl SpoofFixture {
    async fn new() -> Self {
        let temp_dir = tempdir().expect("tempdir");
        let sock_path = temp_dir.path().join("dfu3.sock");
        let bio_dir = temp_dir.path().join("biometrics");
        let ev_dir = temp_dir.path().join("evidence");
        std::fs::create_dir_all(&ev_dir).expect("create evidence dir");

        let bio_store = Arc::new(
            BiometricStore::new(&bio_dir, BioMasterKey::generate().expect("bio key"))
                .expect("biometric store"),
        );
        let evidence_store = Arc::new(EvidenceStore::new(
            EvidenceConfig {
                enabled: true,
                base_dir: ev_dir,
                retention_days: 7,
                daily_cap_per_uid: 5,
                key_path: temp_dir.path().join("evidence.key"),
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
        let mut first = None;
        for _ in 0..100 {
            if let Some(frame) = camera.latest_frame() {
                first = Some(frame);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let frame = first.expect("mock camera frame");

        let detector = Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95));
        let pad = Arc::new(MockPadDetector::new_live());
        let vision = Arc::new(VisionPipeline::new(
            detector,
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
        let enrolled = vision
            .process_frame(&frame)
            .expect("process frame")
            .embedding
            .as_slice()
            .to_vec();
        let current_uid = nix::unistd::getuid().as_raw();
        bio_store
            .enroll(
                &BiometricTemplate::new(
                    current_uid,
                    "mock-model".into(),
                    "1.0".into(),
                    1,
                    zeroize::Zeroizing::new(enrolled),
                )
                .expect("template"),
            )
            .expect("enroll");
        pad.set_result(PadResult::spoof(0.04, AttackType::PrintPhoto));

        let components = PipelineComponents {
            camera: camera.clone(),
            vision,
            biometric_store: bio_store,
            evidence_store: evidence_store.clone(),
            policy: Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
                ThresholdConfig::default(),
                RateLimiter::new(RateLimitConfig::new(10, 60_000_000_000)),
            ))),
        };
        let health = Arc::new(HealthState::new());
        health.set_socket_ready(true);
        health.set_camera_ready(camera.is_ready());
        health.set_models_verified(true);
        let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
            DispatcherConfig {
                max_concurrent_connections: 8,
                connection_timeout: Duration::from_secs(5),
                enforce_active_session: false,
                logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
            },
            health,
            components,
        ));
        Self {
            dispatcher,
            evidence_store,
            camera,
            sock_path,
            current_uid,
            _temp_dir: temp_dir,
        }
    }

    async fn auth_once(&self) -> Response {
        let listener = UnixListener::bind(&self.sock_path).expect("bind");
        let dispatcher = self.dispatcher.clone();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let _ = dispatcher.handle_connection(stream).await;
            }
        });
        let req = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id: [87; 32],
            uid_hint: self.current_uid,
            service: "sudo".into(),
            deadline_monotonic_ns: u64::MAX,
        };
        let mut client = UnixStream::connect(&self.sock_path).await.expect("connect");
        client.write_all(&encode(&req).unwrap()).await.unwrap();
        let mut len_bytes = [0u8; 4];
        client.read_exact(&mut len_bytes).await.unwrap();
        let len = u32::from_be_bytes(len_bytes) as usize;
        let mut buf = vec![0u8; 4 + len];
        buf[..4].copy_from_slice(&len_bytes);
        client.read_exact(&mut buf[4..]).await.unwrap();
        decode::<Response>(&buf).unwrap()
    }

    fn records(&self) -> Vec<EvidenceRecord> {
        let base = self.evidence_store.config().base_dir.clone();
        let mut records = Vec::new();
        for entry in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            if entry.path().is_dir() {
                let date = entry.file_name().to_string_lossy().to_string();
                for file in self.evidence_store.list_snapshots_for_date(&date).unwrap() {
                    records.push(self.evidence_store.load_snapshot(&file).unwrap());
                }
            }
        }
        records
    }
}

impl Drop for SpoofFixture {
    fn drop(&mut self) {
        self.camera.stop();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_spoof_evidence_write_is_tracked_and_drained_at_shutdown() {
    let fixture = SpoofFixture::new().await;
    let resp = fixture.auth_once().await;
    assert_eq!(resp.verdict, Verdict::Deny);
    assert_eq!(resp.reason_class, ReasonClass::PadFailed);

    // The write was spawned before the response was rendered, so it is tracked now.
    let report = fixture
        .dispatcher
        .evidence_writes()
        .drain(Duration::from_secs(5))
        .await;
    assert_eq!(
        report.completed, 1,
        "the spoof evidence write must be tracked and drained"
    );
    assert!(report.is_clean());
    // No polling: once drained, the snapshot is on disk.
    let records = fixture.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason, PAD_FAILED_EVIDENCE_REASON);
}

#[test]
fn test_production_main_drains_evidence_writes_within_the_shutdown_budget() {
    let main_rs = include_str!("../src/main.rs");
    let connections = main_rs
        .find(".drain(drain_budget)")
        .expect("bounded connection drain at shutdown");
    let evidence = main_rs
        .find("evidence_writes().drain(")
        .expect("main.rs must drain the tracked evidence writes at shutdown");
    assert!(
        connections < evidence,
        "evidence writes are drained after the handlers that spawn them"
    );
    assert!(
        main_rs.contains("drain_budget.saturating_sub("),
        "the evidence drain uses what is left of the one-connection_timeout budget"
    );
    let dispatcher_rs = include_str!("../src/dispatcher.rs");
    assert!(
        !dispatcher_rs.contains("drop(tokio::task::spawn_blocking"),
        "no evidence write may be detached"
    );
}

// ---------------------------------------------------------------------------
// DFU4: clock failure downgrade branch
// ---------------------------------------------------------------------------

const ALL_VERDICTS: [Verdict; 4] = [
    Verdict::Allow,
    Verdict::Deny,
    Verdict::Unavailable,
    Verdict::ProtocolError,
];

fn clock_error() -> Result<u64, DaemonError> {
    Err(DaemonError::Clock("simulated clock failure".to_string()))
}

#[test]
fn test_clock_failure_downgrades_allow_to_unavailable_internal_error() {
    let resp = stamp_response(
        [3; 32],
        Verdict::Allow,
        ReasonClass::FaceMatch,
        clock_error(),
    );
    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.reason_class, ReasonClass::InternalError);
    assert_eq!(resp.issued_monotonic_ns, 0);
    assert_eq!(resp.expires_monotonic_ns, 0);
    assert_eq!(resp.request_id, [3; 32]);
    assert_eq!(resp.version, CURRENT_VERSION);
}

#[test]
fn test_zero_clock_reading_downgrades_allow() {
    let resp = stamp_response([4; 32], Verdict::Allow, ReasonClass::FaceMatch, Ok(0));
    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.reason_class, ReasonClass::InternalError);
    assert_eq!(
        (resp.issued_monotonic_ns, resp.expires_monotonic_ns),
        (0, 0)
    );
}

#[test]
fn test_clock_failure_never_yields_allow_for_any_verdict() {
    for verdict in ALL_VERDICTS {
        for clock in [clock_error(), Ok(0)] {
            let resp = stamp_response([5; 32], verdict, ReasonClass::FaceMatch, clock);
            assert_ne!(resp.verdict, Verdict::Allow, "{verdict:?}");
            assert_eq!(resp.issued_monotonic_ns, 0, "{verdict:?}");
            assert_eq!(resp.expires_monotonic_ns, 0, "{verdict:?}");
            if verdict != Verdict::Allow {
                assert_eq!(resp.verdict, verdict, "non-Allow verdicts are kept");
                assert_eq!(resp.reason_class, ReasonClass::FaceMatch);
            }
        }
    }
}

#[test]
fn test_working_clock_keeps_verdict_and_stamps_validity_window() {
    let resp = stamp_response([6; 32], Verdict::Allow, ReasonClass::FaceMatch, Ok(5));
    assert_eq!(resp.verdict, Verdict::Allow);
    assert_eq!(resp.reason_class, ReasonClass::FaceMatch);
    assert_eq!(resp.issued_monotonic_ns, 5);
    assert_eq!(resp.expires_monotonic_ns, 5 + RESPONSE_VALIDITY_NS);
    let resp = stamp_response([6; 32], Verdict::Deny, ReasonClass::NoFace, Ok(u64::MAX));
    assert_eq!(resp.expires_monotonic_ns, u64::MAX, "the window saturates");
}

#[test]
fn test_build_response_renders_through_stamp_response() {
    let dispatcher_rs = include_str!("../src/dispatcher.rs");
    let body = dispatcher_rs
        .split("fn build_response(")
        .nth(1)
        .expect("build_response exists");
    let body = body.split("\n    }\n").next().unwrap_or(body);
    assert!(
        body.contains("stamp_response("),
        "build_response must render through stamp_response so the tested downgrade is the production branch"
    );
}

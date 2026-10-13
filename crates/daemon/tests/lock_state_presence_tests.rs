//! Presence-side publication of the lock-state snapshot and presence timing under
//! `LockState` polling (GitHub #380, PR 1 of #352; ADR 2026-10-13 "Read-Only Lock-Screen State
//! Channel"; matrix LKS12-LKS13). Driven through `PresenceWorker::tick()` with a shifted test
//! clock, as the presence contract tests of GitHub #323.

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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use common::{
    build_worker, fast_presence_config, locked_session, Enrollment, PipelineOptions,
    ScriptedAccountGuard, ScriptedAnswer, WorkerFixture, MS_NS,
};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::inference::InteractiveDemandGuard;
use soos_daemon::lock_state::{LockStateBoard, LockStateView, NOT_RECOGNIZED_REVEAL_FLOOR_MS};
use soos_daemon::presence::account::{AccountRefusal, AccountState};
use soos_daemon::presence::worker::{PresenceTick, ScanOutcome, SkipReason};
use soos_daemon::presence::{GLOBAL_DISABLE_FLAG, PRESENCE_DISABLE_FLAG};
use soos_daemon::DaemonError;
use soos_inference_ort::{AttackType, PadResult};
use soos_protocol::codec::decode;
use soos_protocol::message::encode_request;
use soos_protocol::types::{
    LockStateResponse, LockWidgetState, Request, RequestKind, CURRENT_VERSION,
};

const UID: u32 = 1000;
const FLOOR_NS: u64 = NOT_RECOGNIZED_REVEAL_FLOOR_MS * MS_NS;

fn advance(offset: &AtomicU64, ms: u64) {
    offset.fetch_add(ms * MS_NS, Ordering::SeqCst);
}

fn usable() -> ScriptedAccountGuard {
    ScriptedAccountGuard::always(AccountState::Usable)
}

fn view(state: LockWidgetState, sequence: u64) -> LockStateView {
    LockStateView { state, sequence }
}

/// A worker over one bound, locked session "2" of UID 1000, publishing into `board`.
async fn worker_with_board(
    options: PipelineOptions,
    config: soos_daemon::presence::config::PresenceConfig,
    board: &LockStateBoard,
) -> WorkerFixture<ScriptedAccountGuard> {
    worker_with_board_and_guard(options, config, board, usable()).await
}

async fn worker_with_board_and_guard(
    options: PipelineOptions,
    config: soos_daemon::presence::config::PresenceConfig,
    board: &LockStateBoard,
    guard: ScriptedAccountGuard,
) -> WorkerFixture<ScriptedAccountGuard> {
    let WorkerFixture {
        worker,
        logind,
        display,
        gate,
        switch_dir,
        parts,
    } = build_worker(options, config, guard).await;
    let worker = worker.with_lock_state_board(board.clone());
    logind.set_sessions(vec![locked_session("2", UID, "alice")]);
    WorkerFixture {
        worker,
        logind,
        display,
        gate,
        switch_dir,
        parts,
    }
}

/// Records the board view of UID 1000 at every camera wake (mid-scan observation).
fn record_views_on_wake(
    fx: &WorkerFixture<ScriptedAccountGuard>,
    board: &LockStateBoard,
    clock: fn() -> Result<u64, DaemonError>,
) -> Arc<Mutex<Vec<LockStateView>>> {
    let seen: Arc<Mutex<Vec<LockStateView>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let board = board.clone();
    *fx.parts.camera.on_wake.lock().unwrap() = Some(Box::new(move || {
        sink.lock().unwrap().push(board.view(UID, clock().unwrap()));
    }));
    seen
}

/// Ticks inside the grace (asserting it), shifts the clock past it and ticks again.
async fn scan_once(
    fx: &mut WorkerFixture<ScriptedAccountGuard>,
    offset: &AtomicU64,
) -> PresenceTick {
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(offset, 1000);
    fx.worker.tick().await
}

fn assert_scanning_then_not_recognized(
    label: &str,
    seen: &Arc<Mutex<Vec<LockStateView>>>,
    board: &LockStateBoard,
    clock: fn() -> Result<u64, DaemonError>,
) {
    let seen = seen.lock().unwrap().clone();
    assert!(!seen.is_empty(), "{label}: the camera was woken");
    assert!(
        seen.iter()
            .all(|v| *v == view(LockWidgetState::Scanning, 1)),
        "{label}: Scanning during the scan, got {seen:?}"
    );
    let now = clock().unwrap();
    assert_eq!(
        board.view(UID, now),
        view(LockWidgetState::Scanning, 1),
        "{label}: the end is hidden before the reveal floor"
    );
    assert_eq!(
        board.view(UID, now + FLOOR_NS),
        view(LockWidgetState::NotRecognized, 2),
        "{label}: collapsed into NotRecognized"
    );
}

/// LKS12: a presence non-match publishes `Scanning`, then `NotRecognized` after the floor.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_presence_no_match_maps_to_not_recognized() {
    test_clock!(OFFSET, clock);
    let board = LockStateBoard::new();
    let mut options = PipelineOptions::new(clock);
    options.enrolled = vec![(UID, Enrollment::Similarity(0.10))];
    let mut fx = worker_with_board(options, fast_presence_config(), &board).await;
    let seen = record_views_on_wake(&fx, &board, clock);
    assert_eq!(
        scan_once(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::NoMatch)
    );
    assert_scanning_then_not_recognized("no match", &seen, &board, clock);
}

/// LKS12: a presence spoof veto reads exactly like a non-match.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_presence_spoof_maps_to_not_recognized() {
    test_clock!(OFFSET, clock);
    let board = LockStateBoard::new();
    let mut fx =
        worker_with_board(PipelineOptions::new(clock), fast_presence_config(), &board).await;
    fx.parts
        .pad
        .set_result(PadResult::spoof(0.99, AttackType::PrintPhoto));
    let seen = record_views_on_wake(&fx, &board, clock);
    assert_eq!(
        scan_once(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::SpoofVetoed)
    );
    assert_scanning_then_not_recognized("spoof", &seen, &board, clock);
}

/// LKS12: a camera that never becomes ready reads `NotRecognized`, never a success.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_presence_camera_unavailable_maps_to_not_recognized() {
    test_clock!(OFFSET, clock);
    let board = LockStateBoard::new();
    let mut fx =
        worker_with_board(PipelineOptions::new(clock), fast_presence_config(), &board).await;
    fx.parts.camera.never_ready.store(true, Ordering::SeqCst);
    let seen = record_views_on_wake(&fx, &board, clock);
    assert_eq!(
        scan_once(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::CameraUnavailable)
    );
    assert_scanning_then_not_recognized("camera unavailable", &seen, &board, clock);
}

/// LKS12: an unlocked session publishes `Scanning` then `Idle` (no success state).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_presence_unlock_reads_idle() {
    test_clock!(OFFSET, clock);
    let board = LockStateBoard::new();
    let mut fx =
        worker_with_board(PipelineOptions::new(clock), fast_presence_config(), &board).await;
    let seen = record_views_on_wake(&fx, &board, clock);
    assert!(matches!(
        scan_once(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::Unlocked { .. })
    ));
    assert!(!seen.lock().unwrap().is_empty());
    let now = clock().unwrap();
    assert_eq!(board.view(UID, now), view(LockWidgetState::Scanning, 1));
    assert_eq!(
        board.view(UID, now + FLOOR_NS),
        view(LockWidgetState::Idle, 2)
    );
}

/// LKS12: a scan preempted by a PAM request settles to `Idle` (the PAM scan publishes its own).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_presence_preempted_scan_reads_idle() {
    test_clock!(OFFSET, clock);
    let board = LockStateBoard::new();
    let mut fx =
        worker_with_board(PipelineOptions::new(clock), fast_presence_config(), &board).await;
    let guards: Arc<Mutex<Vec<InteractiveDemandGuard>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let gate = fx.gate.clone();
        let guards = Arc::clone(&guards);
        fx.parts.set_extractor_hook(move || {
            let mut held = guards.lock().unwrap();
            if held.is_empty() {
                held.push(gate.register_interactive());
            }
        });
    }
    assert_eq!(
        scan_once(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::Preempted)
    );
    let now = clock().unwrap();
    assert_eq!(
        board.view(UID, now),
        view(LockWidgetState::Scanning, 1),
        "the preemption is not observable before the floor"
    );
    assert_eq!(
        board.view(UID, now + FLOOR_NS),
        view(LockWidgetState::Idle, 2)
    );
}

/// LKS12: a refusal after a match (fresh account check) reads `NotRecognized`, never a
/// success.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_presence_post_match_refusal_maps_to_not_recognized() {
    test_clock!(OFFSET, clock);
    let board = LockStateBoard::new();
    let guard = ScriptedAccountGuard::script(
        vec![ScriptedAnswer {
            delay: Duration::ZERO,
            state: AccountState::Usable,
        }],
        AccountState::Refused(AccountRefusal::PasswordLocked),
    );
    let mut fx = worker_with_board_and_guard(
        PipelineOptions::new(clock),
        fast_presence_config(),
        &board,
        guard,
    )
    .await;
    let seen = record_views_on_wake(&fx, &board, clock);
    assert_eq!(
        scan_once(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::AccountRefused(AccountRefusal::PasswordLocked))
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
    assert_scanning_then_not_recognized("post-match refusal", &seen, &board, clock);
}

/// LKS12: ticks that start no scan (grace, kill switch) publish nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_presence_skipped_ticks_publish_nothing() {
    for flag in [GLOBAL_DISABLE_FLAG, PRESENCE_DISABLE_FLAG] {
        test_clock!(OFFSET, clock);
        let board = LockStateBoard::new();
        let mut fx =
            worker_with_board(PipelineOptions::new(clock), fast_presence_config(), &board).await;
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::InGrace)
        );
        std::fs::write(fx.switch_dir.join(flag), "").unwrap();
        advance(&OFFSET, 2000);
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::KillSwitch)
        );
        assert_eq!(
            board.view(UID, clock().unwrap()),
            view(LockWidgetState::Idle, 0),
            "{flag}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// LKS13: presence timing is unchanged by LockState polling
// ---------------------------------------------------------------------------------------

async fn poll_lock_state(sock: &std::path::Path, uid: u32, count: u8) {
    let mut stream = UnixStream::connect(sock).await.expect("connect");
    for nonce in 0..count {
        let req = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::LockState,
            request_id: [nonce; 32],
            uid_hint: uid,
            service: "soos-lockstatus".into(),
            deadline_monotonic_ns: u64::MAX,
        };
        stream
            .write_all(&encode_request(&req).expect("encode"))
            .await
            .expect("write");
        let mut len = [0u8; 4];
        stream.read_exact(&mut len).await.expect("length");
        let mut frame = len.to_vec();
        frame.resize(4 + u32::from_be_bytes(len) as usize, 0);
        stream.read_exact(&mut frame[4..]).await.expect("body");
        assert!(
            decode::<LockStateResponse>(&frame).is_ok(),
            "LockState answered"
        );
    }
}

/// LKS13: the scan schedule of the presence contract (grace, one scan, `NotDue` within the
/// interval, the next scan after it, exactly two attempts) is unchanged while a widget polls
/// `LockState` between every tick through a dispatcher sharing the pipeline, the inference
/// gate and the board.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_lock_state_polling_never_changes_presence_timing() {
    test_clock!(OFFSET, clock);
    let board = LockStateBoard::new();
    let mut options = PipelineOptions::new(clock);
    options.enrolled = vec![(UID, Enrollment::Similarity(0.10))];
    let config = soos_daemon::presence::config::PresenceConfig {
        scan_interval: Duration::from_millis(5000),
        ..fast_presence_config()
    };
    let mut fx = worker_with_board(options, config, &board).await;

    let sock_dir = tempfile::tempdir().unwrap();
    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(
            DispatcherConfig {
                max_concurrent_connections: 8,
                connection_timeout: Duration::from_secs(5),
                enforce_active_session: false,
                logind_sessions_dir: sock_dir.path().to_path_buf(),
            },
            Arc::new(HealthState::new()),
            fx.parts.components.clone(),
        )
        .with_inference_gate(fx.gate.clone())
        .with_lock_state_board(board.clone()),
    );
    let sock = sock_dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let disp = Arc::clone(&dispatcher);
            tokio::spawn(async move {
                let _ = disp.handle_connection(stream).await;
            });
        }
    });
    let peer = nix::unistd::getuid().as_raw();

    poll_lock_state(&sock, peer, 2).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    poll_lock_state(&sock, peer, 2).await;
    assert_eq!(fx.parts.camera.wakes(), 0, "polling never wakes the camera");
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Scanned(ScanOutcome::NoMatch)
    );
    let wakes = fx.parts.camera.wakes();
    poll_lock_state(&sock, peer, 2).await;
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::NotDue),
        "polling never brings a scan forward"
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    poll_lock_state(&sock, peer, 2).await;
    assert_eq!(
        fx.parts.camera.wakes(),
        wakes,
        "no scan before the interval"
    );
    advance(&OFFSET, 4000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Scanned(ScanOutcome::NoMatch)
    );
    assert_eq!(
        fx.parts.remaining_attempts(UID),
        38,
        "exactly one attempt per scan, none for polling"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

// ---------------------------------------------------------------------------------------
// LKS19: the next presence scan never depends on when the previous one ended
// (candid review of 2026-10-13, MAJOR #1)
// ---------------------------------------------------------------------------------------

/// A worker whose first scan ends early (spoof veto at the first capture) or late (full
/// non-match at the decision budget), both after the grace.
async fn failing_worker(
    fast: bool,
    clock: fn() -> Result<u64, DaemonError>,
    board: &LockStateBoard,
) -> WorkerFixture<ScriptedAccountGuard> {
    let mut options = PipelineOptions::new(clock);
    if !fast {
        options.enrolled = vec![(UID, Enrollment::Similarity(0.10))];
    }
    let fx = worker_with_board(options, fast_presence_config(), board).await;
    if fast {
        fx.parts
            .pad
            .set_result(PadResult::spoof(0.99, AttackType::PrintPhoto));
    }
    fx
}

/// LKS19: after a failed scan the worker holds until the scan start plus the reveal floor,
/// whatever the end; after an unlock or a skipped tick it does not hold at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_post_scan_hold_is_anchored_to_the_scan_start() {
    for fast in [true, false] {
        test_clock!(OFFSET, clock);
        let board = LockStateBoard::new();
        let mut fx = failing_worker(fast, clock, &board).await;
        let wakes: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let wakes = Arc::clone(&wakes);
            *fx.parts.camera.on_wake.lock().unwrap() = Some(Box::new(move || {
                wakes.lock().unwrap().push(clock().unwrap());
            }));
        }
        let tick = scan_once(&mut fx, &OFFSET).await;
        assert!(matches!(tick, PresenceTick::Scanned(_)), "{tick:?}");
        let now = clock().unwrap();
        let hold = fx.worker.post_scan_hold(&tick);
        let start = wakes.lock().unwrap()[0];
        let resume = now + u64::try_from(hold.as_nanos()).unwrap();
        assert!(
            resume + 5 * MS_NS >= start + FLOOR_NS && resume <= start + FLOOR_NS + 50 * MS_NS,
            "fast={fast}: the hold ends at the scan start plus the floor (start {start}, resume {resume})"
        );
        assert_eq!(
            fx.worker
                .post_scan_hold(&PresenceTick::Skipped(SkipReason::NotDue)),
            Duration::ZERO
        );
    }
    test_clock!(OFFSET2, clock2);
    let board = LockStateBoard::new();
    let mut fx =
        worker_with_board(PipelineOptions::new(clock2), fast_presence_config(), &board).await;
    let tick = scan_once(&mut fx, &OFFSET2).await;
    assert!(matches!(
        tick,
        PresenceTick::Scanned(ScanOutcome::Unlocked { .. })
    ));
    assert_eq!(
        fx.worker.post_scan_hold(&tick),
        Duration::ZERO,
        "a successful unlock keeps its latency (no hold)"
    );
}

/// LKS19: driving `run()`, a fast failure (spoof veto) and a slow failure (full non-match)
/// start their next scan at the same offset from the previous start (floor + one poll
/// interval), and that offset is past the reveal floor, so both show `NotRecognized` before
/// the next `Scanning`: neither the next-scan time nor the visible trace depends on the end.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_lks_fast_and_slow_presence_failures_schedule_the_next_scan_identically() {
    use soos_daemon::pipeline::current_monotonic_nanos;
    use soos_daemon::presence::LOCK_POLL_INTERVAL_MS;

    async fn second_scan_gap(fast: bool) -> (u64, Vec<LockStateView>) {
        let board = LockStateBoard::new();
        let fx = failing_worker(fast, current_monotonic_nanos, &board).await;
        let wakes: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let wakes = Arc::clone(&wakes);
            *fx.parts.camera.on_wake.lock().unwrap() = Some(Box::new(move || {
                wakes
                    .lock()
                    .unwrap()
                    .push(current_monotonic_nanos().unwrap());
            }));
        }
        let WorkerFixture {
            worker,
            logind,
            display,
            gate,
            switch_dir,
            parts,
        } = fx;
        let (stop, rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(worker.run(rx));
        // A widget polling every 25 ms: the distinct views it would render, in order, until
        // the second scan is visible.
        let mut trace: Vec<LockStateView> = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            let seen = board.view(UID, current_monotonic_nanos().unwrap());
            if trace.last() != Some(&seen) {
                trace.push(seen);
            }
            if seen.sequence >= 3 && wakes.lock().unwrap().len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let _ = stop.send(true);
        let _ = task.await;
        drop((logind, display, gate, switch_dir));
        let wakes = wakes.lock().unwrap().clone();
        assert!(wakes.len() >= 2, "fast={fast}: two scans ran");
        drop(parts);
        (wakes[1] - wakes[0], trace)
    }

    let (fast_gap, fast_views) = second_scan_gap(true).await;
    let (slow_gap, slow_views) = second_scan_gap(false).await;
    let expected = FLOOR_NS + LOCK_POLL_INTERVAL_MS * MS_NS;
    for (label, gap) in [("fast", fast_gap), ("slow", slow_gap)] {
        assert!(
            gap >= expected && gap <= expected + 400 * MS_NS,
            "{label}: next scan {gap} ns after the previous start, expected about {expected}"
        );
    }
    assert!(
        fast_gap.abs_diff(slow_gap) <= 250 * MS_NS,
        "the next-scan time must not depend on the previous end: fast {fast_gap}, slow {slow_gap}"
    );
    assert_eq!(
        fast_views,
        vec![
            view(LockWidgetState::Idle, 0),
            view(LockWidgetState::Scanning, 1),
            view(LockWidgetState::NotRecognized, 2),
            view(LockWidgetState::Scanning, 3),
        ],
        "fast: the failure is visible before the next scan"
    );
    assert_eq!(fast_views, slow_views, "identical visible trace");
}

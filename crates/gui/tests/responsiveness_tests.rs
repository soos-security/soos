//! Contractual tests for GUI responsiveness (review finding CAM-06, GitHub #154).
//!
//! Contract:
//! - Daemon service-state polling is throttled (`PollThrottle`, injectable clock) and runs on a
//!   background `DaemonMonitor` thread; reading the state from the UI never forks a process.
//! - Privileged (pkexec) work runs on a background thread through `TaskRunner`; `submit` returns
//!   immediately even while the executor blocks, the outcome comes back through a channel
//!   drained by `poll`, and a second privileged action is rejected while one is in flight so the
//!   user never gets stacked Polkit dialogs.
//! - `app.rs` (the UI thread code) spawns no process directly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use soos_gui::daemon_control::{
    DaemonMonitor, DaemonState, DaemonStatusProbe, PollThrottle, DAEMON_POLL_INTERVAL,
};
use soos_gui::privileged::{
    PrivilegedAction, PrivilegedExecutor, PrivilegedOutcome, TaskRunner, TaskRunnerError,
};

fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cond()
}

// ---------------------------------------------------------------------------
// PollThrottle
// ---------------------------------------------------------------------------

#[test]
fn test_poll_interval_is_two_seconds() {
    assert_eq!(DAEMON_POLL_INTERVAL, Duration::from_secs(2));
}

#[test]
fn test_poll_throttle_first_poll_is_due_then_waits_for_interval() {
    let t0 = Instant::now();
    let mut throttle = PollThrottle::new(Duration::from_secs(2));
    assert!(throttle.due(t0), "the first poll must happen immediately");
    assert!(!throttle.due(t0));
    assert!(!throttle.due(t0 + Duration::from_millis(1999)));
    assert!(throttle.due(t0 + Duration::from_secs(2)));
    assert!(!throttle.due(t0 + Duration::from_millis(2500)));
    assert!(throttle.due(t0 + Duration::from_secs(4)));
}

#[test]
fn test_poll_throttle_bounds_polls_under_repaint_storm() {
    // 60 repaints per second for 10 simulated seconds must yield at most 6 polls.
    let t0 = Instant::now();
    let mut throttle = PollThrottle::new(Duration::from_secs(2));
    let mut polls = 0;
    for frame in 0..600u64 {
        if throttle.due(t0 + Duration::from_micros(frame * 16_667)) {
            polls += 1;
        }
    }
    assert!(polls <= 6, "throttle allowed {polls} polls in 10 s");
    assert!(
        polls >= 5,
        "throttle starved polling: {polls} polls in 10 s"
    );
}

#[test]
fn test_poll_throttle_force_makes_next_poll_due() {
    let t0 = Instant::now();
    let mut throttle = PollThrottle::new(Duration::from_secs(2));
    assert!(throttle.due(t0));
    throttle.force();
    assert!(throttle.due(t0 + Duration::from_millis(10)));
    assert!(!throttle.due(t0 + Duration::from_millis(20)));
}

// ---------------------------------------------------------------------------
// DaemonMonitor
// ---------------------------------------------------------------------------

struct CountingProbe {
    calls: AtomicUsize,
    active: AtomicBool,
}

impl DaemonStatusProbe for CountingProbe {
    fn is_active(&self) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.active.load(Ordering::SeqCst)
    }
}

#[test]
fn test_daemon_monitor_state_reads_never_call_probe() {
    let probe = Arc::new(CountingProbe {
        calls: AtomicUsize::new(0),
        active: AtomicBool::new(true),
    });
    let monitor = DaemonMonitor::spawn(probe.clone(), Duration::from_secs(60)).unwrap();

    assert!(wait_until(Duration::from_secs(2), || monitor.state()
        == DaemonState::Active));
    let calls_before = probe.calls.load(Ordering::SeqCst);

    // Simulate a burst of repaints reading the header state.
    for _ in 0..10_000 {
        let _ = monitor.state();
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        calls_before,
        "reading the daemon state from the UI must not trigger a probe"
    );
}

#[test]
fn test_daemon_monitor_refresh_picks_up_state_change() {
    let probe = Arc::new(CountingProbe {
        calls: AtomicUsize::new(0),
        active: AtomicBool::new(true),
    });
    let monitor = DaemonMonitor::spawn(probe.clone(), Duration::from_secs(60)).unwrap();
    assert!(wait_until(Duration::from_secs(2), || monitor.state()
        == DaemonState::Active));

    probe.active.store(false, Ordering::SeqCst);
    monitor.request_refresh();
    assert!(
        wait_until(Duration::from_secs(2), || monitor.state()
            == DaemonState::Inactive),
        "request_refresh must trigger a prompt background re-poll"
    );
}

struct BlockingProbe {
    release: Mutex<mpsc::Receiver<()>>,
}

impl DaemonStatusProbe for BlockingProbe {
    fn is_active(&self) -> bool {
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        true
    }
}

#[test]
fn test_daemon_monitor_state_does_not_block_on_slow_probe() {
    let (tx, rx) = mpsc::channel();
    let probe = Arc::new(BlockingProbe {
        release: Mutex::new(rx),
    });
    let monitor = DaemonMonitor::spawn(probe, Duration::from_secs(60)).unwrap();

    let start = Instant::now();
    assert_eq!(monitor.state(), DaemonState::Unknown);
    assert!(start.elapsed() < Duration::from_millis(50));

    tx.send(()).unwrap();
    assert!(wait_until(Duration::from_secs(2), || monitor.state()
        == DaemonState::Active));
}

// ---------------------------------------------------------------------------
// TaskRunner
// ---------------------------------------------------------------------------

struct GateExecutor {
    gate: Mutex<mpsc::Receiver<()>>,
    executed: AtomicUsize,
}

impl PrivilegedExecutor for GateExecutor {
    fn execute(&self, action: PrivilegedAction) -> PrivilegedOutcome {
        let _ = self
            .gate
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        self.executed.fetch_add(1, Ordering::SeqCst);
        match action {
            PrivilegedAction::PauseDaemon => PrivilegedOutcome::DaemonPaused(Ok(())),
            PrivilegedAction::ResumeDaemon => {
                PrivilegedOutcome::DaemonResumed(Err("denied".into()))
            }
            PrivilegedAction::ListProfiles => PrivilegedOutcome::ProfilesListed(Ok(Vec::new())),
            PrivilegedAction::ImportTemplate { uid, .. } => PrivilegedOutcome::TemplateImported {
                uid,
                result: Ok(()),
            },
            PrivilegedAction::DeleteTemplate { uid } => PrivilegedOutcome::TemplateDeleted {
                uid,
                result: Ok(()),
            },
        }
    }
}

fn gated_runner() -> (
    TaskRunner,
    mpsc::Sender<()>,
    Arc<GateExecutor>,
    Arc<AtomicUsize>,
) {
    let (tx, rx) = mpsc::channel();
    let exec = Arc::new(GateExecutor {
        gate: Mutex::new(rx),
        executed: AtomicUsize::new(0),
    });
    let repaints = Arc::new(AtomicUsize::new(0));
    let repaints_clone = Arc::clone(&repaints);
    let runner = TaskRunner::new(
        exec.clone(),
        Arc::new(move || {
            repaints_clone.fetch_add(1, Ordering::SeqCst);
        }),
    );
    (runner, tx, exec, repaints)
}

#[test]
fn test_task_runner_submit_returns_while_executor_blocks() {
    let (mut runner, tx, exec, repaints) = gated_runner();

    let start = Instant::now();
    runner.submit(PrivilegedAction::PauseDaemon).unwrap();
    assert!(
        start.elapsed() < Duration::from_millis(100),
        "submit must not wait for the privileged command"
    );
    assert!(runner.is_busy());
    assert!(runner.poll().is_empty());
    assert_eq!(exec.executed.load(Ordering::SeqCst), 0);

    tx.send(()).unwrap();
    let mut outcomes = Vec::new();
    assert!(wait_until(Duration::from_secs(2), || {
        outcomes.extend(runner.poll());
        !outcomes.is_empty()
    }));
    assert!(matches!(
        outcomes.as_slice(),
        [PrivilegedOutcome::DaemonPaused(Ok(()))]
    ));
    assert!(!runner.is_busy());
    assert!(
        repaints.load(Ordering::SeqCst) >= 1,
        "the UI must be woken when the outcome arrives"
    );
}

#[test]
fn test_task_runner_rejects_concurrent_privileged_action() {
    let (mut runner, tx, _exec, _repaints) = gated_runner();
    runner.submit(PrivilegedAction::PauseDaemon).unwrap();
    assert!(matches!(
        runner.submit(PrivilegedAction::ResumeDaemon),
        Err(TaskRunnerError::Busy)
    ));

    tx.send(()).unwrap();
    assert!(wait_until(Duration::from_secs(2), || !runner
        .poll()
        .is_empty()));
    runner.submit(PrivilegedAction::ResumeDaemon).unwrap();
    tx.send(()).unwrap();
    let mut outcomes = Vec::new();
    assert!(wait_until(Duration::from_secs(2), || {
        outcomes.extend(runner.poll());
        !outcomes.is_empty()
    }));
    assert!(matches!(
        outcomes.as_slice(),
        [PrivilegedOutcome::DaemonResumed(Err(_))]
    ));
}

#[test]
fn test_import_action_debug_never_prints_embedding() {
    let action = PrivilegedAction::ImportTemplate {
        uid: 1000,
        embedding: zeroize::Zeroizing::new(vec![0.123_456_f32; 4]),
    };
    let text = format!("{action:?}");
    assert!(text.contains("1000"));
    assert!(
        !text.contains("0.123"),
        "embedding values must never reach Debug output: {text}"
    );
}

// ---------------------------------------------------------------------------
// Static guard: no process spawning on the UI thread
// ---------------------------------------------------------------------------

#[test]
fn test_app_ui_code_spawns_no_process() {
    let src = include_str!("../src/app.rs");
    assert!(
        !src.contains("Command::new"),
        "app.rs runs on the UI thread and must delegate process execution to \
         daemon_control / privileged background workers"
    );
    assert!(
        src.contains("DaemonMonitor") && src.contains("TaskRunner"),
        "app.rs must use the background DaemonMonitor and TaskRunner"
    );
}

//! Contract tests for runtime camera-source switching (GitHub #154 / #150, candid review
//! findings 1 and 5).
//!
//! Contract:
//! - The camera source is re-decided at runtime from the `DaemonMonitor` state, not once at
//!   startup: daemon active => daemon IPC preview (or a blocked notice); daemon paused =>
//!   direct V4L2 through the shared resolver.
//! - A direct V4L2 manager is released (stopped and dropped, so the device node is closed)
//!   as soon as the daemon is active, and *before* the privileged Resume runs, so the daemon
//!   never meets `EBUSY` because of the GUI.
//! - An unknown daemon state never opens the device directly (fail-closed).
//! - Transient preview failures (rate limit, I/O, unavailable, daemon still starting) are
//!   retried a bounded number of times; permanent ones (not authorized, permission denied)
//!   are re-evaluated only when the daemon state changes.
//! - The decision logic is a pure state machine (`CameraSourcePlanner`) with an injected clock
//!   and a lazily-invoked probe.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use soos_camera_v4l::{CameraManager, Frame, PixelFormat};
use soos_gui::camera_mode::{CameraBlockReason, CameraMode, DaemonSocketProbe};
use soos_gui::camera_source::{
    release_manager, CameraSourceBackend, CameraSourcePlanner, CameraSourceSupervisor,
    DaemonStateSource, HandoverExecutor, SourceProbe, SupervisorTiming, SwitchableCamera,
    MAX_TRANSIENT_PROBE_RETRIES, TRANSIENT_RETRY_INTERVAL,
};
use soos_gui::daemon_control::DaemonState;
use soos_gui::privileged::{PrivilegedAction, PrivilegedExecutor, PrivilegedOutcome};
use soos_gui::IpcPreviewError;

const NOT_RUNNING: SourceProbe = (DaemonSocketProbe::NotRunning, None);
const IPC_OK: SourceProbe = (DaemonSocketProbe::Reachable, Some(Ok(())));

fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    cond()
}

/// Runs one planner step with a probe that counts its invocations.
fn step(
    planner: &mut CameraSourcePlanner,
    now: Instant,
    daemon: DaemonState,
    handover: bool,
    probe: SourceProbe,
    probes: &Cell<usize>,
) -> Option<CameraMode> {
    planner.step(now, daemon, handover, || {
        probes.set(probes.get() + 1);
        probe
    })
}

// ---------------------------------------------------------------------------
// Pure planner
// ---------------------------------------------------------------------------

#[test]
fn test_planner_unknown_daemon_state_never_opens_direct() {
    let mut planner = CameraSourcePlanner::default();
    let probes = Cell::new(0);
    let t0 = Instant::now();
    for i in 0..5u64 {
        let now = t0 + TRANSIENT_RETRY_INTERVAL * (i as u32);
        assert_eq!(
            step(
                &mut planner,
                now,
                DaemonState::Unknown,
                false,
                NOT_RUNNING,
                &probes
            ),
            None,
            "an unknown daemon state must never select a source"
        );
    }
    assert_eq!(probes.get(), 0);
    assert_eq!(planner.current(), None);
}

#[test]
fn test_planner_initial_selection_follows_daemon_state() {
    let probes = Cell::new(0);
    let now = Instant::now();

    let mut paused = CameraSourcePlanner::default();
    assert_eq!(
        step(
            &mut paused,
            now,
            DaemonState::Inactive,
            false,
            NOT_RUNNING,
            &probes
        ),
        Some(CameraMode::DirectV4l)
    );
    assert_eq!(paused.current(), Some(CameraMode::DirectV4l));

    let mut active = CameraSourcePlanner::default();
    assert_eq!(
        step(
            &mut active,
            now,
            DaemonState::Active,
            false,
            IPC_OK,
            &probes
        ),
        Some(CameraMode::DaemonIpc)
    );
}

#[test]
fn test_planner_releases_direct_when_daemon_becomes_active() {
    let mut planner = CameraSourcePlanner::default();
    let probes = Cell::new(0);
    let t0 = Instant::now();
    assert_eq!(
        step(
            &mut planner,
            t0,
            DaemonState::Inactive,
            false,
            NOT_RUNNING,
            &probes
        ),
        Some(CameraMode::DirectV4l)
    );

    // The daemon was started (Resume or `systemctl start`) but its socket is not up yet.
    let released = step(
        &mut planner,
        t0,
        DaemonState::Active,
        false,
        NOT_RUNNING,
        &probes,
    );
    assert_eq!(
        released,
        Some(CameraMode::Blocked(CameraBlockReason::DaemonUnreachable)),
        "the direct camera must be released immediately when the daemon is active"
    );

    // Once the daemon accepts the preview, the GUI switches to the IPC source.
    let later = t0 + TRANSIENT_RETRY_INTERVAL;
    assert_eq!(
        step(
            &mut planner,
            later,
            DaemonState::Active,
            false,
            IPC_OK,
            &probes
        ),
        Some(CameraMode::DaemonIpc)
    );
}

#[test]
fn test_planner_handover_never_selects_direct() {
    let mut planner = CameraSourcePlanner::default();
    let probes = Cell::new(0);
    let t0 = Instant::now();
    step(
        &mut planner,
        t0,
        DaemonState::Inactive,
        false,
        NOT_RUNNING,
        &probes,
    );

    // Resume requested: the systemd state is still inactive, the socket is still absent.
    assert_eq!(
        step(
            &mut planner,
            t0,
            DaemonState::Inactive,
            true,
            NOT_RUNNING,
            &probes
        ),
        Some(CameraMode::Blocked(CameraBlockReason::HandingOver))
    );
    for i in 1..=10u32 {
        let now = t0 + TRANSIENT_RETRY_INTERVAL * i;
        let next = step(
            &mut planner,
            now,
            DaemonState::Inactive,
            true,
            NOT_RUNNING,
            &probes,
        );
        assert_ne!(next, Some(CameraMode::DirectV4l));
        assert_ne!(planner.current(), Some(CameraMode::DirectV4l));
    }
}

#[test]
fn test_planner_switches_ipc_to_direct_after_pause() {
    let mut planner = CameraSourcePlanner::default();
    let probes = Cell::new(0);
    let t0 = Instant::now();
    step(
        &mut planner,
        t0,
        DaemonState::Active,
        false,
        IPC_OK,
        &probes,
    );
    assert_eq!(
        step(
            &mut planner,
            t0,
            DaemonState::Inactive,
            false,
            NOT_RUNNING,
            &probes
        ),
        Some(CameraMode::DirectV4l),
        "after Pause the GUI must switch to the direct camera instead of a blank feed"
    );
}

#[test]
fn test_planner_keeps_steady_sources_without_probing() {
    let t0 = Instant::now();

    let mut ipc = CameraSourcePlanner::default();
    let probes = Cell::new(0);
    step(&mut ipc, t0, DaemonState::Active, false, IPC_OK, &probes);
    for i in 1..=20u32 {
        let now = t0 + TRANSIENT_RETRY_INTERVAL * i;
        assert_eq!(
            step(&mut ipc, now, DaemonState::Active, false, IPC_OK, &probes),
            None
        );
    }
    assert_eq!(probes.get(), 1, "a steady IPC source must not be re-probed");

    let mut direct = CameraSourcePlanner::default();
    let probes = Cell::new(0);
    step(
        &mut direct,
        t0,
        DaemonState::Inactive,
        false,
        NOT_RUNNING,
        &probes,
    );
    for i in 1..=20u32 {
        let now = t0 + TRANSIENT_RETRY_INTERVAL * i;
        assert_eq!(
            step(
                &mut direct,
                now,
                DaemonState::Inactive,
                false,
                NOT_RUNNING,
                &probes
            ),
            None
        );
    }
    assert_eq!(
        probes.get(),
        1,
        "a steady direct source must not be re-probed"
    );
}

#[test]
fn test_planner_retries_transient_preview_failure_until_ipc() {
    let mut planner = CameraSourcePlanner::default();
    let probes = Cell::new(0);
    let t0 = Instant::now();
    let rate_limited: SourceProbe = (
        DaemonSocketProbe::Reachable,
        Some(Err(IpcPreviewError::RateLimited)),
    );
    assert_eq!(
        step(
            &mut planner,
            t0,
            DaemonState::Active,
            false,
            rate_limited,
            &probes
        ),
        Some(CameraMode::Blocked(CameraBlockReason::PreviewUnavailable(
            IpcPreviewError::RateLimited
        )))
    );
    // Not due yet: no probe.
    assert_eq!(
        step(
            &mut planner,
            t0,
            DaemonState::Active,
            false,
            IPC_OK,
            &probes
        ),
        None
    );
    assert_eq!(probes.get(), 1);
    // Due: a transient startup failure no longer blocks the GUI until restart.
    assert_eq!(
        step(
            &mut planner,
            t0 + TRANSIENT_RETRY_INTERVAL,
            DaemonState::Active,
            false,
            IPC_OK,
            &probes
        ),
        Some(CameraMode::DaemonIpc)
    );
}

#[test]
fn test_planner_transient_retries_are_bounded() {
    let mut planner = CameraSourcePlanner::default();
    let probes = Cell::new(0);
    let t0 = Instant::now();
    let io: SourceProbe = (DaemonSocketProbe::Reachable, Some(Err(IpcPreviewError::Io)));
    for i in 0..(MAX_TRANSIENT_PROBE_RETRIES + 20) {
        let now = t0 + TRANSIENT_RETRY_INTERVAL * i;
        step(&mut planner, now, DaemonState::Active, false, io, &probes);
    }
    assert_eq!(
        probes.get(),
        1 + MAX_TRANSIENT_PROBE_RETRIES as usize,
        "one initial probe plus at most MAX_TRANSIENT_PROBE_RETRIES retries"
    );

    // A daemon state change re-arms the evaluation.
    let later = t0 + TRANSIENT_RETRY_INTERVAL * (MAX_TRANSIENT_PROBE_RETRIES + 21);
    assert_eq!(
        step(
            &mut planner,
            later,
            DaemonState::Inactive,
            false,
            NOT_RUNNING,
            &probes
        ),
        Some(CameraMode::DirectV4l)
    );
}

#[test]
fn test_planner_permanent_block_not_retried_until_state_change() {
    let mut planner = CameraSourcePlanner::default();
    let probes = Cell::new(0);
    let t0 = Instant::now();
    let unauthorized: SourceProbe = (
        DaemonSocketProbe::Reachable,
        Some(Err(IpcPreviewError::Unauthorized)),
    );
    step(
        &mut planner,
        t0,
        DaemonState::Active,
        false,
        unauthorized,
        &probes,
    );
    for i in 1..=10u32 {
        let now = t0 + TRANSIENT_RETRY_INTERVAL * i;
        assert_eq!(
            step(
                &mut planner,
                now,
                DaemonState::Active,
                false,
                IPC_OK,
                &probes
            ),
            None
        );
    }
    assert_eq!(probes.get(), 1, "an authorization refusal is not hammered");

    let later = t0 + TRANSIENT_RETRY_INTERVAL * 11;
    assert_eq!(
        step(
            &mut planner,
            later,
            DaemonState::Inactive,
            false,
            NOT_RUNNING,
            &probes
        ),
        Some(CameraMode::DirectV4l)
    );
}

#[test]
fn test_block_reason_transience_classification() {
    for reason in [
        CameraBlockReason::DaemonUnreachable,
        CameraBlockReason::HandingOver,
        CameraBlockReason::DirectOpenFailed,
        CameraBlockReason::PreviewUnavailable(IpcPreviewError::RateLimited),
        CameraBlockReason::PreviewUnavailable(IpcPreviewError::Unavailable),
        CameraBlockReason::PreviewUnavailable(IpcPreviewError::Io),
    ] {
        assert!(reason.is_transient(), "{reason:?} must be retried");
    }
    for reason in [
        CameraBlockReason::PermissionDenied,
        CameraBlockReason::PreviewUnavailable(IpcPreviewError::Unauthorized),
        CameraBlockReason::PreviewUnavailable(IpcPreviewError::Protocol),
    ] {
        assert!(!reason.is_transient(), "{reason:?} must not be retried");
    }
}

#[test]
fn test_block_messages_state_whether_retry_happens() {
    let transient =
        CameraBlockReason::PreviewUnavailable(IpcPreviewError::RateLimited).user_message();
    assert!(transient.contains("retries automatically"), "{transient}");
    let unauthorized =
        CameraBlockReason::PreviewUnavailable(IpcPreviewError::Unauthorized).user_message();
    assert!(
        !unauthorized.contains("retries automatically"),
        "{unauthorized}"
    );
    let handover = CameraBlockReason::HandingOver.user_message();
    assert!(handover.contains("soos-daemon"), "{handover}");
    let unreachable = CameraBlockReason::DaemonUnreachable.user_message();
    assert!(
        !unreachable.contains("restart soos-gui"),
        "the GUI now switches sources by itself: {unreachable}"
    );
}

// ---------------------------------------------------------------------------
// Switchable camera and release
// ---------------------------------------------------------------------------

/// Fake camera manager recording `stop` and `Drop`.
struct FakeCamera {
    tag: u8,
    stopped: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl FakeCamera {
    fn new(tag: u8) -> (Arc<Self>, Arc<AtomicBool>, Arc<AtomicBool>) {
        let stopped = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        (
            Arc::new(Self {
                tag,
                stopped: Arc::clone(&stopped),
                dropped: Arc::clone(&dropped),
            }),
            stopped,
            dropped,
        )
    }
}

impl CameraManager for FakeCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        Some(Arc::new(Frame::new(
            vec![self.tag; 6],
            2,
            1,
            0,
            PixelFormat::Rgb24,
            u64::from(self.tag),
        )))
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn notify_activity(&self) {}

    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

impl Drop for FakeCamera {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[test]
fn test_switchable_camera_delegates_to_current_source_and_bumps_generation() {
    let switch = SwitchableCamera::new();
    assert!(!switch.is_ready());
    assert!(switch.latest_frame().is_none());
    assert_eq!(switch.mode(), None);
    let g0 = switch.generation();

    let (first, _, _) = FakeCamera::new(1);
    let _old = switch.replace(first, Some(CameraMode::DirectV4l), None);
    assert!(switch.generation() > g0);
    assert!(switch.is_ready());
    assert_eq!(switch.latest_frame().unwrap().data, vec![1u8; 6]);
    assert_eq!(switch.mode(), Some(CameraMode::DirectV4l));
    assert_eq!(switch.notice(), None);

    let (second, _, _) = FakeCamera::new(2);
    let g1 = switch.generation();
    let _old = switch.replace(
        second,
        Some(CameraMode::Blocked(CameraBlockReason::HandingOver)),
        Some("notice".to_string()),
    );
    assert!(switch.generation() > g1);
    assert_eq!(switch.latest_frame().unwrap().data, vec![2u8; 6]);
    assert_eq!(switch.notice().as_deref(), Some("notice"));
}

#[test]
fn test_release_manager_stops_and_drops_the_last_reference() {
    let (cam, stopped, dropped) = FakeCamera::new(3);
    let as_dyn: Arc<dyn CameraManager> = cam;
    assert!(release_manager(as_dyn, Duration::from_secs(1)));
    assert!(stopped.load(Ordering::SeqCst));
    assert!(
        dropped.load(Ordering::SeqCst),
        "dropping frees the device node"
    );
}

// ---------------------------------------------------------------------------
// Supervisor and privileged handover
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FakeDaemon(Arc<Mutex<DaemonState>>);

impl FakeDaemon {
    fn new(state: DaemonState) -> Self {
        Self(Arc::new(Mutex::new(state)))
    }
    fn set(&self, state: DaemonState) {
        *self.0.lock().unwrap() = state;
    }
}

impl DaemonStateSource for FakeDaemon {
    fn state(&self) -> DaemonState {
        *self.0.lock().unwrap()
    }
}

struct FakeBackend {
    probe: Mutex<SourceProbe>,
    direct_opened: AtomicUsize,
    ipc_opened: AtomicUsize,
    direct_dropped: Mutex<Vec<Arc<AtomicBool>>>,
}

impl FakeBackend {
    fn new(probe: SourceProbe) -> Arc<Self> {
        Arc::new(Self {
            probe: Mutex::new(probe),
            direct_opened: AtomicUsize::new(0),
            ipc_opened: AtomicUsize::new(0),
            direct_dropped: Mutex::new(Vec::new()),
        })
    }
    fn set_probe(&self, probe: SourceProbe) {
        *self.probe.lock().unwrap() = probe;
    }
    fn all_direct_dropped(&self) -> bool {
        self.direct_dropped
            .lock()
            .unwrap()
            .iter()
            .all(|d| d.load(Ordering::SeqCst))
    }
}

impl CameraSourceBackend for FakeBackend {
    fn probe(&self) -> SourceProbe {
        *self.probe.lock().unwrap()
    }

    fn open_daemon_ipc(&self) -> Arc<dyn CameraManager> {
        self.ipc_opened.fetch_add(1, Ordering::SeqCst);
        let (cam, _, _) = FakeCamera::new(20);
        cam
    }

    fn open_direct(&self) -> Result<Arc<dyn CameraManager>, String> {
        self.direct_opened.fetch_add(1, Ordering::SeqCst);
        let (cam, _, dropped) = FakeCamera::new(10);
        self.direct_dropped.lock().unwrap().push(dropped);
        Ok(cam)
    }
}

fn fast_timing() -> SupervisorTiming {
    SupervisorTiming {
        tick: Duration::from_millis(2),
        retry_interval: Duration::from_millis(10),
        handover_grace: Duration::from_millis(200),
        release_timeout: Duration::from_secs(3),
    }
}

#[test]
fn test_supervisor_switches_sources_on_daemon_state_changes() {
    let switch = Arc::new(SwitchableCamera::new());
    let backend = FakeBackend::new(NOT_RUNNING);
    let daemon = FakeDaemon::new(DaemonState::Inactive);
    let _supervisor = CameraSourceSupervisor::spawn_with_timing(
        Arc::clone(&switch),
        backend.clone(),
        Arc::new(daemon.clone()),
        fast_timing(),
    )
    .unwrap();

    assert!(
        wait_until(Duration::from_secs(5), || switch.mode()
            == Some(CameraMode::DirectV4l)),
        "paused daemon => direct V4L2"
    );

    backend.set_probe(IPC_OK);
    daemon.set(DaemonState::Active);
    assert!(
        wait_until(Duration::from_secs(5), || switch.mode()
            == Some(CameraMode::DaemonIpc)),
        "active daemon => IPC preview"
    );
    assert!(
        backend.all_direct_dropped(),
        "the direct manager must be dropped (device freed) when the daemon is active"
    );

    backend.set_probe(NOT_RUNNING);
    daemon.set(DaemonState::Inactive);
    assert!(
        wait_until(Duration::from_secs(5), || switch.mode()
            == Some(CameraMode::DirectV4l)),
        "paused again => direct V4L2 through the resolver"
    );
    assert_eq!(backend.direct_opened.load(Ordering::SeqCst), 2);
    assert_eq!(backend.ipc_opened.load(Ordering::SeqCst), 1);
}

/// Inner executor recording whether every direct manager was already dropped at Resume time.
struct RecordingExecutor {
    backend: Arc<FakeBackend>,
    released_at_resume: Mutex<Option<bool>>,
    resume_result: Result<(), String>,
}

impl PrivilegedExecutor for RecordingExecutor {
    fn execute(&self, action: PrivilegedAction) -> PrivilegedOutcome {
        match action {
            PrivilegedAction::ResumeDaemon => {
                *self.released_at_resume.lock().unwrap() = Some(self.backend.all_direct_dropped());
                PrivilegedOutcome::DaemonResumed(self.resume_result.clone())
            }
            PrivilegedAction::PauseDaemon => PrivilegedOutcome::DaemonPaused(Ok(())),
            other => panic!("unexpected action {other:?}"),
        }
    }
}

fn direct_setup() -> (
    Arc<SwitchableCamera>,
    Arc<FakeBackend>,
    FakeDaemon,
    CameraSourceSupervisor,
) {
    let switch = Arc::new(SwitchableCamera::new());
    let backend = FakeBackend::new(NOT_RUNNING);
    let daemon = FakeDaemon::new(DaemonState::Inactive);
    let supervisor = CameraSourceSupervisor::spawn_with_timing(
        Arc::clone(&switch),
        backend.clone(),
        Arc::new(daemon.clone()),
        fast_timing(),
    )
    .unwrap();
    assert!(wait_until(Duration::from_secs(5), || switch.mode()
        == Some(CameraMode::DirectV4l)));
    (switch, backend, daemon, supervisor)
}

#[test]
fn test_handover_releases_direct_camera_before_resume_executes() {
    let (switch, backend, _daemon, supervisor) = direct_setup();
    let inner = Arc::new(RecordingExecutor {
        backend: backend.clone(),
        released_at_resume: Mutex::new(None),
        resume_result: Ok(()),
    });
    let slot = Arc::new(OnceLock::new());
    assert!(slot.set(supervisor.handover()).is_ok());
    let executor = HandoverExecutor::new(inner.clone(), slot);

    let outcome = executor.execute(PrivilegedAction::ResumeDaemon);
    assert!(matches!(outcome, PrivilegedOutcome::DaemonResumed(Ok(()))));
    assert_eq!(
        *inner.released_at_resume.lock().unwrap(),
        Some(true),
        "the direct V4L2 manager must be dropped before pkexec starts the daemon"
    );
    assert_ne!(switch.mode(), Some(CameraMode::DirectV4l));

    // The daemon state is not yet observed as active: the device stays released.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(backend.direct_opened.load(Ordering::SeqCst), 1);
    assert_ne!(switch.mode(), Some(CameraMode::DirectV4l));
}

#[test]
fn test_failed_resume_lets_the_gui_reopen_the_direct_camera() {
    let (switch, backend, _daemon, supervisor) = direct_setup();
    let inner = Arc::new(RecordingExecutor {
        backend: backend.clone(),
        released_at_resume: Mutex::new(None),
        resume_result: Err("authorization denied".to_string()),
    });
    let slot = Arc::new(OnceLock::new());
    assert!(slot.set(supervisor.handover()).is_ok());
    let executor = HandoverExecutor::new(inner, slot);

    let outcome = executor.execute(PrivilegedAction::ResumeDaemon);
    assert!(matches!(outcome, PrivilegedOutcome::DaemonResumed(Err(_))));
    assert!(
        wait_until(Duration::from_secs(5), || backend
            .direct_opened
            .load(Ordering::SeqCst)
            == 2
            && switch.mode() == Some(CameraMode::DirectV4l)),
        "the daemon did not start: the GUI goes back to the direct camera"
    );
}

#[test]
fn test_handover_executor_passes_other_actions_through_without_release() {
    let (switch, backend, _daemon, supervisor) = direct_setup();
    let inner = Arc::new(RecordingExecutor {
        backend: backend.clone(),
        released_at_resume: Mutex::new(None),
        resume_result: Ok(()),
    });
    let slot = Arc::new(OnceLock::new());
    assert!(slot.set(supervisor.handover()).is_ok());
    let executor = HandoverExecutor::new(inner, slot);
    assert!(matches!(
        executor.execute(PrivilegedAction::PauseDaemon),
        PrivilegedOutcome::DaemonPaused(Ok(()))
    ));
    assert_eq!(switch.mode(), Some(CameraMode::DirectV4l));
    assert!(!backend.all_direct_dropped());
}

#[test]
fn test_handover_executor_without_camera_source_still_resumes() {
    let backend = FakeBackend::new(NOT_RUNNING);
    let inner = Arc::new(RecordingExecutor {
        backend,
        released_at_resume: Mutex::new(None),
        resume_result: Ok(()),
    });
    let executor = HandoverExecutor::new(inner, Arc::new(OnceLock::new()));
    assert!(matches!(
        executor.execute(PrivilegedAction::ResumeDaemon),
        PrivilegedOutcome::DaemonResumed(Ok(()))
    ));
}

// ---------------------------------------------------------------------------
// Static wiring guards
// ---------------------------------------------------------------------------

#[test]
fn test_gui_main_selects_camera_source_at_runtime() {
    let main = include_str!("../src/main.rs");
    assert!(
        !main.contains("decide_camera_mode("),
        "main.rs must not freeze the camera source with a one-shot startup decision"
    );
    assert!(
        main.contains("attach_camera_source"),
        "main.rs must hand the camera source to the runtime supervisor"
    );
    let app = include_str!("../src/app.rs");
    assert!(app.contains("CameraSourceSupervisor") && app.contains("HandoverExecutor"));
}

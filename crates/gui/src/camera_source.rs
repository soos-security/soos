//! Runtime camera-source switching for `soos-gui` (GitHub #154 / #150, candid review
//! findings 1 and 5).
//!
//! The camera source used to be chosen once at startup. Pausing the daemon then left the IPC
//! feed blank until restart, and resuming it while the GUI streamed directly made the daemon
//! fail with `EBUSY` (face unlock silently fell back to the password). The source is now
//! re-decided at runtime:
//!
//! - [`CameraSourcePlanner`] is a pure state machine (injected clock, lazily-invoked probe)
//!   deciding which [`CameraMode`] to use from the [`DaemonState`] published by the background
//!   `DaemonMonitor` and from a daemon socket/preview probe.
//! - [`SwitchableCamera`] is the [`CameraManager`] handed to the vision worker; it delegates to
//!   the current source, which can be replaced at any time.
//! - [`CameraSourceSupervisor`] runs the planner on its own thread and applies its decisions.
//!   A replaced source is stopped and dropped (see [`release_manager`]) **before** the next one
//!   is opened, so a direct V4L2 manager has closed `/dev/video*` by the time the daemon needs it.
//! - [`HandoverExecutor`] wraps the privileged executor: before `ResumeDaemon` runs it releases
//!   any direct V4L2 manager and keeps direct mode disabled until the Resume outcome is known.
//!
//! Every blocking operation (probes, joins, `pkexec`) runs on the supervisor or privileged
//! worker threads, never on the UI thread.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use soos_camera_v4l::{CameraConfigBuilder, CameraHealth, CameraManager, CameraStatus, Frame};

use crate::camera_mode::{
    decide_camera_mode, probe_daemon_socket, CameraBlockReason, CameraMode, DaemonSocketProbe,
    UnavailableCameraManager,
};
use crate::daemon_control::{DaemonState, DaemonStateReader};
use crate::ipc_camera::{IpcCameraManager, IpcPreviewError};
use crate::privileged::{PrivilegedAction, PrivilegedExecutor, PrivilegedOutcome};

/// Interval between two re-probes of a transiently blocked (or changing) source.
pub const TRANSIENT_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum number of re-probes of a transient block before waiting for a daemon state change.
pub const MAX_TRANSIENT_PROBE_RETRIES: u32 = 60;

/// Outcome of one daemon probe: socket reachability and, when reachable, the preview
/// authorization round-trip.
pub type SourceProbe = (DaemonSocketProbe, Option<Result<(), IpcPreviewError>>);

/// Decides the camera mode for a probe result.
///
/// `daemon_active` must already include a pending handover. During a handover the device is
/// reserved for the starting daemon: a missing socket yields [`CameraBlockReason::HandingOver`]
/// and direct mode is never selected.
pub fn decide_source(probe: SourceProbe, daemon_active: bool, handover: bool) -> CameraMode {
    let (socket, preview) = probe;
    if handover
        && matches!(
            socket,
            DaemonSocketProbe::NotRunning | DaemonSocketProbe::Other
        )
    {
        return CameraMode::Blocked(CameraBlockReason::HandingOver);
    }
    decide_camera_mode(socket, preview, daemon_active || handover)
}

/// Pure camera-source state machine.
///
/// [`CameraSourcePlanner::step`] returns `Some(mode)` when the source must be switched to
/// `mode`, `None` to keep the current one. Rules:
/// - an unknown daemon state never selects a source (fail-closed, the device is not touched);
/// - a direct source is left as soon as the daemon is active or a handover is pending (the
///   decision then never yields [`CameraMode::DirectV4l`]);
/// - an IPC source is re-evaluated only while the daemon is not active;
/// - a steady source (IPC while active, direct while inactive) is never re-probed;
/// - a transient block is re-probed every `retry_interval`, at most `max_retries` times;
///   a permanent block only when the daemon state or the handover flag changes.
#[derive(Debug, Clone)]
pub struct CameraSourcePlanner {
    current: Option<CameraMode>,
    last_daemon: DaemonState,
    last_handover: bool,
    transient_retries: u32,
    next_probe: Option<Instant>,
    retry_interval: Duration,
    max_retries: u32,
}

impl Default for CameraSourcePlanner {
    fn default() -> Self {
        Self::new(TRANSIENT_RETRY_INTERVAL, MAX_TRANSIENT_PROBE_RETRIES)
    }
}

impl CameraSourcePlanner {
    /// Creates a planner with no current source.
    pub fn new(retry_interval: Duration, max_retries: u32) -> Self {
        Self {
            current: None,
            last_daemon: DaemonState::Unknown,
            last_handover: false,
            transient_retries: 0,
            next_probe: None,
            retry_interval,
            max_retries,
        }
    }

    /// Returns the source the planner believes is installed.
    pub fn current(&self) -> Option<CameraMode> {
        self.current
    }

    /// Records that `mode` was installed outside [`CameraSourcePlanner::step`] (e.g. a failed
    /// direct open, or a handover release).
    pub fn force(&mut self, mode: CameraMode) {
        if !matches!(self.current, Some(CameraMode::Blocked(_))) {
            self.transient_retries = 0;
        }
        self.current = Some(mode);
    }

    /// Advances the state machine. `probe` is invoked at most once, only when a decision is due.
    pub fn step<P>(
        &mut self,
        now: Instant,
        daemon: DaemonState,
        handover: bool,
        probe: P,
    ) -> Option<CameraMode>
    where
        P: FnOnce() -> SourceProbe,
    {
        let changed = daemon != self.last_daemon || handover != self.last_handover;
        self.last_daemon = daemon;
        self.last_handover = handover;
        if changed {
            self.transient_retries = 0;
            self.next_probe = None;
        }
        if daemon == DaemonState::Unknown && !handover {
            return None;
        }

        let daemon_active = handover || daemon == DaemonState::Active;
        let due = changed || self.next_probe.is_none_or(|at| now >= at);
        let retry_left = self.transient_retries < self.max_retries;
        let should_probe = match self.current {
            None => due,
            Some(CameraMode::DirectV4l) => daemon_active,
            Some(CameraMode::DaemonIpc) => !daemon_active && due,
            Some(CameraMode::Blocked(reason)) => {
                changed || (reason.is_transient() && retry_left && due)
            }
        };
        if !should_probe {
            return None;
        }
        if !changed && matches!(self.current, Some(CameraMode::Blocked(_))) {
            self.transient_retries = self.transient_retries.saturating_add(1);
        }
        self.next_probe = now.checked_add(self.retry_interval);

        let target = decide_source(probe(), daemon_active, handover);
        if self.current == Some(target) {
            return None;
        }
        self.force(target);
        Some(target)
    }
}

/// Stops `manager` and drops it once this is the last reference, so its capture thread is
/// joined and the device node closed before the caller continues.
///
/// Waits at most `timeout` for transient clones (a vision-worker call in flight) to go away.
/// Returns `false` when another reference was still alive at the deadline (the manager is
/// then dropped by its last holder).
pub fn release_manager(manager: Arc<dyn CameraManager>, timeout: Duration) -> bool {
    manager.stop();
    let deadline = Instant::now().checked_add(timeout);
    while Arc::strong_count(&manager) > 1 {
        if deadline.is_none_or(|d| Instant::now() >= d) {
            tracing::warn!("camera source still referenced at release deadline");
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
    drop(manager);
    true
}

/// Currently installed camera source.
struct ActiveSource {
    manager: Arc<dyn CameraManager>,
    mode: Option<CameraMode>,
    notice: Option<String>,
}

/// [`CameraManager`] delegating to a camera source that can be replaced at runtime.
pub struct SwitchableCamera {
    active: RwLock<ActiveSource>,
    generation: AtomicU64,
}

impl Default for SwitchableCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl SwitchableCamera {
    /// Creates a switchable camera holding an [`UnavailableCameraManager`] (no source yet).
    pub fn new() -> Self {
        Self {
            active: RwLock::new(ActiveSource {
                manager: Arc::new(UnavailableCameraManager),
                mode: None,
                notice: None,
            }),
            generation: AtomicU64::new(0),
        }
    }

    fn current(&self) -> Arc<dyn CameraManager> {
        let guard = self.active.read().unwrap_or_else(|e| e.into_inner());
        Arc::clone(&guard.manager)
    }

    /// Installs `manager` as the current source and returns the previous one (the caller
    /// releases it, see [`release_manager`]). Bumps [`SwitchableCamera::generation`].
    pub fn replace(
        &self,
        manager: Arc<dyn CameraManager>,
        mode: Option<CameraMode>,
        notice: Option<String>,
    ) -> Arc<dyn CameraManager> {
        let previous = {
            let mut guard = self.active.write().unwrap_or_else(|e| e.into_inner());
            guard.mode = mode;
            guard.notice = notice;
            std::mem::replace(&mut guard.manager, manager)
        };
        self.generation.fetch_add(1, Ordering::AcqRel);
        previous
    }

    /// Monotonic counter incremented on every source replacement (the UI drops stale frames).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Mode of the installed source (`None` before the first decision).
    pub fn mode(&self) -> Option<CameraMode> {
        self.active.read().unwrap_or_else(|e| e.into_inner()).mode
    }

    /// User-facing notice explaining why no camera is shown, if any.
    pub fn notice(&self) -> Option<String> {
        self.active
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .notice
            .clone()
    }
}

impl CameraManager for SwitchableCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        self.current().latest_frame()
    }

    fn is_ready(&self) -> bool {
        self.current().is_ready()
    }

    fn notify_activity(&self) {
        self.current().notify_activity();
    }

    fn stop(&self) {
        self.current().stop();
    }

    fn health(&self) -> CameraHealth {
        self.current().health()
    }

    fn status(&self) -> CameraStatus {
        self.current().status()
    }
}

/// Opens the concrete camera sources (injectable for tests).
pub trait CameraSourceBackend: Send + Sync {
    /// Probes the daemon socket and, when reachable, the preview authorization. May block.
    fn probe(&self) -> SourceProbe;
    /// Opens the daemon IPC preview source.
    fn open_daemon_ipc(&self) -> Arc<dyn CameraManager>;
    /// Resolves the device with the shared resolver and opens it directly.
    ///
    /// # Errors
    ///
    /// Returns a displayable reason when the capture cannot be started.
    fn open_direct(&self) -> Result<Arc<dyn CameraManager>, String>;
}

/// Production backend: `/run/soos/daemon.sock` IPC and direct V4L2 via the shared resolver.
#[derive(Debug, Clone)]
pub struct SystemCameraSourceBackend {
    socket_path: PathBuf,
    cli_device: Option<PathBuf>,
    daemon_config: PathBuf,
}

impl SystemCameraSourceBackend {
    /// Creates the backend. `cli_device` is the `--camera-device` override (if any);
    /// `daemon_config` is `/etc/soos/daemon.toml` in production.
    pub fn new(socket_path: PathBuf, cli_device: Option<PathBuf>, daemon_config: PathBuf) -> Self {
        Self {
            socket_path,
            cli_device,
            daemon_config,
        }
    }
}

impl CameraSourceBackend for SystemCameraSourceBackend {
    fn probe(&self) -> SourceProbe {
        let socket = probe_daemon_socket(&self.socket_path);
        let preview = (socket == DaemonSocketProbe::Reachable)
            .then(|| IpcCameraManager::probe_preview(&self.socket_path));
        (socket, preview)
    }

    fn open_daemon_ipc(&self) -> Arc<dyn CameraManager> {
        tracing::info!(
            "Streaming the camera preview from soos-daemon at '{}'",
            self.socket_path.display()
        );
        Arc::new(IpcCameraManager::spawn(&self.socket_path))
    }

    fn open_direct(&self) -> Result<Arc<dyn CameraManager>, String> {
        // Same shared resolver and daemon.toml reader as soos-daemon, soos-enroll and
        // soos-admin (GitHub #152, #289); the reader's notes name keys, never values.
        let choice = soos_enrollment_cli::service::resolve_camera_device_from_config_reported(
            self.cli_device.clone(),
            Some(self.daemon_config.as_path()),
            &soos_camera_v4l::SystemCameraEnumerator::default(),
        );
        for note in &choice.notes {
            tracing::warn!("{note}");
        }
        let device_path = choice.path;
        // `[pipeline] allow_virtual_camera` (GitHub #307): off unless the file says `true`.
        let allow_virtual =
            soos_camera_v4l::daemon_config::read_daemon_camera_config(&self.daemon_config)
                .is_ok_and(|config| config.allow_virtual_camera);
        let config = CameraConfigBuilder::new()
            .device_path(device_path.clone())
            .warmup_frames(0)
            .idle_timeout(Duration::ZERO)
            .allow_virtual_device(allow_virtual)
            .build();
        tracing::info!(
            "soos-daemon is not running; opening direct V4L2 camera device '{}'",
            device_path.display()
        );
        soos_camera_v4l::V4lCameraManager::spawn(config)
            .map(|m| Arc::new(m) as Arc<dyn CameraManager>)
            .map_err(|e| e.to_string())
    }
}

/// Source of the daemon service state (the `DaemonMonitor` in production).
pub trait DaemonStateSource: Send + Sync {
    /// Returns the last known state. Must not block.
    fn state(&self) -> DaemonState;
}

impl DaemonStateSource for DaemonStateReader {
    fn state(&self) -> DaemonState {
        DaemonStateReader::state(self)
    }
}

/// Timing parameters of the [`CameraSourceSupervisor`].
#[derive(Debug, Clone, Copy)]
pub struct SupervisorTiming {
    /// Planner evaluation period.
    pub tick: Duration,
    /// Re-probe interval for transient states.
    pub retry_interval: Duration,
    /// How long direct mode stays disabled after a successful Resume while the daemon is not
    /// yet reported active.
    pub handover_grace: Duration,
    /// Maximum wait for a replaced source to lose its last transient reference.
    pub release_timeout: Duration,
}

impl Default for SupervisorTiming {
    fn default() -> Self {
        Self {
            tick: Duration::from_millis(100),
            retry_interval: TRANSIENT_RETRY_INTERVAL,
            handover_grace: Duration::from_secs(10),
            release_timeout: Duration::from_secs(3),
        }
    }
}

/// Handover of the camera to a starting daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handover {
    /// No Resume in progress.
    Idle,
    /// A Resume is running (Polkit dialog open, `systemctl start` in flight).
    Pending,
    /// The Resume succeeded; direct mode stays disabled until the daemon is seen active or
    /// the grace deadline passes.
    Started { until: Instant },
}

struct SupervisorShared {
    camera: Arc<SwitchableCamera>,
    backend: Arc<dyn CameraSourceBackend>,
    daemon: Arc<dyn DaemonStateSource>,
    planner: Mutex<CameraSourcePlanner>,
    handover: Mutex<Handover>,
    running: AtomicBool,
    timing: SupervisorTiming,
}

impl SupervisorShared {
    /// Returns whether direct mode is reserved for the daemon, expiring finished handovers.
    fn handover_active(&self, now: Instant, daemon: DaemonState) -> bool {
        let mut guard = self.handover.lock().unwrap_or_else(|e| e.into_inner());
        match *guard {
            Handover::Idle => false,
            Handover::Pending => true,
            Handover::Started { until } => {
                if daemon == DaemonState::Active || now >= until {
                    *guard = Handover::Idle;
                    false
                } else {
                    true
                }
            }
        }
    }

    /// Replaces the current source with `mode`, releasing the previous source first.
    fn apply(&self, planner: &mut CameraSourcePlanner, mode: CameraMode) {
        let previous = self
            .camera
            .replace(Arc::new(UnavailableCameraManager), None, None);
        release_manager(previous, self.timing.release_timeout);

        let (manager, installed, notice): (Arc<dyn CameraManager>, CameraMode, Option<String>) =
            match mode {
                CameraMode::DaemonIpc => (self.backend.open_daemon_ipc(), mode, None),
                CameraMode::DirectV4l => match self.backend.open_direct() {
                    Ok(manager) => (manager, mode, None),
                    Err(e) => {
                        tracing::warn!("direct camera capture could not be started: {e}");
                        let reason = CameraBlockReason::DirectOpenFailed;
                        planner.force(CameraMode::Blocked(reason));
                        (
                            Arc::new(UnavailableCameraManager),
                            CameraMode::Blocked(reason),
                            Some(reason.user_message()),
                        )
                    }
                },
                CameraMode::Blocked(reason) => {
                    let message = reason.user_message();
                    tracing::warn!(reason = ?reason, "{message}");
                    (Arc::new(UnavailableCameraManager), mode, Some(message))
                }
            };
        tracing::info!(mode = ?installed, "camera source switched");
        let _placeholder = self.camera.replace(manager, Some(installed), notice);
    }

    fn tick(&self) {
        let mut planner = self.planner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let daemon = self.daemon.state();
        let handover = self.handover_active(now, daemon);
        let backend = Arc::clone(&self.backend);
        if let Some(mode) = planner.step(now, daemon, handover, || backend.probe()) {
            self.apply(&mut planner, mode);
        }
    }
}

/// Background thread applying [`CameraSourcePlanner`] decisions to a [`SwitchableCamera`].
pub struct CameraSourceSupervisor {
    shared: Arc<SupervisorShared>,
    handle: Option<JoinHandle<()>>,
}

impl CameraSourceSupervisor {
    /// Spawns the supervisor with the production timing.
    ///
    /// # Errors
    ///
    /// Returns the OS error if the thread cannot be spawned.
    pub fn spawn(
        camera: Arc<SwitchableCamera>,
        backend: Arc<dyn CameraSourceBackend>,
        daemon: Arc<dyn DaemonStateSource>,
    ) -> std::io::Result<Self> {
        Self::spawn_with_timing(camera, backend, daemon, SupervisorTiming::default())
    }

    /// Spawns the supervisor with explicit timing (tests).
    ///
    /// # Errors
    ///
    /// Returns the OS error if the thread cannot be spawned.
    pub fn spawn_with_timing(
        camera: Arc<SwitchableCamera>,
        backend: Arc<dyn CameraSourceBackend>,
        daemon: Arc<dyn DaemonStateSource>,
        timing: SupervisorTiming,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(SupervisorShared {
            camera,
            backend,
            daemon,
            planner: Mutex::new(CameraSourcePlanner::new(
                timing.retry_interval,
                MAX_TRANSIENT_PROBE_RETRIES,
            )),
            handover: Mutex::new(Handover::Idle),
            running: AtomicBool::new(true),
            timing,
        });
        let thread_shared = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name("soos-gui-camera-source".to_string())
            .spawn(move || {
                while thread_shared.running.load(Ordering::Acquire) {
                    thread_shared.tick();
                    thread::sleep(thread_shared.timing.tick);
                }
            })?;
        Ok(Self {
            shared,
            handle: Some(handle),
        })
    }

    /// Returns the handover handle used by [`HandoverExecutor`].
    pub fn handover(&self) -> CameraHandover {
        CameraHandover {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for CameraSourceSupervisor {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let previous = self
            .shared
            .camera
            .replace(Arc::new(UnavailableCameraManager), None, None);
        release_manager(previous, self.shared.timing.release_timeout);
    }
}

/// Handle reserving the camera for a daemon that is being resumed.
#[derive(Clone)]
pub struct CameraHandover {
    shared: Arc<SupervisorShared>,
}

impl CameraHandover {
    /// Disables direct mode and releases any direct V4L2 manager. Blocks until the device is
    /// closed (bounded by the release timeout); call from a background thread.
    pub fn release_for_daemon(&self) {
        {
            let mut guard = self
                .shared
                .handover
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *guard = Handover::Pending;
        }
        // Serializes with the supervisor: once the planner lock is held, no direct manager can
        // be opened any more (every later step sees the pending handover).
        let mut planner = self
            .shared
            .planner
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.shared.camera.mode() == Some(CameraMode::DirectV4l) {
            let reason = CameraBlockReason::HandingOver;
            let previous = self.shared.camera.replace(
                Arc::new(UnavailableCameraManager),
                Some(CameraMode::Blocked(reason)),
                Some(reason.user_message()),
            );
            let released = release_manager(previous, self.shared.timing.release_timeout);
            tracing::info!(released, "direct camera released for soos-daemon");
            planner.force(CameraMode::Blocked(reason));
        }
    }

    /// Reports the Resume outcome: on success direct mode stays disabled until the daemon is
    /// seen active (or the grace period ends); on failure it is re-enabled immediately.
    pub fn finish_handover(&self, daemon_started: bool) {
        let mut guard = self
            .shared
            .handover
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = if daemon_started {
            Instant::now()
                .checked_add(self.shared.timing.handover_grace)
                .map_or(Handover::Idle, |until| Handover::Started { until })
        } else {
            Handover::Idle
        };
    }
}

/// Late-bound slot for the [`CameraHandover`] (set once the camera source is attached).
pub type HandoverSlot = Arc<OnceLock<CameraHandover>>;

/// Privileged executor releasing the direct camera before `ResumeDaemon` runs.
pub struct HandoverExecutor {
    inner: Arc<dyn PrivilegedExecutor>,
    handover: HandoverSlot,
}

impl HandoverExecutor {
    /// Wraps `inner`; `handover` may still be empty (mock camera, no supervisor).
    pub fn new(inner: Arc<dyn PrivilegedExecutor>, handover: HandoverSlot) -> Self {
        Self { inner, handover }
    }
}

impl PrivilegedExecutor for HandoverExecutor {
    fn execute(&self, action: PrivilegedAction) -> PrivilegedOutcome {
        let Some(handover) = self.handover.get() else {
            return self.inner.execute(action);
        };
        if !matches!(action, PrivilegedAction::ResumeDaemon) {
            return self.inner.execute(action);
        }
        handover.release_for_daemon();
        let outcome = self.inner.execute(action);
        let started = matches!(outcome, PrivilegedOutcome::DaemonResumed(Ok(())));
        handover.finish_handover(started);
        outcome
    }
}

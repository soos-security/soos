//! Background, throttled polling of the `soos-daemon.service` state (review finding CAM-06).
//!
//! The header used to run `systemctl is-active` on every repaint, i.e. one fork/exec per camera
//! frame on the UI thread. The [`DaemonMonitor`] now polls from a dedicated thread at most once
//! per [`DAEMON_POLL_INTERVAL`] (or promptly after [`DaemonMonitor::request_refresh`]) and
//! publishes the result in an atomic that the UI reads without blocking.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Interval between two background `systemctl is-active` probes.
pub const DAEMON_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Granularity at which the monitor thread checks for shutdown and refresh requests.
const MONITOR_TICK: Duration = Duration::from_millis(50);

/// Source of truth for whether the daemon service is running (injectable for tests).
pub trait DaemonStatusProbe: Send + Sync {
    /// Returns whether `soos-daemon.service` is active. May block; never called on the UI thread.
    fn is_active(&self) -> bool;
}

/// Upper bound on the `systemctl show` output read (one short `ActiveState` value).
const MAX_ACTIVE_STATE_BYTES: usize = 256;

/// Whether a systemd `ActiveState` value means the daemon may own the camera.
///
/// Only `inactive` and `failed` release the device. `active`, `reloading`, `refreshing`,
/// `activating` (start-up and `auto-restart`) and `deactivating` keep the camera with the
/// daemon, and so does any value this build does not know or an empty/unreadable answer:
/// opening `/dev/video*` directly while the daemon restarts would fight it for the device
/// (EBUSY; GitHub #314, CAM-NEW-7).
pub fn active_state_means_running(active_state: &str) -> bool {
    !matches!(active_state.trim(), "inactive" | "failed")
}

/// Production probe reading `systemctl show --property=ActiveState --value soos-daemon.service`
/// (`/usr/bin/systemctl`, never resolved through `PATH`).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemctlProbe;

impl DaemonStatusProbe for SystemctlProbe {
    fn is_active(&self) -> bool {
        let output = std::process::Command::new(crate::privileged::SYSTEMCTL_PROGRAM)
            .args([
                "show",
                "--property=ActiveState",
                "--value",
                "soos-daemon.service",
            ])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        // A spawn failure or a non-UTF-8 answer is treated as running (fail safe: the GUI
        // then uses the IPC preview and never grabs the device).
        match output {
            Ok(out) => {
                let text = out
                    .stdout
                    .get(..MAX_ACTIVE_STATE_BYTES)
                    .unwrap_or(&out.stdout);
                std::str::from_utf8(text).map_or(true, active_state_means_running)
            }
            Err(_) => true,
        }
    }
}

/// Rate limiter deciding when the next probe is due, with an injectable clock (`now`).
#[derive(Debug, Clone)]
pub struct PollThrottle {
    interval: Duration,
    last: Option<Instant>,
    forced: bool,
}

impl PollThrottle {
    /// Creates a throttle whose first poll is due immediately.
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
            forced: false,
        }
    }

    /// Returns `true` (and records `now` as the poll time) when a poll is due at `now`.
    pub fn due(&mut self, now: Instant) -> bool {
        let due = self.forced
            || self
                .last
                .is_none_or(|last| now.saturating_duration_since(last) >= self.interval);
        if due {
            self.last = Some(now);
            self.forced = false;
        }
        due
    }

    /// Makes the next [`PollThrottle::due`] call return `true` regardless of the interval.
    pub fn force(&mut self) {
        self.forced = true;
    }
}

/// Last known state of the daemon service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    /// No probe has completed yet.
    Unknown,
    /// The service is running.
    Active,
    /// The service is stopped (paused by the user, failed or not installed).
    Inactive,
}

const STATE_UNKNOWN: u8 = 0;
const STATE_ACTIVE: u8 = 1;
const STATE_INACTIVE: u8 = 2;

fn decode_state(raw: u8) -> DaemonState {
    match raw {
        STATE_ACTIVE => DaemonState::Active,
        STATE_INACTIVE => DaemonState::Inactive,
        _ => DaemonState::Unknown,
    }
}

/// Cloneable, lock-free read handle on the state published by a [`DaemonMonitor`].
///
/// Used by background threads (the camera-source supervisor) that must follow the daemon
/// state without owning the monitor.
#[derive(Debug, Clone)]
pub struct DaemonStateReader {
    state: Arc<AtomicU8>,
}

impl DaemonStateReader {
    /// Returns the last published state. Never blocks.
    pub fn state(&self) -> DaemonState {
        decode_state(self.state.load(Ordering::Acquire))
    }
}

/// Background thread publishing the daemon service state.
pub struct DaemonMonitor {
    state: Arc<AtomicU8>,
    refresh: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl DaemonMonitor {
    /// Spawns the monitor thread probing at most once per `interval`.
    ///
    /// # Errors
    ///
    /// Returns the OS error if the thread cannot be spawned.
    pub fn spawn(probe: Arc<dyn DaemonStatusProbe>, interval: Duration) -> std::io::Result<Self> {
        let state = Arc::new(AtomicU8::new(STATE_UNKNOWN));
        let refresh = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));

        let thread_state = Arc::clone(&state);
        let thread_refresh = Arc::clone(&refresh);
        let thread_running = Arc::clone(&running);
        let handle = thread::Builder::new()
            .name("soos-gui-daemon-monitor".to_string())
            .spawn(move || {
                let mut throttle = PollThrottle::new(interval);
                let mut last_logged = STATE_UNKNOWN;
                while thread_running.load(Ordering::Acquire) {
                    if thread_refresh.swap(false, Ordering::AcqRel) {
                        throttle.force();
                    }
                    if throttle.due(Instant::now()) {
                        let value = if probe.is_active() {
                            STATE_ACTIVE
                        } else {
                            STATE_INACTIVE
                        };
                        thread_state.store(value, Ordering::Release);
                        if value != last_logged {
                            tracing::info!(
                                active = value == STATE_ACTIVE,
                                "soos-daemon.service state changed"
                            );
                            last_logged = value;
                        }
                    }
                    thread::sleep(MONITOR_TICK);
                }
            })?;

        Ok(Self {
            state,
            refresh,
            running,
            handle: Some(handle),
        })
    }

    /// Returns the last published state. Lock-free and never blocks; safe to call every frame.
    pub fn state(&self) -> DaemonState {
        decode_state(self.state.load(Ordering::Acquire))
    }

    /// Returns a cloneable read handle on the published state.
    pub fn state_reader(&self) -> DaemonStateReader {
        DaemonStateReader {
            state: Arc::clone(&self.state),
        }
    }

    /// Asks the monitor thread to probe again on its next tick (e.g. after Pause/Resume).
    pub fn request_refresh(&self) {
        self.refresh.store(true, Ordering::Release);
    }
}

impl Drop for DaemonMonitor {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

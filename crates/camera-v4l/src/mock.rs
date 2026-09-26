//! Mock camera manager implementation for hardware-free simulation and testing.

use crate::config::CameraConfig;
use crate::error::CameraError;
use crate::frame::{Frame, PixelFormat};
use crate::manager::CameraManager;
use arc_swap::ArcSwapOption;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Synthetic frame generator and fault injector for headless CI and unit testing.
pub struct MockCameraManager {
    config: CameraConfig,
    latest_frame: Arc<ArcSwapOption<Frame>>,
    is_ready: Arc<AtomicBool>,
    starved: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    warmup_remaining: Arc<AtomicUsize>,
    sequence: Arc<AtomicU64>,
    frozen: Arc<AtomicBool>,
    active_error: Arc<RwLock<Option<CameraError>>>,
    last_activity: Arc<RwLock<Instant>>,
    worker_handle: Option<JoinHandle<()>>,
}

/// Stores a new frame into the ArcSwapOption slot only if its sequence number is >=
/// the current frame's sequence number, enforcing strict monotonic visibility.
fn store_frame_monotonic(latest: &ArcSwapOption<Frame>, new_frame: Arc<Frame>) {
    latest.rcu(|current| {
        if let Some(cur) = current.as_ref() {
            if new_frame.sequence < cur.sequence {
                return Some(Arc::clone(cur));
            }
        }
        Some(Arc::clone(&new_frame))
    });
}

impl MockCameraManager {
    /// Creates and starts a new mock camera manager with the given configuration.
    pub fn new(config: CameraConfig) -> Self {
        let latest_frame = Arc::new(ArcSwapOption::empty());
        let is_ready = Arc::new(AtomicBool::new(false));
        let starved = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let frozen = Arc::new(AtomicBool::new(false));
        let warmup_remaining = Arc::new(AtomicUsize::new(config.warmup_frames));
        let sequence = Arc::new(AtomicU64::new(0));
        let active_error = Arc::new(RwLock::new(None));
        let last_activity = Arc::new(RwLock::new(Instant::now()));

        let latest_clone = Arc::clone(&latest_frame);
        let ready_clone = Arc::clone(&is_ready);
        let starved_clone = Arc::clone(&starved);
        let running_clone = Arc::clone(&running);
        let frozen_clone = Arc::clone(&frozen);
        let warmup_clone = Arc::clone(&warmup_remaining);
        let sequence_clone = Arc::clone(&sequence);
        let error_clone = Arc::clone(&active_error);
        let activity_clone = Arc::clone(&last_activity);
        let cfg = config.clone();

        let handle = thread::Builder::new()
            .name("soos-mock-camera".into())
            .spawn(move || {
                let _start_time = Instant::now();

                let full_frame_interval = Duration::from_micros(
                    1_000_000u64.checked_div(cfg.fps as u64).unwrap_or(33_333),
                );

                while running_clone.load(Ordering::Acquire) {
                    let is_frozen = frozen_clone.load(Ordering::Acquire);
                    if is_frozen {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    // Check if an error is injected
                    let has_error = {
                        let guard = error_clone.read().unwrap_or_else(|e| e.into_inner());
                        guard.is_some()
                    };

                    let is_starved = starved_clone.load(Ordering::Acquire);

                    if has_error || is_starved {
                        ready_clone.store(false, Ordering::Release);
                        latest_clone.store(None);
                        warmup_clone.store(cfg.warmup_frames, Ordering::Release);
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }

                    // Handle warmup frame discard at configured frame rate
                    let remaining = warmup_clone.load(Ordering::Acquire);
                    if remaining > 0 {
                        ready_clone.store(false, Ordering::Release);
                        warmup_clone.fetch_sub(1, Ordering::AcqRel);
                        sequence_clone.fetch_add(1, Ordering::SeqCst);
                        thread::sleep(full_frame_interval);
                        if warmup_clone.load(Ordering::Acquire) == 0 {
                            // Warmup completed: refresh activity timestamp so idle timeout starts from here
                            let mut guard =
                                activity_clone.write().unwrap_or_else(|e| e.into_inner());
                            *guard = Instant::now();
                        }
                        continue;
                    }

                    // Compute capture interval based on idle timeout
                    let now = Instant::now();
                    let elapsed_idle = {
                        let last = activity_clone.read().unwrap_or_else(|e| e.into_inner());
                        now.duration_since(*last)
                    };

                    if elapsed_idle > cfg.idle_timeout {
                        // Suspended state: camera device is closed, privacy LED extinguished
                        ready_clone.store(false, Ordering::Release);
                        latest_clone.store(None);
                        warmup_clone.store(cfg.warmup_frames, Ordering::Release);

                        // Sleep in suspended state until activity is notified or stopped
                        while running_clone.load(Ordering::Acquire) {
                            let recent_activity = {
                                let last = activity_clone.read().unwrap_or_else(|e| e.into_inner());
                                Instant::now().duration_since(*last) < cfg.idle_timeout
                            };
                            if recent_activity {
                                break;
                            }
                            thread::sleep(Duration::from_millis(15));
                        }
                        continue;
                    }

                    // Idle throttled state: when idle for more than half idle_timeout
                    let half_timeout = cfg.idle_timeout.checked_div(2).unwrap_or(cfg.idle_timeout);
                    let is_throttled = elapsed_idle > half_timeout;
                    let effective_fps = if is_throttled { cfg.idle_fps } else { cfg.fps };
                    let frame_interval = Duration::from_micros(
                        1_000_000u64
                            .checked_div(effective_fps as u64)
                            .unwrap_or(33_333),
                    );

                    let seq = sequence_clone.fetch_add(1, Ordering::SeqCst);
                    // Generate synthetic frame with true CLOCK_MONOTONIC timestamp
                    let mono_ns = monotonic_nanos();
                    let frame =
                        generate_synthetic_frame(cfg.width, cfg.height, cfg.format, seq, mono_ns);

                    if starved_clone.load(Ordering::Acquire) {
                        ready_clone.store(false, Ordering::Release);
                        latest_clone.store(None);
                        continue;
                    }

                    store_frame_monotonic(&latest_clone, Arc::new(frame));
                    // Camera has completed warmup and is healthy: publish ready flag after frame store
                    ready_clone.store(true, Ordering::Release);

                    // Responsive sleep checking running_clone and starved_clone in small increments
                    let sleep_start = Instant::now();
                    while running_clone.load(Ordering::Acquire)
                        && !starved_clone.load(Ordering::Acquire)
                        && sleep_start.elapsed() < frame_interval
                    {
                        let is_idle_expired = {
                            let last = activity_clone.read().unwrap_or_else(|e| e.into_inner());
                            last.elapsed() > cfg.idle_timeout
                        };
                        if is_idle_expired {
                            break;
                        }
                        let rem = frame_interval.saturating_sub(sleep_start.elapsed());
                        thread::sleep(Duration::from_millis(5).min(rem));
                    }
                }
            })
            .ok();

        Self {
            config,
            latest_frame,
            is_ready,
            starved,
            running,
            frozen,
            warmup_remaining,
            sequence,
            active_error,
            last_activity,
            worker_handle: handle,
        }
    }

    /// Creates a mock camera manager with default configuration.
    pub fn new_default() -> Self {
        Self::new(CameraConfig::default())
    }

    /// Configures an active simulated error (e.g. ENODEV, EBUSY, EIO).
    pub fn set_error(&self, error: Option<CameraError>) {
        let mut guard = self.active_error.write().unwrap_or_else(|e| e.into_inner());
        *guard = error;
        if guard.is_some() {
            self.is_ready.store(false, Ordering::Release);
            self.latest_frame.store(None);
        }
    }

    /// Freezes frame generation without dropping readiness, simulating a frozen sensor.
    pub fn set_frozen(&self, frozen: bool) {
        self.frozen.store(frozen, Ordering::Release);
    }

    /// Simulates frame starvation (no new frames generated).
    pub fn set_starved(&self, starved: bool) {
        self.starved.store(starved, Ordering::Release);
        if starved {
            self.is_ready.store(false, Ordering::Release);
            self.latest_frame.store(None);
        }
    }

    /// Overrides the ready status directly.
    pub fn set_ready(&self, ready: bool) {
        if ready {
            let mut guard = self
                .last_activity
                .write()
                .unwrap_or_else(|e| e.into_inner());
            *guard = Instant::now();
        }
        self.is_ready.store(ready, Ordering::Release);
    }

    /// Sets remaining warmup frames before becoming ready.
    pub fn set_warmup_remaining(&self, frames: usize) {
        self.warmup_remaining.store(frames, Ordering::Release);
        if frames > 0 {
            self.is_ready.store(false, Ordering::Release);
        }
    }

    /// Injects a specific frame into the latest frame slot.
    pub fn push_frame(&self, frame: Frame) {
        let mut guard = self
            .last_activity
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Instant::now();
        drop(guard);

        self.sequence
            .fetch_max(frame.sequence.saturating_add(1), Ordering::SeqCst);
        store_frame_monotonic(&self.latest_frame, Arc::new(frame));
        self.is_ready.store(true, Ordering::Release);
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &CameraConfig {
        &self.config
    }
}

impl CameraManager for MockCameraManager {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        self.latest_frame.load_full()
    }

    fn is_ready(&self) -> bool {
        if self.starved.load(Ordering::Acquire) {
            return false;
        }
        let elapsed = {
            let last = self.last_activity.read().unwrap_or_else(|e| e.into_inner());
            last.elapsed()
        };
        if elapsed > self.config.idle_timeout {
            return false;
        }
        self.is_ready.load(Ordering::Acquire)
    }

    fn notify_activity(&self) {
        let mut guard = self
            .last_activity
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Instant::now();

        // Fulfill CameraManager contract ("immediately restoring full FPS") and
        // guarantee fresh frame availability for incoming auth requests under CI load.
        if !self.starved.load(Ordering::Acquire)
            && !self.frozen.load(Ordering::Acquire)
            && self.running.load(Ordering::Acquire)
        {
            let has_error = self
                .active_error
                .read()
                .map(|g| g.is_some())
                .unwrap_or(false);
            if !has_error {
                let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
                let mono_ns = monotonic_nanos();
                let frame = generate_synthetic_frame(
                    self.config.width,
                    self.config.height,
                    self.config.format,
                    seq,
                    mono_ns,
                );
                store_frame_monotonic(&self.latest_frame, Arc::new(frame));
                self.is_ready.store(true, Ordering::Release);
            }
        }
    }

    fn stop(&self) {
        self.running.store(false, Ordering::Release);
        self.is_ready.store(false, Ordering::Release);
    }
}

impl Drop for MockCameraManager {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        self.is_ready.store(false, Ordering::Release);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Helper function to generate synthetic frame buffers.
fn generate_synthetic_frame(
    width: u32,
    height: u32,
    format: PixelFormat,
    sequence: u64,
    timestamp_mono_ns: u64,
) -> Frame {
    let size = format.expected_buffer_size(width, height).unwrap_or(
        (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(2),
    );

    let mut data = vec![0u8; size];
    let mut val: u8 = 0;
    for byte in data.iter_mut() {
        *byte = val.wrapping_mul(31);
        val = val.wrapping_add(1);
    }

    Frame::new(data, width, height, timestamp_mono_ns, format, sequence)
}

/// Returns the current monotonic clock timestamp in nanoseconds.
fn monotonic_nanos() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: Stack-allocated timespec pointer is valid and non-null for clock_gettime.
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    if ret == 0 && ts.tv_sec >= 0 && ts.tv_nsec >= 0 {
        let sec = ts.tv_sec.cast_unsigned().saturating_mul(1_000_000_000);
        sec.saturating_add(ts.tv_nsec.cast_unsigned())
    } else {
        0
    }
}

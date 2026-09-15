//! Mock camera manager implementation for hardware-free simulation and testing.

use crate::config::CameraConfig;
use crate::error::CameraError;
use crate::frame::{Frame, PixelFormat};
use crate::manager::CameraManager;
use arc_swap::ArcSwapOption;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    active_error: Arc<RwLock<Option<CameraError>>>,
    last_activity: Arc<RwLock<Instant>>,
    worker_handle: Option<JoinHandle<()>>,
}

impl MockCameraManager {
    /// Creates and starts a new mock camera manager with the given configuration.
    pub fn new(config: CameraConfig) -> Self {
        let latest_frame = Arc::new(ArcSwapOption::empty());
        let is_ready = Arc::new(AtomicBool::new(false));
        let starved = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let warmup_remaining = Arc::new(AtomicUsize::new(config.warmup_frames));
        let active_error = Arc::new(RwLock::new(None));
        let last_activity = Arc::new(RwLock::new(Instant::now()));

        let latest_clone = Arc::clone(&latest_frame);
        let ready_clone = Arc::clone(&is_ready);
        let starved_clone = Arc::clone(&starved);
        let running_clone = Arc::clone(&running);
        let warmup_clone = Arc::clone(&warmup_remaining);
        let error_clone = Arc::clone(&active_error);
        let activity_clone = Arc::clone(&last_activity);
        let cfg = config.clone();

        let handle = thread::Builder::new()
            .name("soos-mock-camera".into())
            .spawn(move || {
                let mut sequence: u64 = 0;
                let _start_time = Instant::now();

                let full_frame_interval = Duration::from_micros(
                    1_000_000u64.checked_div(cfg.fps as u64).unwrap_or(33_333),
                );

                while running_clone.load(Ordering::Relaxed) {
                    // Check if an error is injected
                    let has_error = {
                        let guard = error_clone.read().unwrap_or_else(|e| e.into_inner());
                        guard.is_some()
                    };

                    let is_starved = starved_clone.load(Ordering::Relaxed);

                    if has_error || is_starved {
                        ready_clone.store(false, Ordering::Relaxed);
                        latest_clone.store(None);
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }

                    // Handle warmup frame discard at configured frame rate
                    let remaining = warmup_clone.load(Ordering::Relaxed);
                    if remaining > 0 {
                        ready_clone.store(false, Ordering::Relaxed);
                        warmup_clone.fetch_sub(1, Ordering::Relaxed);
                        sequence = sequence.saturating_add(1);
                        thread::sleep(full_frame_interval);
                        continue;
                    }

                    // Camera has completed warmup and is healthy
                    ready_clone.store(true, Ordering::Relaxed);

                    // Compute capture interval based on idle timeout
                    let now = Instant::now();
                    let is_idle = {
                        let last = activity_clone.read().unwrap_or_else(|e| e.into_inner());
                        now.duration_since(*last) > cfg.idle_timeout
                    };

                    let effective_fps = if is_idle { cfg.idle_fps } else { cfg.fps };
                    let frame_interval = Duration::from_micros(
                        1_000_000u64
                            .checked_div(effective_fps as u64)
                            .unwrap_or(33_333),
                    );

                    // Generate synthetic frame with true CLOCK_MONOTONIC timestamp
                    let mono_ns = monotonic_nanos();
                    let frame = generate_synthetic_frame(
                        cfg.width, cfg.height, cfg.format, sequence, mono_ns,
                    );

                    latest_clone.store(Some(Arc::new(frame)));
                    sequence = sequence.saturating_add(1);

                    thread::sleep(frame_interval);
                }
            })
            .ok();

        Self {
            config,
            latest_frame,
            is_ready,
            starved,
            running,
            warmup_remaining,
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
            self.is_ready.store(false, Ordering::Relaxed);
            self.latest_frame.store(None);
        }
    }

    /// Simulates frame starvation (no new frames generated).
    pub fn set_starved(&self, starved: bool) {
        self.starved.store(starved, Ordering::Relaxed);
        if starved {
            self.is_ready.store(false, Ordering::Relaxed);
            self.latest_frame.store(None);
        }
    }

    /// Overrides the ready status directly.
    pub fn set_ready(&self, ready: bool) {
        self.is_ready.store(ready, Ordering::Relaxed);
    }

    /// Sets remaining warmup frames before becoming ready.
    pub fn set_warmup_remaining(&self, frames: usize) {
        self.warmup_remaining.store(frames, Ordering::Relaxed);
        if frames > 0 {
            self.is_ready.store(false, Ordering::Relaxed);
        }
    }

    /// Injects a specific frame into the latest frame slot.
    pub fn push_frame(&self, frame: Frame) {
        self.latest_frame.store(Some(Arc::new(frame)));
        self.is_ready.store(true, Ordering::Relaxed);
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &CameraConfig {
        &self.config
    }
}

impl CameraManager for MockCameraManager {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        if !self.is_ready() {
            return None;
        }
        self.latest_frame.load_full()
    }

    fn is_ready(&self) -> bool {
        self.is_ready.load(Ordering::Relaxed)
    }

    fn notify_activity(&self) {
        let mut guard = self
            .last_activity
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Instant::now();
    }

    fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        self.is_ready.store(false, Ordering::Relaxed);
    }
}

impl Drop for MockCameraManager {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
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
    let step = u8::try_from(sequence % 256).unwrap_or(0);

    // Fill with a synthetic gradient pattern based on sequence number
    for (idx, byte) in data.iter_mut().enumerate() {
        let idx_u8 = u8::try_from(idx % 256).unwrap_or(0);
        *byte = (idx_u8.wrapping_add(step)).wrapping_mul(31);
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

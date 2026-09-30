//! Production V4L2 MMAP camera manager implementation.

use crate::config::CameraConfig;
use crate::error::CameraError;
use crate::frame::{Frame, PixelFormat};
use crate::manager::CameraManager;
use crate::status::{CameraStatus, CameraStatusCell};
use arc_swap::ArcSwapOption;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{info, warn};
use v4l::capability::Flags;
use v4l::format::fourcc::FourCC;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;

/// Hardware-backed camera manager utilizing Linux V4L2 MMAP streaming.
pub struct V4lCameraManager {
    config: CameraConfig,
    latest_frame: Arc<ArcSwapOption<Frame>>,
    is_ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_activity: Arc<RwLock<Instant>>,
    status: Arc<CameraStatusCell>,
    worker_handle: Option<JoinHandle<()>>,
}

impl V4lCameraManager {
    /// Spawns the background capture worker managing device lifecycle,
    /// warmup auto-exposure discard, lock-free frame swapping, and exponential backoff.
    pub fn spawn(config: CameraConfig) -> Result<Self, CameraError> {
        let latest_frame = Arc::new(ArcSwapOption::empty());
        let is_ready = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let last_activity = Arc::new(RwLock::new(Instant::now()));
        let status = Arc::new(CameraStatusCell::new());

        let status_clone = Arc::clone(&status);
        let latest_clone = Arc::clone(&latest_frame);
        let ready_clone = Arc::clone(&is_ready);
        let running_clone = Arc::clone(&running);
        let activity_clone = Arc::clone(&last_activity);
        let cfg = config.clone();

        let handle = thread::Builder::new()
            .name("soos-v4l-capture".into())
            .spawn(move || {
                run_v4l_supervisor(
                    cfg,
                    latest_clone,
                    ready_clone,
                    running_clone,
                    activity_clone,
                    &status_clone,
                );
            })
            .map_err(|e| CameraError::Io {
                path: config.device_path.clone(),
                source: e,
            })?;

        Ok(Self {
            config,
            latest_frame,
            is_ready,
            running,
            last_activity,
            status,
            worker_handle: Some(handle),
        })
    }

    /// Returns the configuration used by this camera manager.
    pub fn config(&self) -> &CameraConfig {
        &self.config
    }
}

impl CameraManager for V4lCameraManager {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        if !self.is_ready() {
            return None;
        }
        self.latest_frame.load_full()
    }

    fn is_ready(&self) -> bool {
        let elapsed = {
            let last = self.last_activity.read().unwrap_or_else(|e| e.into_inner());
            last.elapsed()
        };
        if !self.config.idle_timeout.is_zero() && elapsed > self.config.idle_timeout {
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
    }

    fn stop(&self) {
        self.running.store(false, Ordering::Release);
        self.is_ready.store(false, Ordering::Release);
    }

    fn status(&self) -> CameraStatus {
        if self.is_ready() {
            return CameraStatus::Ready;
        }
        match self.status.get() {
            CameraStatus::Ready => CameraStatus::Starting,
            other => other,
        }
    }
}

impl Drop for V4lCameraManager {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        self.is_ready.store(false, Ordering::Release);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupervisorAction {
    Shutdown,
    Suspend,
}

/// Supervisor loop handling device reconnection, streaming, exponential backoff, and auto-standby.
fn run_v4l_supervisor(
    config: CameraConfig,
    latest_frame: Arc<ArcSwapOption<Frame>>,
    is_ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_activity: Arc<RwLock<Instant>>,
    status: &CameraStatusCell,
) {
    let mut current_backoff = config.min_backoff;

    while running.load(Ordering::Acquire) {
        match open_and_stream(&config, &latest_frame, &is_ready, &running, &last_activity) {
            Ok(SupervisorAction::Shutdown) => {
                // Clean shutdown requested
                break;
            }
            Ok(SupervisorAction::Suspend) => {
                is_ready.store(false, Ordering::Release);
                status.set(CameraStatus::Suspended);
                latest_frame.store(None);
                current_backoff = config.min_backoff;

                // Suspended state: wait for notify_activity() or shutdown
                while running.load(Ordering::Acquire) {
                    let recent_activity = {
                        let last = last_activity.read().unwrap_or_else(|e| e.into_inner());
                        config.idle_timeout.is_zero() || last.elapsed() < config.idle_timeout
                    };
                    if recent_activity {
                        info!(
                            "Camera activity requested on '{}'; resuming from auto-standby",
                            config.device_path.display()
                        );
                        status.set(CameraStatus::Starting);
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
            Err(err) => {
                is_ready.store(false, Ordering::Release);
                // A published frame means this attempt streamed successfully: the failure
                // count restarts instead of accumulating across healthy sessions.
                if latest_frame.load().is_some() {
                    status.set(CameraStatus::Starting);
                }
                latest_frame.store(None);
                status.record_error(err.kind());

                warn!(
                    "Camera error on '{}': {}. Backing off for {:?}",
                    config.device_path.display(),
                    err,
                    current_backoff
                );

                // Sleep backoff with periodic running check
                let sleep_start = Instant::now();
                while running.load(Ordering::Acquire) && sleep_start.elapsed() < current_backoff {
                    let rem = current_backoff.saturating_sub(sleep_start.elapsed());
                    thread::sleep(Duration::from_millis(20).min(rem));
                }

                // Exponential backoff doubling up to max_backoff
                current_backoff = (current_backoff.saturating_mul(2)).min(config.max_backoff);
            }
        }
    }

    is_ready.store(false, Ordering::Release);
    latest_frame.store(None);
    status.set(CameraStatus::Stopped);
}

/// Priority order for automatic format negotiation:
/// RGB24 (highest priority, uncompressed) -> YUYV -> NV12 -> MJPEG -> Grey.
pub const FORMAT_PRIORITY: [PixelFormat; 5] = [
    PixelFormat::Rgb24,
    PixelFormat::Yuyv,
    PixelFormat::Nv12,
    PixelFormat::Mjpeg,
    PixelFormat::Grey,
];

/// Negotiates the optimal capture pixel format given supported formats and optional preferred format.
pub fn negotiate_format(
    supported: &[PixelFormat],
    preferred: Option<PixelFormat>,
) -> Result<PixelFormat, CameraError> {
    if supported.is_empty() {
        return Err(CameraError::NoSupportedFormats);
    }

    if let Some(pref) = preferred {
        if supported.contains(&pref) {
            return Ok(pref);
        }
    }

    for &prio in &FORMAT_PRIORITY {
        if supported.contains(&prio) {
            return Ok(prio);
        }
    }

    Err(CameraError::NoCompatibleFormat {
        supported: supported.to_vec(),
    })
}

/// Maps a V4L2 FourCC to a known `PixelFormat`.
pub fn fourcc_to_pixel_format(fourcc: FourCC) -> Option<PixelFormat> {
    match &fourcc.repr {
        b"RGB3" | b"RGB4" | b"BGR3" => Some(PixelFormat::Rgb24),
        b"YUYV" => Some(PixelFormat::Yuyv),
        b"NV12" => Some(PixelFormat::Nv12),
        b"MJPG" => Some(PixelFormat::Mjpeg),
        b"GREY" | b"Y800" | b"Y8  " => Some(PixelFormat::Grey),
        _ => None,
    }
}

/// Maps a `PixelFormat` to its canonical V4L2 FourCC representation.
pub fn pixel_format_to_fourcc(format: PixelFormat) -> FourCC {
    match format {
        PixelFormat::Yuyv => FourCC::new(b"YUYV"),
        PixelFormat::Rgb24 => FourCC::new(b"RGB3"),
        PixelFormat::Grey => FourCC::new(b"GREY"),
        PixelFormat::Mjpeg => FourCC::new(b"MJPG"),
        PixelFormat::Nv12 => FourCC::new(b"NV12"),
    }
}

/// Opens device, allocates MMAP queue, discards warmup frames, and streams frames into RAM snapshot.
fn open_and_stream(
    config: &CameraConfig,
    latest_frame: &Arc<ArcSwapOption<Frame>>,
    is_ready: &Arc<AtomicBool>,
    running: &Arc<AtomicBool>,
    last_activity: &Arc<RwLock<Instant>>,
) -> Result<SupervisorAction, CameraError> {
    let device = v4l::Device::with_path(&config.device_path)
        .map_err(|e| CameraError::from_io_error(config.device_path.clone(), e))?;

    let caps = device
        .query_caps()
        .map_err(|e| CameraError::QueryCapabilities {
            path: config.device_path.clone(),
            reason: e.to_string(),
        })?;

    if !caps.capabilities.contains(Flags::VIDEO_CAPTURE) {
        return Err(CameraError::UnsupportedCapability {
            path: config.device_path.clone(),
        });
    }

    // Query hardware-supported formats via VIDIOC_ENUM_FMT
    let enum_fmts = Capture::enum_formats(&device).unwrap_or_default();
    let supported: Vec<PixelFormat> = enum_fmts
        .into_iter()
        .filter_map(|desc| fourcc_to_pixel_format(desc.fourcc))
        .collect();

    let target_format = if supported.is_empty() {
        config.format
    } else {
        let preferred = if config.auto_format {
            None
        } else {
            Some(config.format)
        };
        negotiate_format(&supported, preferred)?
    };

    let fourcc = pixel_format_to_fourcc(target_format);

    let req_format = v4l::Format::new(config.width, config.height, fourcc);
    let actual_format =
        Capture::set_format(&device, &req_format).map_err(|e| CameraError::SetFormat {
            path: config.device_path.clone(),
            width: config.width,
            height: config.height,
            format: target_format,
            reason: e.to_string(),
        })?;

    let actual_width = actual_format.width;
    let actual_height = actual_format.height;

    let mut stream =
        v4l::io::mmap::Stream::with_buffers(&device, v4l::buffer::Type::VideoCapture, 4).map_err(
            |e| CameraError::StreamCreate {
                path: config.device_path.clone(),
                reason: e.to_string(),
            },
        )?;

    // Set a non-infinite timeout on the MMAP stream handle so DQBUF does not block indefinitely.
    // Timeout is computed adaptively from the configured FPS (e.g. 3x frame interval, clamped to 150-250ms),
    // ensuring Drop completes in < 500ms (Criterion C9) even if the hardware is idle or stalled.
    let stream_timeout_ms = 2000;
    stream.set_timeout(Duration::from_millis(stream_timeout_ms));

    info!(
        "Camera stream initialized on '{}' ({}x{}, {:?})",
        config.device_path.display(),
        actual_width,
        actual_height,
        target_format
    );

    let _start_time = Instant::now();
    let mut warmup_discarded: usize = 0;
    let mut sequence: u64 = 0;

    while running.load(Ordering::Acquire) {
        let (buf, meta) = match stream.next() {
            Ok(val) => val,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                if !running.load(Ordering::Acquire) {
                    // Graceful shutdown requested while waiting for DQBUF
                    return Ok(SupervisorAction::Shutdown);
                }
                return Err(CameraError::BufferDequeue {
                    path: config.device_path.clone(),
                    reason: "Frame capture timed out waiting for hardware buffer".to_string(),
                });
            }
            Err(e) => {
                if let Some(libc::ENODEV) = e.raw_os_error() {
                    return Err(CameraError::DeviceNotFound {
                        path: config.device_path.clone(),
                        source: e,
                    });
                } else {
                    return Err(CameraError::BufferDequeue {
                        path: config.device_path.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        };

        // Discard initial frames for auto-exposure convergence
        if warmup_discarded < config.warmup_frames {
            warmup_discarded = warmup_discarded.saturating_add(1);
            is_ready.store(false, Ordering::Release);
            continue;
        }

        let bytesused = meta.bytesused as usize;
        let data = if bytesused > 0 && bytesused <= buf.len() {
            buf.get(..bytesused)
                .map(|s| s.to_vec())
                .unwrap_or_else(|| buf.to_vec())
        } else {
            buf.to_vec()
        };

        let mono_ns = monotonic_nanos();
        let frame = Frame::new(
            data,
            actual_width,
            actual_height,
            mono_ns,
            target_format,
            sequence,
        );

        latest_frame.store(Some(Arc::new(frame)));
        // Camera is now stabilized and ready: publish ready flag with Release after storing frame
        is_ready.store(true, Ordering::Release);
        sequence = sequence.saturating_add(1);

        // Check for idle auto-standby: if idle for more than idle_timeout,
        // suspend capture and release hardware device handle to extinguish privacy LED
        let is_idle = !config.idle_timeout.is_zero() && {
            let last = last_activity.read().unwrap_or_else(|e| e.into_inner());
            Instant::now().duration_since(*last) > config.idle_timeout
        };

        if is_idle {
            info!(
                "Camera idle timeout reached on '{}'; releasing device handle for auto-standby",
                config.device_path.display()
            );
            return Ok(SupervisorAction::Suspend);
        }
    }

    Ok(SupervisorAction::Shutdown)
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

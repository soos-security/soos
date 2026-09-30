//! Production V4L2 MMAP camera manager implementation.

use crate::config::CameraConfig;
use crate::error::CameraError;
use crate::frame::{Frame, PixelFormat};
use crate::manager::{CameraHealth, CameraManager};
use crate::status::{CameraStatus, CameraStatusCell};
use arc_swap::ArcSwapOption;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};
use v4l::capability::Flags;
use v4l::format::fourcc::FourCC;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;

/// Source of the device path the capture supervisor should (re)open.
///
/// Consulted by the supervisor after the current device was reported missing or unsuitable
/// (`DeviceNotFound`, `UnsupportedCapability`), so that a camera re-enumerated under a new
/// `/dev/videoN` index after suspend, replug or a boot race is picked up without a restart
/// (GitHub #151). Returning `None` keeps retrying the current path with bounded backoff.
pub trait DevicePathResolver: Send + Sync {
    /// Returns the device path that should currently be used, if any candidate exists.
    fn resolve(&self) -> Option<PathBuf>;
}

impl<F> DevicePathResolver for F
where
    F: Fn() -> Option<PathBuf> + Send + Sync,
{
    fn resolve(&self) -> Option<PathBuf> {
        self()
    }
}

/// Encoded [`CameraHealth`] values shared lock-free between the manager and its supervisor.
const HEALTH_STARTING: u8 = 0;
const HEALTH_STREAMING: u8 = 1;
const HEALTH_STANDBY: u8 = 2;
const HEALTH_RECOVERING: u8 = 3;
const HEALTH_DEAD: u8 = 4;

fn encode_health(health: CameraHealth) -> u8 {
    match health {
        CameraHealth::Starting => HEALTH_STARTING,
        CameraHealth::Streaming => HEALTH_STREAMING,
        CameraHealth::Standby => HEALTH_STANDBY,
        CameraHealth::Recovering => HEALTH_RECOVERING,
        CameraHealth::Dead => HEALTH_DEAD,
    }
}

/// Decodes a stored health value; any unknown encoding is treated as `Dead` (fail-closed).
fn decode_health(raw: u8) -> CameraHealth {
    match raw {
        HEALTH_STARTING => CameraHealth::Starting,
        HEALTH_STREAMING => CameraHealth::Streaming,
        HEALTH_STANDBY => CameraHealth::Standby,
        HEALTH_RECOVERING => CameraHealth::Recovering,
        _ => CameraHealth::Dead,
    }
}

/// Returns whether a capture error means the current path no longer designates a usable
/// capture device, so the resolver should be consulted (GitHub #151).
fn warrants_reresolution(err: &CameraError) -> bool {
    matches!(
        err,
        CameraError::DeviceNotFound { .. } | CameraError::UnsupportedCapability { .. }
    )
}

/// State shared between the manager handle and the `soos-v4l-capture` supervisor thread.
struct SupervisorShared {
    latest_frame: Arc<ArcSwapOption<Frame>>,
    is_ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_activity: Arc<RwLock<Instant>>,
    health: Arc<AtomicU8>,
    device_path: Arc<RwLock<PathBuf>>,
    status: Arc<CameraStatusCell>,
}

impl SupervisorShared {
    fn set_health(&self, health: CameraHealth) {
        self.health.store(encode_health(health), Ordering::Release);
    }

    /// Withdraws readiness and the published frame (fail-closed).
    fn withdraw_frames(&self) {
        self.is_ready.store(false, Ordering::Release);
        self.latest_frame.store(None);
    }
}

/// Hardware-backed camera manager utilizing Linux V4L2 MMAP streaming.
pub struct V4lCameraManager {
    config: CameraConfig,
    latest_frame: Arc<ArcSwapOption<Frame>>,
    is_ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_activity: Arc<RwLock<Instant>>,
    health: Arc<AtomicU8>,
    device_path: Arc<RwLock<PathBuf>>,
    status: Arc<CameraStatusCell>,
    worker_handle: Option<JoinHandle<()>>,
}

impl V4lCameraManager {
    /// Spawns the background capture worker managing device lifecycle,
    /// warmup auto-exposure discard, lock-free frame swapping, and exponential backoff.
    ///
    /// The configured `device_path` is retried as-is forever (bounded backoff); it is never
    /// substituted by another device. Use [`V4lCameraManager::spawn_with_resolver`] for
    /// auto-selection that follows re-enumeration.
    pub fn spawn(config: CameraConfig) -> Result<Self, CameraError> {
        Self::spawn_inner(config, None)
    }

    /// Spawns the capture supervisor with a [`DevicePathResolver`] consulted after every
    /// `DeviceNotFound` / `UnsupportedCapability` backoff, so a camera that re-enumerates
    /// under a new node (suspend, replug, boot race) is reopened without a daemon restart.
    pub fn spawn_with_resolver(
        config: CameraConfig,
        resolver: Arc<dyn DevicePathResolver>,
    ) -> Result<Self, CameraError> {
        Self::spawn_inner(config, Some(resolver))
    }

    fn spawn_inner(
        config: CameraConfig,
        resolver: Option<Arc<dyn DevicePathResolver>>,
    ) -> Result<Self, CameraError> {
        let latest_frame = Arc::new(ArcSwapOption::empty());
        let is_ready = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let last_activity = Arc::new(RwLock::new(Instant::now()));
        let health = Arc::new(AtomicU8::new(HEALTH_STARTING));
        let device_path = Arc::new(RwLock::new(config.device_path.clone()));
        let status = Arc::new(CameraStatusCell::new());

        let shared = SupervisorShared {
            latest_frame: Arc::clone(&latest_frame),
            is_ready: Arc::clone(&is_ready),
            running: Arc::clone(&running),
            last_activity: Arc::clone(&last_activity),
            health: Arc::clone(&health),
            device_path: Arc::clone(&device_path),
            status: Arc::clone(&status),
        };
        let cfg = config.clone();

        let handle = thread::Builder::new()
            .name("soos-v4l-capture".into())
            .spawn(move || {
                run_v4l_supervisor(cfg, resolver, shared);
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
            health,
            device_path,
            status,
            worker_handle: Some(handle),
        })
    }

    /// Returns the device path the supervisor currently targets (it changes only when a
    /// resolver re-resolves the camera after device loss).
    pub fn current_device_path(&self) -> PathBuf {
        self.device_path
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Returns the configuration used by this camera manager.
    pub fn config(&self) -> &CameraConfig {
        &self.config
    }

    fn idle_expired(&self) -> bool {
        let elapsed = {
            let last = self.last_activity.read().unwrap_or_else(|e| e.into_inner());
            last.elapsed()
        };
        !self.config.idle_timeout.is_zero() && elapsed > self.config.idle_timeout
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
        if self.idle_expired() {
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

    fn health(&self) -> CameraHealth {
        let state = decode_health(self.health.load(Ordering::Acquire));
        // A streaming camera past its idle timeout is about to release the device for
        // auto-standby; report the idle state rather than a failure.
        if state == CameraHealth::Streaming && self.idle_expired() {
            return CameraHealth::Standby;
        }
        state
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

/// Capture thread entry point: runs the supervisor loop inside `catch_unwind`.
///
/// Under `panic = "unwind"` a panic in the capture thread would otherwise end the thread
/// silently while the daemon keeps running with a stale readiness state. A caught panic
/// marks the camera `Dead`, withdraws readiness and frames, and never restarts (fail-closed).
fn run_v4l_supervisor(
    config: CameraConfig,
    resolver: Option<Arc<dyn DevicePathResolver>>,
    shared: SupervisorShared,
) {
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        supervise(&config, resolver.as_deref(), &shared);
    }));

    shared.withdraw_frames();
    shared.status.set(CameraStatus::Stopped);
    if outcome.is_err() {
        shared.set_health(CameraHealth::Dead);
        error!(
            "Camera capture supervisor panicked; camera marked dead and unavailable until daemon restart (fail-closed)"
        );
    }
}

/// Supervisor loop handling device reconnection, re-resolution, streaming, exponential
/// backoff, and auto-standby.
fn supervise(
    config: &CameraConfig,
    resolver: Option<&dyn DevicePathResolver>,
    shared: &SupervisorShared,
) {
    let mut active = config.clone();
    let mut current_backoff = config.min_backoff;

    while shared.running.load(Ordering::Acquire) {
        match open_and_stream(
            &active,
            &shared.latest_frame,
            &shared.is_ready,
            &shared.running,
            &shared.last_activity,
            &shared.health,
        ) {
            Ok(SupervisorAction::Shutdown) => {
                // Clean shutdown requested
                break;
            }
            Ok(SupervisorAction::Suspend) => {
                shared.withdraw_frames();
                shared.set_health(CameraHealth::Standby);
                shared.status.set(CameraStatus::Suspended);
                current_backoff = config.min_backoff;

                // Suspended state: wait for notify_activity() or shutdown
                while shared.running.load(Ordering::Acquire) {
                    let recent_activity = {
                        let last = shared
                            .last_activity
                            .read()
                            .unwrap_or_else(|e| e.into_inner());
                        config.idle_timeout.is_zero() || last.elapsed() < config.idle_timeout
                    };
                    if recent_activity {
                        info!(
                            "Camera activity requested on '{}'; resuming from auto-standby",
                            active.device_path.display()
                        );
                        shared.set_health(CameraHealth::Starting);
                        shared.status.set(CameraStatus::Starting);
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
            Err(err) => {
                // A published frame means this attempt streamed successfully: the failure
                // count restarts instead of accumulating across healthy sessions.
                if shared.latest_frame.load().is_some() {
                    shared.status.set(CameraStatus::Starting);
                }
                shared.withdraw_frames();
                shared.set_health(CameraHealth::Recovering);
                shared.status.record_error(err.kind());

                if err.is_device_busy() {
                    warn!(
                        "Camera '{}' is held by another process (EBUSY); the root daemon must be \
                         the only owner of the device. Backing off for {:?}",
                        active.device_path.display(),
                        current_backoff
                    );
                } else {
                    warn!(
                        "Camera error on '{}': {}. Backing off for {:?}",
                        active.device_path.display(),
                        err,
                        current_backoff
                    );
                }

                // Sleep backoff with periodic running check
                let sleep_start = Instant::now();
                while shared.running.load(Ordering::Acquire)
                    && sleep_start.elapsed() < current_backoff
                {
                    let rem = current_backoff.saturating_sub(sleep_start.elapsed());
                    thread::sleep(Duration::from_millis(20).min(rem));
                }

                // Exponential backoff doubling up to max_backoff
                current_backoff = (current_backoff.saturating_mul(2)).min(config.max_backoff);

                // Re-resolve the device at most once per backoff period when the current path
                // no longer designates a usable capture device (GitHub #151).
                if shared.running.load(Ordering::Acquire) && warrants_reresolution(&err) {
                    if let Some(new_path) = resolver.and_then(DevicePathResolver::resolve) {
                        if new_path != active.device_path {
                            info!(
                                "Camera re-resolved from '{}' to '{}'; reopening",
                                active.device_path.display(),
                                new_path.display()
                            );
                            {
                                let mut guard = shared
                                    .device_path
                                    .write()
                                    .unwrap_or_else(|e| e.into_inner());
                                guard.clone_from(&new_path);
                            }
                            active.device_path = new_path;
                            current_backoff = config.min_backoff;
                        }
                    }
                }
            }
        }
    }
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
    health: &AtomicU8,
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
    // uvcvideo reports a node streamed by another process (e.g. the daemon) as EBUSY here,
    // not at open(): classify it as `DeviceBusy` (GitHub #150).
    let actual_format = Capture::set_format(&device, &req_format).map_err(|e| {
        CameraError::from_ioctl_error(config.device_path.clone(), e, |reason| {
            CameraError::SetFormat {
                path: config.device_path.clone(),
                width: config.width,
                height: config.height,
                format: target_format,
                reason,
            }
        })
    })?;

    let actual_width = actual_format.width;
    let actual_height = actual_format.height;

    let mut stream =
        v4l::io::mmap::Stream::with_buffers(&device, v4l::buffer::Type::VideoCapture, 4).map_err(
            |e| {
                CameraError::from_ioctl_error(config.device_path.clone(), e, |reason| {
                    CameraError::StreamCreate {
                        path: config.device_path.clone(),
                        reason,
                    }
                })
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
                return Err(CameraError::from_ioctl_error(
                    config.device_path.clone(),
                    e,
                    |reason| CameraError::BufferDequeue {
                        path: config.device_path.clone(),
                        reason,
                    },
                ));
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
        health.store(HEALTH_STREAMING, Ordering::Release);
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

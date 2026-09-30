//! V4L2 capture-path validation and streaming loop (GitHub #192, #193, #194).
//!
//! - **Driver format validation** (#192): `VIDIOC_S_FMT` may substitute the pixel format and pad
//!   rows. [`validate_negotiated_format`] adopts the driver-returned fourcc only when its memory
//!   layout is known, and records `bytesperline` so padded rows can be removed.
//! - **Captured buffer validation** (#192): [`validate_captured_buffer`] skips
//!   `V4L2_BUF_FLAG_ERROR` buffers, bounds `bytesused`, rejects short uncompressed buffers and
//!   de-strides padded rows, so every published [`Frame`] matches
//!   [`PixelFormat::expected_buffer_size`] exactly.
//! - **Frame rate** (#193): [`requested_frame_interval`] is sent with `VIDIOC_S_PARM`;
//!   `idle_fps` is a publication throttle shared with the mock ([`publish_due`],
//!   [`CameraConfig::publish_fps`]).
//! - **Bounded DQBUF wait** (#194): [`dqbuf_poll_timeout`] is clamped to 150-250 ms so that the
//!   capture thread re-checks its shutdown flag at least that often (Criterion C9); consecutive
//!   timeouts escalate to [`CameraError::Starved`] after [`MAX_STREAM_STALL`].

use crate::config::CameraConfig;
use crate::error::CameraError;
use crate::frame::{Frame, PixelFormat};
use crate::sensor::SensorType;
use arc_swap::ArcSwapOption;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, warn};

/// Lower bound of the DQBUF poll timeout.
pub const MIN_DQBUF_POLL_TIMEOUT: Duration = Duration::from_millis(150);
/// Upper bound of the DQBUF poll timeout: the capture thread never blocks longer than this
/// between two checks of its shutdown flag.
pub const MAX_DQBUF_POLL_TIMEOUT: Duration = Duration::from_millis(250);
/// Criterion C9: dropping a camera manager completes within this budget.
pub const C9_SHUTDOWN_BUDGET: Duration = Duration::from_millis(500);
/// Continuous time without a dequeued buffer after which the stream is reported
/// [`CameraError::Starved`] and the device is reopened (the pre-#194 single-poll tolerance).
pub const MAX_STREAM_STALL: Duration = Duration::from_millis(2000);
/// Consecutive rejected (corrupted, short or error-flagged) buffers after which the device is
/// reopened instead of silently dropping frames forever (about one second at 30 fps).
pub const MAX_CONSECUTIVE_REJECTED_FRAMES: u32 = 30;

/// Rate used when a configuration carries a zero frame rate (struct literal bypassing the
/// builder clamp).
const FALLBACK_FPS: u32 = 30;

fn effective_fps(fps: u32) -> u32 {
    if fps == 0 {
        FALLBACK_FPS
    } else {
        fps
    }
}

/// Returns the DQBUF poll timeout for a stream running at `fps`: three frame intervals,
/// clamped to [`MIN_DQBUF_POLL_TIMEOUT`]..=[`MAX_DQBUF_POLL_TIMEOUT`].
pub fn dqbuf_poll_timeout(fps: u32) -> Duration {
    let fps = u64::from(effective_fps(fps));
    let three_intervals_ms = 3_000u64.checked_div(fps).unwrap_or(0);
    Duration::from_millis(three_intervals_ms).clamp(MIN_DQBUF_POLL_TIMEOUT, MAX_DQBUF_POLL_TIMEOUT)
}

/// Returns how many consecutive poll timeouts of `poll_timeout` make up [`MAX_STREAM_STALL`]
/// (rounded up, at least 1).
pub fn max_consecutive_poll_timeouts(poll_timeout: Duration) -> u32 {
    let poll = poll_timeout.as_nanos();
    if poll == 0 {
        return 1;
    }
    let count = MAX_STREAM_STALL.as_nanos().div_ceil(poll).max(1);
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Frame interval `(numerator, denominator)` in seconds requested with `VIDIOC_S_PARM`.
pub fn requested_frame_interval(fps: u32) -> (u32, u32) {
    (1, effective_fps(fps))
}

/// Integer frame rate granted by the driver for a `numerator / denominator` second interval,
/// or `None` when the driver reported no usable interval.
pub fn granted_fps(numerator: u32, denominator: u32) -> Option<u32> {
    denominator.checked_div(numerator).filter(|fps| *fps > 0)
}

/// Returns whether a frame should be published given the time since the last published frame.
///
/// At the full rate every hardware frame is published. Below it (the `idle_fps` throttle),
/// a frame is published once one throttled interval has elapsed, with half a hardware frame
/// interval of tolerance for dequeue jitter.
pub fn publish_due(since_last: Option<Duration>, publish_fps: u32, full_fps: u32) -> bool {
    let full_fps = effective_fps(full_fps);
    if publish_fps == 0 || publish_fps >= full_fps {
        return true;
    }
    let Some(elapsed) = since_last else {
        return true;
    };
    let interval = Duration::from_micros(
        1_000_000u64
            .checked_div(u64::from(publish_fps))
            .unwrap_or(0),
    );
    let tolerance = Duration::from_micros(
        1_000_000u64
            .checked_div(u64::from(full_fps))
            .and_then(|us| us.checked_div(2))
            .unwrap_or(0),
    );
    elapsed.saturating_add(tolerance) >= interval
}

/// Capture format actually granted by the driver (`VIDIOC_S_FMT` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegotiatedFormat {
    /// Pixel format of the buffers the driver will deliver (the driver-returned fourcc).
    pub format: PixelFormat,
    /// Granted frame width in pixels.
    pub width: u32,
    /// Granted frame height in pixels.
    pub height: u32,
    /// Bytes between the starts of two rows; `0` for compressed formats.
    pub bytes_per_line: u32,
    /// Whether the driver substituted a different format than the one requested.
    pub substituted: bool,
}

/// Why a driver-returned format cannot be streamed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormatValidationError {
    /// The driver returned a fourcc whose memory layout is not supported.
    #[error("driver returned unsupported pixel format '{actual}' for requested '{requested}'")]
    UnsupportedFourcc {
        /// Requested fourcc.
        requested: String,
        /// Driver-returned fourcc (escaped).
        actual: String,
    },
    /// The driver returned a frame size that cannot be streamed with this format.
    #[error("driver returned invalid frame size {width}x{height} for {format:?}")]
    InvalidDimensions {
        /// Driver-returned format.
        format: PixelFormat,
        /// Driver-returned width.
        width: u32,
        /// Driver-returned height.
        height: u32,
    },
    /// The driver-returned stride cannot hold one row of pixels.
    #[error(
        "driver stride {bytes_per_line} is smaller than a {format:?} row of {row_bytes} bytes"
    )]
    StrideTooSmall {
        /// Driver-returned format.
        format: PixelFormat,
        /// Driver-returned `bytesperline`.
        bytes_per_line: u32,
        /// Bytes of pixel data in one row.
        row_bytes: usize,
    },
}

/// Why a dequeued buffer was not published.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FrameRejection {
    /// The driver flagged the buffer with `V4L2_BUF_FLAG_ERROR` (corrupted data).
    #[error("driver flagged the buffer as corrupted (V4L2_BUF_FLAG_ERROR)")]
    ErrorFlag,
    /// `bytesused` exceeds the mapped buffer.
    #[error("bytesused {bytesused} exceeds the mapped buffer of {capacity} bytes")]
    BytesUsedExceedsBuffer {
        /// Driver-reported `bytesused`.
        bytesused: u32,
        /// Mapped buffer length.
        capacity: usize,
    },
    /// The buffer is shorter than the negotiated frame.
    #[error("buffer holds {actual} bytes, the negotiated frame needs {expected}")]
    ShortBuffer {
        /// Bytes required by the negotiated layout.
        expected: usize,
        /// Bytes available.
        actual: usize,
    },
    /// A compressed buffer carries no payload.
    #[error("compressed buffer carries no payload")]
    EmptyPayload,
}

/// Maps a driver-returned fourcc to a [`PixelFormat`] whose memory layout matches exactly.
///
/// Stricter than [`crate::fourcc_to_pixel_format`]: `BGR3` (swapped channels) and `RGB4`
/// (four bytes per pixel) are never labelled [`PixelFormat::Rgb24`].
fn layout_pixel_format(fourcc: [u8; 4]) -> Option<PixelFormat> {
    match &fourcc {
        b"RGB3" => Some(PixelFormat::Rgb24),
        b"YUYV" => Some(PixelFormat::Yuyv),
        b"NV12" => Some(PixelFormat::Nv12),
        b"MJPG" => Some(PixelFormat::Mjpeg),
        b"GREY" | b"Y800" | b"Y8  " => Some(PixelFormat::Grey),
        _ => None,
    }
}

/// Bytes of pixel data in one row, or `None` for compressed formats.
fn row_bytes(format: PixelFormat, width: u32) -> Option<usize> {
    let width = usize::try_from(width).ok()?;
    match format {
        PixelFormat::Yuyv => width.checked_mul(2),
        PixelFormat::Rgb24 => width.checked_mul(3),
        PixelFormat::Grey | PixelFormat::Nv12 => Some(width),
        PixelFormat::Mjpeg => None,
    }
}

/// Number of `bytes_per_line` rows in a frame (NV12 adds a half-height chroma plane).
fn stride_rows(format: PixelFormat, height: u32) -> Option<usize> {
    let height = usize::try_from(height).ok()?;
    match format {
        PixelFormat::Nv12 => height.checked_add(height.checked_div(2)?),
        _ => Some(height),
    }
}

/// Validates the format returned by `VIDIOC_S_FMT` against the request.
///
/// A driver substitution is adopted (and the frames labelled with it) only when the returned
/// fourcc has a known layout; `bytes_per_line == 0` means tightly packed rows.
///
/// # Errors
///
/// [`FormatValidationError`] when the fourcc layout is unknown, a dimension is zero (or odd
/// for NV12), the frame size overflows, or the stride cannot hold a row.
pub fn validate_negotiated_format(
    requested: PixelFormat,
    actual_fourcc: [u8; 4],
    width: u32,
    height: u32,
    bytes_per_line: u32,
) -> Result<NegotiatedFormat, FormatValidationError> {
    let format = layout_pixel_format(actual_fourcc).ok_or_else(|| {
        FormatValidationError::UnsupportedFourcc {
            requested: requested.fourcc_str().to_string(),
            actual: actual_fourcc.as_slice().escape_ascii().to_string(),
        }
    })?;
    let invalid = FormatValidationError::InvalidDimensions {
        format,
        width,
        height,
    };
    if width == 0 || height == 0 {
        return Err(invalid);
    }
    if format == PixelFormat::Nv12 && (!width.is_multiple_of(2) || !height.is_multiple_of(2)) {
        return Err(invalid);
    }
    let bytes_per_line = match row_bytes(format, width) {
        None => 0,
        Some(row) => {
            let row_u32 = u32::try_from(row).map_err(|_| invalid.clone())?;
            let stride = if bytes_per_line == 0 {
                row_u32
            } else if bytes_per_line < row_u32 {
                return Err(FormatValidationError::StrideTooSmall {
                    format,
                    bytes_per_line,
                    row_bytes: row,
                });
            } else {
                bytes_per_line
            };
            let rows = stride_rows(format, height).ok_or_else(|| invalid.clone())?;
            usize::try_from(stride)
                .ok()
                .and_then(|s| s.checked_mul(rows))
                .ok_or(invalid)?;
            stride
        }
    };
    Ok(NegotiatedFormat {
        format,
        width,
        height,
        bytes_per_line,
        substituted: format != requested,
    })
}

/// Metadata of one dequeued buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferMeta {
    /// Driver-reported bytes of payload (`0`: not reported).
    pub bytesused: u32,
    /// Whether `V4L2_BUF_FLAG_ERROR` is set.
    pub error_flag: bool,
}

/// Validates one dequeued buffer against the negotiated layout and returns the frame payload.
///
/// Uncompressed payloads are returned tightly packed (padding removed), so their length is
/// exactly [`PixelFormat::expected_buffer_size`]; MJPEG payloads are returned as the first
/// `bytesused` bytes. No copy is made for a rejected buffer.
///
/// # Errors
///
/// [`FrameRejection`] when the buffer is error-flagged, `bytesused` exceeds the mapping, an
/// uncompressed buffer is short, or a compressed buffer is empty.
pub fn validate_captured_buffer(
    buf: &[u8],
    meta: BufferMeta,
    layout: &NegotiatedFormat,
) -> Result<Vec<u8>, FrameRejection> {
    if meta.error_flag {
        return Err(FrameRejection::ErrorFlag);
    }
    let used = usize::try_from(meta.bytesused).unwrap_or(usize::MAX);
    if used > buf.len() {
        return Err(FrameRejection::BytesUsedExceedsBuffer {
            bytesused: meta.bytesused,
            capacity: buf.len(),
        });
    }
    let payload = if used == 0 {
        buf
    } else {
        buf.get(..used).unwrap_or(buf)
    };

    let Some(row) = row_bytes(layout.format, layout.width) else {
        if used == 0 || payload.is_empty() {
            return Err(FrameRejection::EmptyPayload);
        }
        return Ok(payload.to_vec());
    };

    let short = |expected: usize| FrameRejection::ShortBuffer {
        expected,
        actual: payload.len(),
    };
    let rows = stride_rows(layout.format, layout.height).unwrap_or(usize::MAX);
    let stride = usize::try_from(layout.bytes_per_line).unwrap_or(usize::MAX);
    let tight = row.checked_mul(rows).ok_or_else(|| short(usize::MAX))?;
    // The last row's padding is optional; its pixels are not.
    let required = rows
        .checked_sub(1)
        .and_then(|r| r.checked_mul(stride))
        .and_then(|b| b.checked_add(row))
        .ok_or_else(|| short(usize::MAX))?;
    if payload.len() < required {
        return Err(short(required));
    }
    if stride == row {
        return payload
            .get(..tight)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| short(tight));
    }
    let mut out = Vec::with_capacity(tight);
    for chunk in payload.chunks(stride).take(rows) {
        out.extend_from_slice(chunk.get(..row).ok_or_else(|| short(required))?);
    }
    Ok(out)
}

/// How the streaming loop ended without an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamExit {
    /// Shutdown was requested.
    Shutdown,
    /// The idle timeout elapsed: release the device (auto-standby).
    Suspend,
}

/// Abstraction over an MMAP capture stream, implemented by the V4L2 stream and by test doubles.
pub(crate) trait CaptureSource {
    /// Waits up to `timeout` for a filled buffer; `Ok(false)` on timeout.
    fn wait_ready(&mut self, timeout: Duration) -> io::Result<bool>;
    /// Re-queues the previously returned buffer and dequeues the next filled one (the first
    /// call queues every buffer and starts streaming).
    fn next_buffer(&mut self) -> io::Result<(&[u8], BufferMeta)>;
    /// Dequeues one filled buffer without re-queuing, discarding it. Used after
    /// [`CaptureSource::next_buffer`] timed out: its re-queued buffer is still owned by the
    /// driver, so calling `next_buffer` again would queue it twice.
    fn resync(&mut self) -> io::Result<()>;
}

/// Shared state the streaming loop publishes into.
pub(crate) struct StreamTargets<'a> {
    pub(crate) latest_frame: &'a ArcSwapOption<Frame>,
    pub(crate) is_ready: &'a AtomicBool,
    pub(crate) running: &'a AtomicBool,
    pub(crate) last_activity: &'a RwLock<Instant>,
    pub(crate) health: &'a AtomicU8,
    /// Encoded health value stored after each published frame.
    pub(crate) streaming_health: u8,
}

/// Per-stream parameters of the streaming loop.
pub(crate) struct StreamSettings<'a> {
    pub(crate) config: &'a CameraConfig,
    pub(crate) device_path: &'a Path,
    pub(crate) layout: NegotiatedFormat,
    pub(crate) sensor_type: SensorType,
    pub(crate) poll_timeout: Duration,
}

fn dequeue_error(path: &Path, err: io::Error) -> CameraError {
    CameraError::from_ioctl_error(path.to_path_buf(), err, |reason| {
        CameraError::BufferDequeue {
            path: path.to_path_buf(),
            reason,
        }
    })
}

/// Streams frames from `source` until shutdown, auto-standby or an error.
///
/// Never blocks longer than `settings.poll_timeout` between two checks of `running`.
pub(crate) fn run_capture_loop<S: CaptureSource + ?Sized>(
    source: &mut S,
    settings: &StreamSettings<'_>,
    targets: &StreamTargets<'_>,
    clock_ns: &dyn Fn() -> u64,
) -> Result<StreamExit, CameraError> {
    let config = settings.config;
    let stall_limit = max_consecutive_poll_timeouts(settings.poll_timeout);
    let mut stalls: u32 = 0;
    let mut rejected: u32 = 0;
    let mut started = false;
    let mut resync_pending = false;
    let mut warmup_discarded: usize = 0;
    let mut sequence: u64 = 0;
    let mut last_published: Option<Instant> = None;

    // Records one poll timeout: Shutdown when requested, Starved once the stall budget is spent.
    let on_timeout = |stalls: &mut u32| -> Option<Result<StreamExit, CameraError>> {
        if !targets.running.load(Ordering::Acquire) {
            return Some(Ok(StreamExit::Shutdown));
        }
        *stalls = stalls.saturating_add(1);
        (*stalls >= stall_limit).then_some(Err(CameraError::Starved))
    };

    while targets.running.load(Ordering::Acquire) {
        if resync_pending {
            match source.resync() {
                Ok(()) => {
                    resync_pending = false;
                    stalls = 0;
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                    if let Some(exit) = on_timeout(&mut stalls) {
                        return exit;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(dequeue_error(settings.device_path, e)),
            }
            continue;
        }

        if started {
            match source.wait_ready(settings.poll_timeout) {
                Ok(true) => {}
                Ok(false) => {
                    if let Some(exit) = on_timeout(&mut stalls) {
                        return exit;
                    }
                    continue;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(dequeue_error(settings.device_path, e)),
            }
        }

        let next = source.next_buffer();
        started = true;
        let (buf, meta) = match next {
            Ok(val) => val,
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                resync_pending = true;
                if let Some(exit) = on_timeout(&mut stalls) {
                    return exit;
                }
                continue;
            }
            Err(e) => return Err(dequeue_error(settings.device_path, e)),
        };
        stalls = 0;

        // Discard initial frames for auto-exposure convergence.
        if warmup_discarded < config.warmup_frames {
            warmup_discarded = warmup_discarded.saturating_add(1);
            targets.is_ready.store(false, Ordering::Release);
            continue;
        }

        let idle_elapsed = targets
            .last_activity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed();
        // Idle auto-standby: release the device handle to extinguish the privacy LED.
        if !config.idle_timeout.is_zero() && idle_elapsed > config.idle_timeout {
            return Ok(StreamExit::Suspend);
        }

        // idle_fps publication throttle (same rule as the mock), checked before any copy.
        if !publish_due(
            last_published.map(|t| t.elapsed()),
            config.publish_fps(idle_elapsed),
            config.fps,
        ) {
            continue;
        }

        let data = match validate_captured_buffer(buf, meta, &settings.layout) {
            Ok(data) => {
                if rejected > 0 {
                    debug!(
                        "Camera '{}' recovered after {} rejected frame(s)",
                        settings.device_path.display(),
                        rejected
                    );
                }
                rejected = 0;
                data
            }
            Err(rejection) => {
                rejected = rejected.saturating_add(1);
                if rejected == 1 {
                    warn!(
                        "Dropping invalid frame from '{}': {}",
                        settings.device_path.display(),
                        rejection
                    );
                }
                if rejected >= MAX_CONSECUTIVE_REJECTED_FRAMES {
                    return Err(CameraError::BufferDequeue {
                        path: settings.device_path.to_path_buf(),
                        reason: format!(
                            "{rejected} consecutive invalid frames (last: {rejection})"
                        ),
                    });
                }
                continue;
            }
        };

        let frame = Frame::new(
            data,
            settings.layout.width,
            settings.layout.height,
            clock_ns(),
            settings.layout.format,
            sequence,
        )
        .with_sensor_type(settings.sensor_type);

        targets.latest_frame.store(Some(Arc::new(frame)));
        // Publish readiness with Release after storing the frame.
        targets.is_ready.store(true, Ordering::Release);
        targets
            .health
            .store(targets.streaming_health, Ordering::Release);
        sequence = sequence.saturating_add(1);
        last_published = Some(Instant::now());
    }

    Ok(StreamExit::Shutdown)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Unit tests use direct assertions on scripted capture sources"
)]
mod tests {
    use super::*;
    use crate::config::CameraConfigBuilder;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Arc;

    const STREAMING: u8 = 1;

    /// One scripted outcome of a [`CaptureSource`] call.
    enum Step {
        /// `next_buffer` returns this buffer.
        Frame(Vec<u8>, BufferMeta),
        /// `next_buffer` (or `resync`) times out inside the driver poll.
        DequeueTimeout,
        /// `wait_ready` times out.
        PollTimeout,
        /// `resync` succeeds.
        Resynced,
        /// Clears `running` and times out `wait_ready` (shutdown requested mid-stall).
        StopDuringPoll,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Wait(Duration),
        Next,
        Resync,
    }

    struct ScriptedSource {
        steps: VecDeque<Step>,
        calls: Vec<Call>,
        running: Arc<AtomicBool>,
        current: Vec<u8>,
    }

    impl ScriptedSource {
        fn new(steps: Vec<Step>, running: Arc<AtomicBool>) -> Self {
            Self {
                steps: steps.into(),
                calls: Vec::new(),
                running,
                current: Vec::new(),
            }
        }

        /// When the script is exhausted the stream stalls and shutdown is requested.
        fn stop(&self) {
            self.running.store(false, Ordering::Release);
        }
    }

    impl CaptureSource for ScriptedSource {
        fn wait_ready(&mut self, timeout: Duration) -> io::Result<bool> {
            self.calls.push(Call::Wait(timeout));
            match self.steps.front() {
                Some(Step::PollTimeout) => {
                    self.steps.pop_front();
                    Ok(false)
                }
                Some(Step::StopDuringPoll) | None => {
                    self.steps.pop_front();
                    self.stop();
                    Ok(false)
                }
                Some(_) => Ok(true),
            }
        }

        fn next_buffer(&mut self) -> io::Result<(&[u8], BufferMeta)> {
            self.calls.push(Call::Next);
            match self.steps.pop_front() {
                Some(Step::Frame(data, meta)) => {
                    self.current = data;
                    Ok((self.current.as_slice(), meta))
                }
                Some(Step::DequeueTimeout) => {
                    Err(io::Error::new(io::ErrorKind::TimedOut, "VIDIOC_DQBUF"))
                }
                _ => {
                    self.stop();
                    Err(io::Error::new(io::ErrorKind::TimedOut, "VIDIOC_DQBUF"))
                }
            }
        }

        fn resync(&mut self) -> io::Result<()> {
            self.calls.push(Call::Resync);
            match self.steps.pop_front() {
                Some(Step::Resynced) => Ok(()),
                Some(Step::DequeueTimeout) => {
                    Err(io::Error::new(io::ErrorKind::TimedOut, "VIDIOC_DQBUF"))
                }
                _ => {
                    self.stop();
                    Err(io::Error::new(io::ErrorKind::TimedOut, "VIDIOC_DQBUF"))
                }
            }
        }
    }

    struct Harness {
        latest: ArcSwapOption<Frame>,
        ready: AtomicBool,
        running: Arc<AtomicBool>,
        activity: RwLock<Instant>,
        health: AtomicU8,
        config: CameraConfig,
        path: PathBuf,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                latest: ArcSwapOption::empty(),
                ready: AtomicBool::new(false),
                running: Arc::new(AtomicBool::new(true)),
                activity: RwLock::new(Instant::now()),
                health: AtomicU8::new(0),
                config: CameraConfigBuilder::new()
                    .warmup_frames(0)
                    .idle_timeout(Duration::ZERO)
                    .build(),
                path: PathBuf::from("/dev/video-scripted"),
            }
        }

        fn run(
            &self,
            steps: Vec<Step>,
            layout: NegotiatedFormat,
        ) -> (Result<StreamExit, CameraError>, ScriptedSource) {
            let mut source = ScriptedSource::new(steps, Arc::clone(&self.running));
            let settings = StreamSettings {
                config: &self.config,
                device_path: &self.path,
                layout,
                sensor_type: SensorType::Rgb,
                poll_timeout: MAX_DQBUF_POLL_TIMEOUT,
            };
            let targets = StreamTargets {
                latest_frame: &self.latest,
                is_ready: &self.ready,
                running: &self.running,
                last_activity: &self.activity,
                health: &self.health,
                streaming_health: STREAMING,
            };
            let result = run_capture_loop(&mut source, &settings, &targets, &|| 42);
            (result, source)
        }
    }

    fn yuyv_4x2() -> NegotiatedFormat {
        validate_negotiated_format(PixelFormat::Yuyv, *b"YUYV", 4, 2, 0).unwrap()
    }

    fn good(fill: u8) -> Step {
        Step::Frame(
            vec![fill; 16],
            BufferMeta {
                bytesused: 16,
                error_flag: false,
            },
        )
    }

    #[test]
    fn test_error_flag_frame_skipped() {
        let h = Harness::new();
        let flagged = Step::Frame(
            vec![0xEE; 16],
            BufferMeta {
                bytesused: 16,
                error_flag: true,
            },
        );
        let (result, _) = h.run(vec![flagged], yuyv_4x2());
        assert_eq!(result.unwrap(), StreamExit::Shutdown);
        assert!(
            h.latest.load_full().is_none(),
            "an error-flagged frame must never be published"
        );
        assert!(!h.ready.load(Ordering::Acquire));

        let h = Harness::new();
        let flagged = Step::Frame(
            vec![0xEE; 16],
            BufferMeta {
                bytesused: 16,
                error_flag: true,
            },
        );
        let (result, _) = h.run(vec![flagged, good(7)], yuyv_4x2());
        assert_eq!(result.unwrap(), StreamExit::Shutdown);
        let frame = h.latest.load_full().expect("the valid frame is published");
        assert_eq!(frame.data, vec![7; 16]);
        assert_eq!(frame.sequence, 0, "skipped frames never consume a sequence");
        assert_eq!(frame.format, PixelFormat::Yuyv);
        assert_eq!(frame.sensor_type, SensorType::Rgb);
        assert!(h.ready.load(Ordering::Acquire));
        assert_eq!(h.health.load(Ordering::Acquire), STREAMING);
    }

    #[test]
    fn test_short_frame_never_published() {
        let h = Harness::new();
        let short = Step::Frame(
            vec![1; 15],
            BufferMeta {
                bytesused: 15,
                error_flag: false,
            },
        );
        let (result, _) = h.run(vec![short], yuyv_4x2());
        assert_eq!(result.unwrap(), StreamExit::Shutdown);
        assert!(h.latest.load_full().is_none());
    }

    #[test]
    fn test_padded_frame_published_tightly_packed() {
        let h = Harness::new();
        let layout = validate_negotiated_format(PixelFormat::Yuyv, *b"YUYV", 4, 2, 12).unwrap();
        let mut data = vec![1u8; 8];
        data.extend_from_slice(&[0xEE; 4]);
        data.extend_from_slice(&[2u8; 8]);
        data.extend_from_slice(&[0xEE; 4]);
        let step = Step::Frame(
            data,
            BufferMeta {
                bytesused: 24,
                error_flag: false,
            },
        );
        let (result, _) = h.run(vec![step], layout);
        assert_eq!(result.unwrap(), StreamExit::Shutdown);
        let frame = h.latest.load_full().unwrap();
        let mut expected = vec![1u8; 8];
        expected.extend_from_slice(&[2u8; 8]);
        assert_eq!(frame.data, expected);
    }

    #[test]
    fn test_consecutive_invalid_frames_reopen_device() {
        let h = Harness::new();
        let steps = (0..MAX_CONSECUTIVE_REJECTED_FRAMES)
            .map(|_| {
                Step::Frame(
                    vec![0; 4],
                    BufferMeta {
                        bytesused: 4,
                        error_flag: false,
                    },
                )
            })
            .collect();
        let (result, _) = h.run(steps, yuyv_4x2());
        let err = result.expect_err("a stream of invalid frames must reopen the device");
        assert!(
            matches!(err, CameraError::BufferDequeue { .. }),
            "unexpected error {err:?}"
        );
        assert!(h.latest.load_full().is_none());
    }

    #[test]
    fn test_poll_timeouts_escalate_to_starved() {
        let h = Harness::new();
        let limit = max_consecutive_poll_timeouts(MAX_DQBUF_POLL_TIMEOUT);
        let mut steps = vec![good(1)];
        steps.extend((0..limit).map(|_| Step::PollTimeout));
        let (result, source) = h.run(steps, yuyv_4x2());
        assert!(
            matches!(result, Err(CameraError::Starved)),
            "a sustained stall must be reported as Starved, got {result:?}"
        );
        let waits: Vec<_> = source
            .calls
            .iter()
            .filter_map(|c| match c {
                Call::Wait(t) => Some(*t),
                _ => None,
            })
            .collect();
        assert_eq!(waits.len(), limit as usize);
        assert!(
            waits.iter().all(|t| *t <= MAX_DQBUF_POLL_TIMEOUT),
            "every wait is bounded by the 250 ms poll timeout"
        );
    }

    #[test]
    fn test_frame_between_timeouts_resets_stall_budget() {
        let h = Harness::new();
        let limit = max_consecutive_poll_timeouts(MAX_DQBUF_POLL_TIMEOUT);
        let mut steps = vec![good(1)];
        steps.extend((1..limit).map(|_| Step::PollTimeout));
        steps.push(good(2));
        steps.extend((1..limit).map(|_| Step::PollTimeout));
        steps.push(Step::StopDuringPoll);
        let (result, _) = h.run(steps, yuyv_4x2());
        assert_eq!(result.unwrap(), StreamExit::Shutdown);
        assert_eq!(h.latest.load_full().unwrap().data, vec![2; 16]);
    }

    #[test]
    fn test_shutdown_observed_after_one_bounded_poll() {
        let h = Harness::new();
        let (result, source) = h.run(vec![good(1), Step::StopDuringPoll], yuyv_4x2());
        assert_eq!(result.unwrap(), StreamExit::Shutdown);
        assert_eq!(
            source.calls,
            vec![Call::Next, Call::Wait(MAX_DQBUF_POLL_TIMEOUT)],
            "the loop must exit right after the first poll that follows the stop request"
        );
    }

    #[test]
    fn test_dequeue_timeout_resyncs_without_requeue() {
        let h = Harness::new();
        let steps = vec![
            // First buffer never arrives within the driver poll (slow STREAMON).
            Step::DequeueTimeout,
            // Still nothing on the direct dequeue.
            Step::DequeueTimeout,
            // The pending buffer is dequeued (and discarded) without a second QBUF.
            Step::Resynced,
            good(5),
        ];
        let (result, source) = h.run(steps, yuyv_4x2());
        assert_eq!(result.unwrap(), StreamExit::Shutdown);
        assert_eq!(
            source.calls[..5],
            [
                Call::Next,
                Call::Resync,
                Call::Resync,
                Call::Wait(MAX_DQBUF_POLL_TIMEOUT),
                Call::Next,
            ],
            "after a dequeue timeout next_buffer must not be called before a resync"
        );
        assert_eq!(h.latest.load_full().unwrap().data, vec![5; 16]);
    }

    #[test]
    fn test_warmup_frames_are_discarded_before_validation() {
        let mut h = Harness::new();
        h.config.warmup_frames = 2;
        let (result, _) = h.run(vec![good(1), good(2), good(3)], yuyv_4x2());
        assert_eq!(result.unwrap(), StreamExit::Shutdown);
        assert_eq!(h.latest.load_full().unwrap().data, vec![3; 16]);
    }

    #[test]
    fn test_idle_timeout_suspends_stream() {
        let mut h = Harness::new();
        h.config.idle_timeout = Duration::from_millis(1);
        *h.activity.write().unwrap() = Instant::now() - Duration::from_secs(1);
        let (result, _) = h.run(vec![good(1)], yuyv_4x2());
        assert_eq!(result.unwrap(), StreamExit::Suspend);
    }

    #[test]
    fn test_idle_fps_throttles_publication() {
        let mut h = Harness::new();
        h.config.fps = 30;
        h.config.idle_fps = 5;
        h.config.idle_timeout = Duration::from_secs(10);
        // Past half the idle timeout: throttled to idle_fps.
        *h.activity.write().unwrap() = Instant::now() - Duration::from_secs(6);
        let (result, _) = h.run(vec![good(1), good(2), good(3)], yuyv_4x2());
        assert_eq!(result.unwrap(), StreamExit::Shutdown);
        let frame = h.latest.load_full().unwrap();
        assert_eq!(
            frame.data,
            vec![1; 16],
            "back-to-back frames inside one idle interval are not published"
        );
        assert_eq!(frame.sequence, 0);
    }
}

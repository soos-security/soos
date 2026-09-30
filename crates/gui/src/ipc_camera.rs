//! IPC Camera Manager proxying live preview frames from `soos-daemon` over Unix domain socket.
//!
//! Authorization contract (GitHub #143): the daemon serves `RequestKind::PreviewFrame` only to
//! a root peer or to a UID explicitly listed in its `[preview]` configuration. A refusal arrives
//! as a standard `Response` (`ProtocolError`) bound to the request nonce; this client surfaces it
//! as [`IpcPreviewError::Unauthorized`] and stops polling instead of hammering the daemon.

use arc_swap::ArcSwapOption;
use soos_camera_v4l::{
    CameraErrorKind, CameraManager, CameraStatus, CameraStatusCell, Frame, PixelFormat,
};
use soos_protocol::codec::{decode, decode_preview, encode};
use soos_protocol::types::{
    PreviewResponse, ReasonClass, Request, RequestId, RequestKind, Response, Verdict,
    CURRENT_VERSION, MAX_MESSAGE_SIZE, MAX_PREVIEW_MESSAGE_SIZE,
};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Read timeout applied to every preview exchange with the daemon.
const PREVIEW_READ_TIMEOUT: Duration = Duration::from_millis(2500);
/// Write timeout applied to every preview exchange with the daemon.
const PREVIEW_WRITE_TIMEOUT: Duration = Duration::from_millis(500);
/// Poll interval targeting roughly 30 preview frames per second.
const PREVIEW_POLL_INTERVAL: Duration = Duration::from_millis(33);
/// Back-off applied after the daemon reported a rate-limit refusal.
const RATE_LIMIT_BACKOFF: Duration = Duration::from_millis(250);
/// Back-off applied while the daemon reports itself unavailable.
const UNAVAILABLE_BACKOFF: Duration = Duration::from_millis(100);
/// Delay between reconnection attempts after a transport failure.
const RECONNECT_DELAY: Duration = Duration::from_millis(200);

/// Failure reported by the daemon (or the transport) for a preview request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IpcPreviewError {
    /// The daemon refused to serve preview frames to this peer (`[preview]` policy).
    #[error("soos-daemon reported this user is not authorized to receive the camera preview")]
    Unauthorized,
    /// The daemon throttled this peer (`[preview] max_requests_per_sec`).
    #[error("soos-daemon rate-limited the camera preview requests")]
    RateLimited,
    /// The daemon accepted the request but could not serve it (clock or internal error).
    #[error("soos-daemon reported the camera preview as unavailable")]
    Unavailable,
    /// The reply violated the wire protocol (bounds, codec or nonce binding).
    #[error("soos-daemon returned a malformed camera preview reply")]
    Protocol,
    /// Socket connection, read or write failure.
    #[error("I/O error while talking to the soos-daemon socket")]
    Io,
}

impl IpcPreviewError {
    /// Classifies this failure for the user-visible camera status (CAM-07).
    pub fn kind(self) -> CameraErrorKind {
        match self {
            Self::Unauthorized => CameraErrorKind::SourceUnauthorized,
            Self::RateLimited => CameraErrorKind::SourceRateLimited,
            Self::Unavailable => CameraErrorKind::SourceUnavailable,
            Self::Protocol => CameraErrorKind::SourceProtocol,
            Self::Io => CameraErrorKind::SourceUnreachable,
        }
    }
}

/// Camera manager implementation querying real-time video frames from `soos-daemon`
/// via the `/run/soos/daemon.sock` IPC socket.
pub struct IpcCameraManager {
    latest_frame: Arc<ArcSwapOption<Frame>>,
    is_ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<IpcPreviewError>>>,
    status: Arc<CameraStatusCell>,
    worker_handle: Option<JoinHandle<()>>,
}

impl IpcCameraManager {
    /// Spawns a background thread streaming preview frames from the specified Unix domain socket path.
    pub fn spawn<P: AsRef<Path>>(socket_path: P) -> Self {
        let path = socket_path.as_ref().to_path_buf();
        let latest_frame = Arc::new(ArcSwapOption::empty());
        let is_ready = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let last_error = Arc::new(Mutex::new(None));
        let status = Arc::new(CameraStatusCell::new());

        let worker = WorkerState {
            latest_frame: Arc::clone(&latest_frame),
            is_ready: Arc::clone(&is_ready),
            running: Arc::clone(&running),
            last_error: Arc::clone(&last_error),
            status: Arc::clone(&status),
        };

        let handle = thread::Builder::new()
            .name("soos-gui-ipc-cam".to_string())
            .spawn(move || {
                run_ipc_camera_worker(path, &worker);
            })
            .ok();

        Self {
            latest_frame,
            is_ready,
            running,
            last_error,
            status,
            worker_handle: handle,
        }
    }

    /// Spawns an IPC camera manager pointing to the default daemon socket `/run/soos/daemon.sock`.
    pub fn spawn_default() -> Self {
        Self::spawn("/run/soos/daemon.sock")
    }

    /// Probes whether the daemon socket is alive and accepting connections.
    pub fn probe<P: AsRef<Path>>(socket_path: P) -> bool {
        UnixStream::connect(socket_path).is_ok()
    }

    /// Performs one authorization round-trip: connects, sends a single preview request and
    /// reports whether the daemon is willing to serve this user (an empty preview counts as
    /// authorized, a `Response` refusal does not).
    ///
    /// # Errors
    ///
    /// Returns the [`IpcPreviewError`] describing why the preview stream is not usable.
    pub fn probe_preview<P: AsRef<Path>>(socket_path: P) -> Result<(), IpcPreviewError> {
        let mut stream = UnixStream::connect(socket_path).map_err(|_| IpcPreviewError::Io)?;
        configure_stream(&stream);
        let uid = nix::unistd::getuid().as_raw();
        request_preview(&mut stream, uid).map(|_| ())
    }

    /// Returns the most recent failure reported by the daemon or the transport, if any.
    /// Cleared as soon as a frame is successfully received.
    pub fn last_error(&self) -> Option<IpcPreviewError> {
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl CameraManager for IpcCameraManager {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        if !self.is_ready() {
            return None;
        }
        self.latest_frame.load_full()
    }

    fn is_ready(&self) -> bool {
        self.is_ready.load(Ordering::Acquire)
    }

    fn notify_activity(&self) {
        // Querying preview frames continuously wakes daemon camera activity on demand.
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

impl Drop for IpcCameraManager {
    fn drop(&mut self) {
        self.stop();
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Shared state between the manager and its polling worker thread.
struct WorkerState {
    latest_frame: Arc<ArcSwapOption<Frame>>,
    is_ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<IpcPreviewError>>>,
    status: Arc<CameraStatusCell>,
}

impl WorkerState {
    fn set_error(&self, error: Option<IpcPreviewError>) {
        let previous = std::mem::replace(
            &mut *self.last_error.lock().unwrap_or_else(|e| e.into_inner()),
            error,
        );
        match error {
            Some(err) => {
                self.status.record_error(err.kind());
                if previous != Some(err) {
                    tracing::warn!(error = %err, "soos-daemon camera preview failure");
                }
            }
            None => {
                self.status.set(CameraStatus::Ready);
                if previous.is_some() {
                    tracing::info!("soos-daemon camera preview recovered");
                }
            }
        }
    }

    fn not_ready(&self) {
        self.is_ready.store(false, Ordering::Release);
    }
}

fn configure_stream(stream: &UnixStream) {
    let _ = stream.set_read_timeout(Some(PREVIEW_READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PREVIEW_WRITE_TIMEOUT));
}

/// Generates a fresh 256-bit request nonce binding the daemon reply to this request.
fn fresh_request_id() -> Result<RequestId, IpcPreviewError> {
    let mut nonce: RequestId = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|_| IpcPreviewError::Io)?;
    Ok(nonce)
}

/// Sends one `PreviewFrame` request and decodes the reply.
///
/// A daemon refusal is a standard `Response` bound to the request nonce and is mapped to the
/// matching [`IpcPreviewError`]; any other reply must decode as a bounded `PreviewResponse`.
fn request_preview(stream: &mut UnixStream, uid: u32) -> Result<PreviewResponse, IpcPreviewError> {
    let request_id = fresh_request_id()?;
    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::PreviewFrame,
        request_id,
        uid_hint: uid,
        service: "soos-gui".to_string(),
        deadline_monotonic_ns: u64::MAX,
    };
    let encoded_req = encode(&req).map_err(|_| IpcPreviewError::Protocol)?;
    stream
        .write_all(&encoded_req)
        .and_then(|()| stream.flush())
        .map_err(|_| IpcPreviewError::Io)?;

    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .map_err(|_| IpcPreviewError::Io)?;
    let declared_size =
        usize::try_from(u32::from_be_bytes(len_bytes)).map_err(|_| IpcPreviewError::Protocol)?;
    if declared_size == 0 || declared_size > MAX_PREVIEW_MESSAGE_SIZE {
        return Err(IpcPreviewError::Protocol);
    }

    let total_size = declared_size.saturating_add(4);
    let mut buf = vec![0u8; total_size];
    if let Some(prefix) = buf.get_mut(..4) {
        prefix.copy_from_slice(&len_bytes);
    }
    let payload_slot = buf.get_mut(4..).ok_or(IpcPreviewError::Protocol)?;
    stream
        .read_exact(payload_slot)
        .map_err(|_| IpcPreviewError::Io)?;

    // A refusal is a small standard `Response` echoing our nonce.
    if declared_size <= MAX_MESSAGE_SIZE {
        if let Ok(resp) = decode::<Response>(&buf) {
            if resp.matches_request(&request_id) {
                return Err(map_refusal(&resp));
            }
        }
    }

    decode_preview::<PreviewResponse>(&buf).map_err(|_| IpcPreviewError::Protocol)
}

fn map_refusal(resp: &Response) -> IpcPreviewError {
    match resp.verdict {
        Verdict::ProtocolError if resp.reason_class == ReasonClass::RateLimited => {
            IpcPreviewError::RateLimited
        }
        Verdict::ProtocolError | Verdict::Deny => IpcPreviewError::Unauthorized,
        Verdict::Unavailable => IpcPreviewError::Unavailable,
        Verdict::Allow => IpcPreviewError::Protocol,
    }
}

fn pixel_format_from_wire(format: u8) -> PixelFormat {
    match format {
        1 => PixelFormat::Grey,
        2 => PixelFormat::Yuyv,
        3 => PixelFormat::Nv12,
        4 => PixelFormat::Mjpeg,
        _ => PixelFormat::Rgb24,
    }
}

/// Outcome of one polling iteration, deciding how the worker loop continues.
enum PollStep {
    /// Keep polling on the same connection after the given delay.
    Continue(Duration),
    /// Drop the connection and reconnect after `RECONNECT_DELAY`.
    Reconnect,
    /// Stop the worker permanently (authorization refused).
    Stop,
}

fn poll_once(stream: &mut UnixStream, uid: u32, state: &WorkerState) -> PollStep {
    match request_preview(stream, uid) {
        Ok(mut resp) => {
            if resp.width > 0 && resp.height > 0 && !resp.data.is_empty() {
                let frame = Frame::new(
                    std::mem::take(&mut resp.data),
                    resp.width,
                    resp.height,
                    resp.timestamp_monotonic_ns,
                    pixel_format_from_wire(resp.format),
                    resp.sequence,
                );
                state.latest_frame.store(Some(Arc::new(frame)));
                state.is_ready.store(true, Ordering::Release);
                state.set_error(None);
            }
            PollStep::Continue(PREVIEW_POLL_INTERVAL)
        }
        Err(IpcPreviewError::Unauthorized) => {
            state.set_error(Some(IpcPreviewError::Unauthorized));
            state.not_ready();
            PollStep::Stop
        }
        Err(IpcPreviewError::RateLimited) => {
            state.set_error(Some(IpcPreviewError::RateLimited));
            PollStep::Continue(RATE_LIMIT_BACKOFF)
        }
        Err(IpcPreviewError::Unavailable) => {
            state.set_error(Some(IpcPreviewError::Unavailable));
            state.not_ready();
            PollStep::Continue(UNAVAILABLE_BACKOFF)
        }
        Err(err @ (IpcPreviewError::Protocol | IpcPreviewError::Io)) => {
            state.set_error(Some(err));
            state.not_ready();
            PollStep::Reconnect
        }
    }
}

fn run_ipc_camera_worker(socket_path: PathBuf, state: &WorkerState) {
    let uid = nix::unistd::getuid().as_raw();

    while state.running.load(Ordering::Acquire) {
        let mut stream = match UnixStream::connect(&socket_path) {
            Ok(s) => s,
            Err(_) => {
                state.set_error(Some(IpcPreviewError::Io));
                state.not_ready();
                thread::sleep(RECONNECT_DELAY);
                continue;
            }
        };
        configure_stream(&stream);

        while state.running.load(Ordering::Acquire) {
            match poll_once(&mut stream, uid, state) {
                PollStep::Continue(delay) => thread::sleep(delay),
                PollStep::Reconnect => break,
                PollStep::Stop => {
                    state.running.store(false, Ordering::Release);
                    break;
                }
            }
        }

        drop(stream);
        state.not_ready();
        if state.running.load(Ordering::Acquire) {
            thread::sleep(UNAVAILABLE_BACKOFF);
        }
    }
}

//! Daemon preview client of the live camera view (ADR 2026-10-07, architect spec §5).
//!
//! `soos-remote` never opens a video device: it asks `soos-daemon` for its latest preview
//! frame over `/run/soos/daemon.sock` with `RequestKind::PreviewFrame` (`soos-protocol`).
//! At most one daemon connection exists at any instant; it is reused across frames,
//! replaced before the daemon's lifetime and request caps with a graceful close (write
//! shutdown, bounded wait for the daemon's EOF, then connect), and retried once at once when
//! a reused connection turns out to be idle-closed. The daemon peer must be root. Every
//! reply is bounded before allocation and held in a zeroized buffer. No logging here.

use std::future::Future;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use soos_protocol::codec::{decode, decode_preview};
use soos_protocol::message::encode_request;
use soos_protocol::types::{
    PreviewResponse, ReasonClass, Request, RequestId, RequestKind, Response, Verdict,
    CURRENT_VERSION, MAX_MESSAGE_SIZE, MAX_PREVIEW_MESSAGE_SIZE, MAX_RESPONSE_FUTURE_SKEW_NS,
    REQUEST_ID_LEN,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::{timeout, timeout_at, Instant};
use zeroize::Zeroizing;

use crate::auth::{system_random, RandomSource};
use crate::{
    CAMERA_DAEMON_CLOSE_WAIT_MS, CAMERA_DAEMON_CONNECT_TIMEOUT_MS, CAMERA_DAEMON_IO_TIMEOUT_MS,
    CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION, CAMERA_DAEMON_RECONNECT_AFTER_MS,
    CAMERA_DAEMON_SERVICE,
};

/// Size of the fixed sink that drains a connection being closed.
const CLOSE_SINK_BYTES: usize = 64;

/// One preview frame copied out of a `PreviewResponse`. No `Debug`, no `Clone`.
pub struct PreviewFrame {
    /// Daemon frame sequence.
    pub sequence: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `soos_protocol::types::PREVIEW_FORMAT_*`.
    pub format: u8,
    /// Pixels, moved out of the reply (zeroized on drop).
    pub data: Zeroizing<Vec<u8>>,
}

/// Why a frame could not be fetched. Fixed texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PreviewError {
    /// Daemon refusal (`ProtocolError`/`Deny` other than `RateLimited`). Ends the view.
    #[error("preview refused by the daemon")]
    Refused,
    /// `ProtocolError` + `RateLimited`: back off, keep the view.
    #[error("preview rate limited")]
    RateLimited,
    /// `Verdict::Unavailable`.
    #[error("daemon unavailable")]
    Unavailable,
    /// Connect, read or write failure or timeout; daemon peer not root.
    #[error("daemon connection failed")]
    Io,
    /// Malformed, oversize, stale or unbound reply; unknown verdict.
    #[error("daemon protocol error")]
    Protocol,
}

/// Seam of the view loop (tests inject a scripted source).
pub trait PreviewSource: Send {
    /// Fetches the latest frame. Cancel-safe: a dropped future drops the daemon connection.
    fn next_frame(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<PreviewFrame, PreviewError>> + Send + '_>>;
}

/// Builds one source per view (production: [`DaemonPreviewClient`]).
pub type PreviewSourceFactory = Arc<dyn Fn() -> Box<dyn PreviewSource> + Send + Sync>;

/// One open daemon connection.
struct Conn {
    stream: UnixStream,
    opened_at: Instant,
    requests: u32,
}

/// Production client: at most one daemon connection.
pub struct DaemonPreviewClient {
    socket_path: PathBuf,
    uid: u32,
    expected_daemon_uid: u32,
    conn: Option<Conn>,
    /// The crate CSPRNG (request nonces).
    random: RandomSource,
}

/// Failure of one exchange attempt.
enum AttemptError {
    /// Write failure, or EOF/reset before the first reply byte (idle close of the daemon).
    Retryable,
    /// Any other failure.
    Final(PreviewError),
}

impl DaemonPreviewClient {
    /// A client of the daemon at `socket_path` for the own `uid`; the daemon must be root.
    #[must_use]
    pub fn new(socket_path: PathBuf, uid: u32) -> Self {
        Self {
            socket_path,
            uid,
            expected_daemon_uid: 0,
            conn: None,
            random: system_random(),
        }
    }

    /// Test hook: the expected daemon peer uid.
    #[doc(hidden)]
    #[must_use]
    pub fn with_expected_daemon_uid(mut self, uid: u32) -> Self {
        self.expected_daemon_uid = uid;
        self
    }

    /// Connects and checks the daemon peer uid, bounded by `CAMERA_DAEMON_CONNECT_TIMEOUT_MS`.
    async fn connect(&self) -> Result<Conn, PreviewError> {
        let attempt = async {
            let stream = UnixStream::connect(&self.socket_path)
                .await
                .map_err(|_| PreviewError::Io)?;
            let cred = stream.peer_cred().map_err(|_| PreviewError::Io)?;
            if cred.uid() != self.expected_daemon_uid {
                return Err(PreviewError::Io);
            }
            Ok(stream)
        };
        let stream = timeout(
            Duration::from_millis(CAMERA_DAEMON_CONNECT_TIMEOUT_MS),
            attempt,
        )
        .await
        .map_err(|_| PreviewError::Io)??;
        Ok(Conn {
            stream,
            opened_at: Instant::now(),
            requests: 0,
        })
    }

    /// Whether `conn` may serve one more request.
    fn reusable(conn: &Conn) -> bool {
        conn.opened_at.elapsed() < Duration::from_millis(CAMERA_DAEMON_RECONNECT_AFTER_MS)
            && conn.requests < CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION
    }

    /// One `next_frame` (spec §5.2).
    async fn exchange(&mut self) -> Result<PreviewFrame, PreviewError> {
        let (mut conn, reused) = match self.conn.take() {
            Some(conn) if Self::reusable(&conn) => (conn, true),
            Some(old) => {
                graceful_close(old.stream).await;
                (self.connect().await?, false)
            }
            None => (self.connect().await?, false),
        };
        // One IO budget for the attempt and its single immediate retry (plan O2).
        let started = Instant::now();
        let deadline = started
            .checked_add(Duration::from_millis(CAMERA_DAEMON_IO_TIMEOUT_MS))
            .unwrap_or(started);
        let mut outcome = attempt(&mut conn.stream, self.uid, &self.random, deadline).await;
        if reused && matches!(outcome, Err(AttemptError::Retryable)) {
            drop(conn);
            conn = self.connect().await?;
            outcome = attempt(&mut conn.stream, self.uid, &self.random, deadline).await;
        }
        let result = match outcome {
            Ok(result) => result,
            Err(AttemptError::Retryable) => Err(PreviewError::Io),
            Err(AttemptError::Final(err)) => Err(err),
        };
        match result {
            Ok(_)
            | Err(PreviewError::Refused | PreviewError::RateLimited | PreviewError::Unavailable) => {
                conn.requests = conn.requests.saturating_add(1);
                self.conn = Some(conn);
            }
            Err(PreviewError::Io | PreviewError::Protocol) => {}
        }
        result
    }
}

impl PreviewSource for DaemonPreviewClient {
    fn next_frame(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<PreviewFrame, PreviewError>> + Send + '_>> {
        Box::pin(self.exchange())
    }
}

/// Half-closes `stream`, then drains it into a fixed sink until the daemon's EOF, at most
/// `CAMERA_DAEMON_CLOSE_WAIT_MS` (the daemon releases its per-UID permit before it closes).
async fn graceful_close(mut stream: UnixStream) {
    if stream.shutdown().await.is_err() {
        return;
    }
    let mut sink = [0u8; CLOSE_SINK_BYTES];
    let drain = async {
        loop {
            match stream.read(&mut sink).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    };
    let _ = timeout(Duration::from_millis(CAMERA_DAEMON_CLOSE_WAIT_MS), drain).await;
}

/// `CLOCK_MONOTONIC` in nanoseconds; 0 (rejected by `check_freshness`) on failure.
fn monotonic_now_ns() -> u64 {
    nix::time::clock_gettime(nix::time::ClockId::CLOCK_MONOTONIC)
        .ok()
        .and_then(|ts| {
            let secs = u64::try_from(ts.tv_sec()).ok()?;
            let nanos = u64::try_from(ts.tv_nsec()).ok()?;
            Some(secs.saturating_mul(1_000_000_000).saturating_add(nanos))
        })
        .unwrap_or(0)
}

/// Maps a daemon refusal bound to the request.
fn map_refusal(resp: &Response) -> PreviewError {
    match resp.verdict {
        Verdict::ProtocolError if resp.reason_class == ReasonClass::RateLimited => {
            PreviewError::RateLimited
        }
        Verdict::ProtocolError | Verdict::Deny => PreviewError::Refused,
        Verdict::Unavailable => PreviewError::Unavailable,
        Verdict::Allow => PreviewError::Protocol,
    }
}

/// One request/reply on `stream` under `deadline`.
async fn attempt(
    stream: &mut UnixStream,
    uid: u32,
    random: &RandomSource,
    deadline: Instant,
) -> Result<Result<PreviewFrame, PreviewError>, AttemptError> {
    let mut request_id: RequestId = [0u8; REQUEST_ID_LEN];
    random(&mut request_id).map_err(|_| AttemptError::Final(PreviewError::Io))?;
    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::PreviewFrame,
        request_id,
        uid_hint: uid,
        service: CAMERA_DAEMON_SERVICE.to_string(),
        deadline_monotonic_ns: u64::MAX,
    };
    let encoded = encode_request(&req).map_err(|_| AttemptError::Final(PreviewError::Protocol))?;
    timeout_at(deadline, exchange_on(stream, &encoded, &request_id))
        .await
        .map_err(|_| AttemptError::Final(PreviewError::Io))?
}

/// Writes the request and reads one bounded reply.
async fn exchange_on(
    stream: &mut UnixStream,
    encoded: &[u8],
    request_id: &RequestId,
) -> Result<Result<PreviewFrame, PreviewError>, AttemptError> {
    if stream.write_all(encoded).await.is_err() || stream.flush().await.is_err() {
        return Err(AttemptError::Retryable);
    }

    let mut len_bytes = [0u8; 4];
    let mut got = 0usize;
    while got < len_bytes.len() {
        let slot = len_bytes.get_mut(got..).unwrap_or_default();
        match stream.read(slot).await {
            Ok(0) if got == 0 => return Err(AttemptError::Retryable),
            Ok(0) => return Err(AttemptError::Final(PreviewError::Io)),
            Ok(n) => got = got.saturating_add(n),
            Err(err)
                if got == 0
                    && matches!(
                        err.kind(),
                        ErrorKind::ConnectionReset
                            | ErrorKind::ConnectionAborted
                            | ErrorKind::BrokenPipe
                    ) =>
            {
                return Err(AttemptError::Retryable);
            }
            Err(_) => return Err(AttemptError::Final(PreviewError::Io)),
        }
    }
    let declared = usize::try_from(u32::from_be_bytes(len_bytes))
        .map_err(|_| AttemptError::Final(PreviewError::Protocol))?;
    if declared == 0 || declared > MAX_PREVIEW_MESSAGE_SIZE {
        return Ok(Err(PreviewError::Protocol));
    }
    let total = declared.saturating_add(len_bytes.len());
    // Exactly the declared size, allocated once, wiped on drop (camera pixels).
    let mut buf = Zeroizing::new(vec![0u8; total]);
    if let Some(prefix) = buf.get_mut(..len_bytes.len()) {
        prefix.copy_from_slice(&len_bytes);
    }
    let payload = buf
        .get_mut(len_bytes.len()..)
        .ok_or(AttemptError::Final(PreviewError::Protocol))?;
    if stream.read_exact(payload).await.is_err() {
        return Err(AttemptError::Final(PreviewError::Io));
    }
    Ok(decode_reply(&buf, request_id))
}

/// Decodes a framed reply: a refusal bound to the request, or a preview frame.
fn decode_reply(buf: &[u8], request_id: &RequestId) -> Result<PreviewFrame, PreviewError> {
    if buf.len() <= MAX_MESSAGE_SIZE.saturating_add(4) {
        if let Ok(resp) = decode::<Response>(buf) {
            if resp.matches_request(request_id) {
                if resp
                    .check_freshness(monotonic_now_ns(), MAX_RESPONSE_FUTURE_SKEW_NS)
                    .is_err()
                {
                    return Err(PreviewError::Protocol);
                }
                return Err(map_refusal(&resp));
            }
        }
    }
    let mut resp: PreviewResponse = decode_preview(buf).map_err(|_| PreviewError::Protocol)?;
    Ok(PreviewFrame {
        sequence: resp.sequence,
        width: resp.width,
        height: resp.height,
        format: resp.format,
        data: Zeroizing::new(std::mem::take(&mut resp.data)),
    })
}

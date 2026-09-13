//! Synchronous IPC client for communication between PAM and the soos daemon.
//!
//! # Latency and Concurrency Invariants
//!
//! - Strictly uses synchronous `std::os::unix::net::UnixStream`. Zero async runtime.
//! - Read and write timeouts are configured before socket transactions.
//! - Total authentication budget: 200–250ms (governed by [`PamConfig::timeout_ms`]).
//! - Telemetry event budget: strict 20ms maximum.
//! - Sockets are closed immediately upon receiving the verdict.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    Event, EventKind, Request, RequestKind, Response, Verdict, CURRENT_VERSION, MAX_MESSAGE_SIZE,
    REQUEST_ID_LEN,
};

use crate::config::PamConfig;

/// Maximum timeout budget dedicated to telemetry event notification (milliseconds).
pub const EVENT_TIMEOUT_MS: u64 = 20;

/// IPC communication error types. All variants fail closed into `PAM_IGNORE`.
#[derive(Debug)]
pub enum IpcError {
    /// Socket connection failure.
    Connect(std::io::Error),
    /// Socket read, write, or timeout failure.
    Io(std::io::Error),
    /// Protocol serialization or deserialization failure.
    Codec(soos_protocol::codec::CodecError),
    /// Cryptographic random number generator failure.
    Random(getrandom::Error),
    /// Communication deadline expired.
    Timeout,
    /// Response declared length is zero.
    EmptyResponse,
    /// Response declared length exceeds maximum permitted buffer size.
    OversizedMessage { size: usize, max: usize },
    /// Response request ID did not match the original nonce.
    RequestIdMismatch,
    /// Received response version does not match supported protocol version.
    UnsupportedVersion { version: u8 },
}

impl core::fmt::Display for IpcError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "failed to connect to daemon socket: {e}"),
            Self::Io(e) => write!(f, "socket I/O error: {e}"),
            Self::Codec(e) => write!(f, "protocol codec error: {e}"),
            Self::Random(e) => write!(f, "failed to generate random request ID: {e}"),
            Self::Timeout => write!(f, "daemon communication deadline exceeded"),
            Self::EmptyResponse => write!(f, "daemon returned empty zero-length response"),
            Self::OversizedMessage { size, max } => {
                write!(f, "response size {size} exceeds maximum {max}")
            }
            Self::RequestIdMismatch => {
                write!(f, "response request_id does not match initial request")
            }
            Self::UnsupportedVersion { version } => {
                write!(f, "unsupported response version {version}")
            }
        }
    }
}

impl std::error::Error for IpcError {}

/// Computes remaining time before deadline or returns `IpcError::Timeout`.
///
/// Filters out zero-duration timeouts since `set_read_timeout` / `set_write_timeout`
/// reject `Duration::ZERO` with `EINVAL` in Rust standard library.
fn remaining_budget(start: Instant, total: Duration) -> Result<Duration, IpcError> {
    let elapsed = start.elapsed();
    total
        .checked_sub(elapsed)
        .filter(|d| !d.is_zero())
        .ok_or(IpcError::Timeout)
}

/// Maps std::io::Error to IpcError, converting socket timeout errors to `IpcError::Timeout`.
fn map_io_err(err: std::io::Error) -> IpcError {
    match err.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => IpcError::Timeout,
        _ => IpcError::Io(err),
    }
}

/// Sends an authentication request to the daemon and awaits the verification verdict.
pub fn authenticate(config: &PamConfig, uid: u32) -> Result<Verdict, IpcError> {
    let start_time = Instant::now();
    let total_timeout = Duration::from_millis(config.timeout_ms);

    let mut stream = UnixStream::connect(&config.socket_path).map_err(IpcError::Connect)?;

    // Configure write timeout based on remaining latency budget
    let remaining_write = remaining_budget(start_time, total_timeout)?;
    stream
        .set_write_timeout(Some(remaining_write))
        .map_err(map_io_err)?;

    // Generate single-use cryptographic 256-bit nonce
    let mut request_id = [0u8; REQUEST_ID_LEN];
    getrandom::fill(&mut request_id).map_err(IpcError::Random)?;

    let now_ns = monotonic_nanos();
    let timeout_ns = config.timeout_ms.saturating_mul(1_000_000);
    let deadline_monotonic_ns = now_ns.saturating_add(timeout_ns);

    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id,
        uid_hint: uid,
        service: config.service.clone(),
        deadline_monotonic_ns,
    };

    let encoded = encode(&req).map_err(IpcError::Codec)?;
    stream.write_all(&encoded).map_err(map_io_err)?;
    stream.flush().map_err(map_io_err)?;

    // Dynamic latency budget refresh: configure read timeout before reading response length prefix
    let remaining_for_len = remaining_budget(start_time, total_timeout)?;
    stream
        .set_read_timeout(Some(remaining_for_len))
        .map_err(map_io_err)?;

    // Read 4-byte big-endian length prefix
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).map_err(map_io_err)?;

    let declared_size =
        usize::try_from(u32::from_be_bytes(len_buf)).map_err(|_| IpcError::OversizedMessage {
            size: usize::MAX,
            max: MAX_MESSAGE_SIZE,
        })?;

    if declared_size == 0 {
        return Err(IpcError::EmptyResponse);
    }

    if declared_size > MAX_MESSAGE_SIZE {
        return Err(IpcError::OversizedMessage {
            size: declared_size,
            max: MAX_MESSAGE_SIZE,
        });
    }

    let total_capacity = declared_size
        .checked_add(4)
        .ok_or(IpcError::OversizedMessage {
            size: usize::MAX,
            max: MAX_MESSAGE_SIZE,
        })?;

    // Single-allocation framed response buffer avoiding redundant secondary Vec and memcpy
    let mut full_buf = vec![0u8; total_capacity];
    let prefix_slice = full_buf.get_mut(..4).ok_or(IpcError::EmptyResponse)?;
    prefix_slice.copy_from_slice(&len_buf);

    // Dynamic latency budget refresh: recompute remaining budget before reading payload body
    // to strictly enforce cumulative deadline across multi-part reads
    let remaining_for_body = remaining_budget(start_time, total_timeout)?;
    stream
        .set_read_timeout(Some(remaining_for_body))
        .map_err(map_io_err)?;

    let body_slice = full_buf.get_mut(4..).ok_or(IpcError::EmptyResponse)?;
    stream.read_exact(body_slice).map_err(map_io_err)?;

    let resp: Response = decode(&full_buf).map_err(IpcError::Codec)?;

    if resp.version != CURRENT_VERSION {
        return Err(IpcError::UnsupportedVersion {
            version: resp.version,
        });
    }

    if resp.request_id != req.request_id {
        return Err(IpcError::RequestIdMismatch);
    }

    // Explicit drop: socket is closed immediately after response receipt
    drop(stream);

    Ok(resp.verdict)
}

/// Transmits a best-effort telemetry event notification to the daemon within 20ms.
pub fn notify_event(config: &PamConfig, _uid: u32, event_kind: EventKind) -> Result<(), IpcError> {
    let start_time = Instant::now();
    let total_timeout = Duration::from_millis(EVENT_TIMEOUT_MS);

    let mut stream = UnixStream::connect(&config.socket_path).map_err(IpcError::Connect)?;

    let remaining = remaining_budget(start_time, total_timeout)?;

    stream
        .set_write_timeout(Some(remaining))
        .map_err(map_io_err)?;

    let event = Event {
        version: CURRENT_VERSION,
        kind: event_kind,
        request_id: None,
        service: config.service.clone(),
        timestamp_monotonic_ns: monotonic_nanos(),
    };

    let encoded = encode(&event).map_err(IpcError::Codec)?;
    stream.write_all(&encoded).map_err(map_io_err)?;
    stream.flush().map_err(map_io_err)?;

    // Fire-and-forget: socket closed immediately
    drop(stream);

    Ok(())
}

/// Returns the current monotonic clock timestamp in nanoseconds.
fn monotonic_nanos() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` points to a valid stack-allocated timespec struct passed to clock_gettime.
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    if ret == 0 && ts.tv_sec >= 0 && ts.tv_nsec >= 0 {
        let sec_ns = ts.tv_sec.cast_unsigned().saturating_mul(1_000_000_000);
        sec_ns.saturating_add(ts.tv_nsec.cast_unsigned())
    } else {
        0
    }
}

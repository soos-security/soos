//! Synchronous IPC client for communication between PAM and the soos daemon.
//!
//! # Latency and Concurrency Invariants
//!
//! - Strictly uses synchronous `std::os::unix::net::UnixStream`. Zero async runtime.
//! - One cumulative deadline per exchange: `SO_RCVTIMEO` / `SO_SNDTIMEO` are re-armed with
//!   the remaining budget before EVERY `read()` / `write()` syscall, and a verdict that
//!   completes after the deadline is discarded (review PAM-02, GitHub #173).
//! - Total authentication budget governed by the clamped [`PamConfig::timeout_ms`].
//! - Telemetry event budget: strict 20ms maximum.
//! - Sockets are closed immediately upon receiving the verdict.

use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use soos_protocol::codec::decode;
use soos_protocol::message::{encode_event, encode_request};
use soos_protocol::types::{
    Event, EventKind, ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION,
    MAX_MESSAGE_SIZE, REQUEST_ID_LEN,
};
use zeroize::Zeroizing;

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
    /// Response stream was truncated before complete frame was received.
    TruncatedResponse { expected: usize, received: usize },
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
            Self::TruncatedResponse { expected, received } => {
                write!(
                    f,
                    "truncated response: expected {expected} bytes, received {received}"
                )
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

/// Absolute deadline shared by every blocking operation of one exchange.
///
/// `SO_RCVTIMEO` / `SO_SNDTIMEO` bound a single syscall, not an exchange: a peer that
/// sends or accepts one byte at a time would otherwise get the full timeout again on
/// every `read()` / `write()` (review PAM-02, GitHub #173). Every syscall below therefore
/// re-arms the socket timeout with the budget that is left.
///
/// The CLOCK_MONOTONIC deadline sent to the daemon is captured once, together with the
/// `Instant` the client measures its own budget from (review PAM-10, GitHub #222): time
/// spent before the request is written (connect, NSS lookup) shrinks the daemon's budget
/// instead of pushing its deadline past the instant the client gives up.
#[derive(Clone, Copy, Debug)]
pub struct ExchangeDeadline {
    start: Instant,
    total: Duration,
    deadline_monotonic_ns: u64,
}

impl ExchangeDeadline {
    /// Starts a budget of `timeout_ms` milliseconds now.
    pub fn start(timeout_ms: u64) -> Self {
        let start = Instant::now();
        let now_ns = monotonic_nanos();
        Self {
            start,
            total: Duration::from_millis(timeout_ms),
            deadline_monotonic_ns: now_ns.saturating_add(timeout_ms.saturating_mul(1_000_000)),
        }
    }

    /// Budget left, or `IpcError::Timeout` once it is spent (never `Duration::ZERO`).
    pub fn remaining(&self) -> Result<Duration, IpcError> {
        remaining_budget(self.start, self.total)
    }

    /// Absolute CLOCK_MONOTONIC deadline (nanoseconds) fixed when the budget started.
    pub fn monotonic_deadline_ns(&self) -> u64 {
        self.deadline_monotonic_ns
    }
}

/// Converts a remaining budget into a `poll()` timeout in milliseconds, rounding UP.
///
/// Truncation turned a sub-millisecond budget into `poll(.., 0)`, which returns at once and
/// produced a spurious `IpcError::Timeout` (review PAM-10, GitHub #222). `Duration::ZERO`
/// maps to 0 and values above `c_int::MAX` milliseconds saturate.
pub fn poll_timeout_ms(remaining: Duration) -> libc::c_int {
    let millis = remaining.as_nanos().div_ceil(1_000_000);
    libc::c_int::try_from(millis).unwrap_or(libc::c_int::MAX)
}

/// Reads exactly `buf.len()` bytes before `deadline`, returning `IpcError::Timeout` once
/// the cumulative budget is spent and `IpcError::TruncatedResponse` on early EOF.
fn read_exact_before_deadline(
    stream: &mut UnixStream,
    buf: &mut [u8],
    expected_total: usize,
    already_received: usize,
    deadline: ExchangeDeadline,
) -> Result<(), IpcError> {
    let mut offset = 0usize;
    while offset < buf.len() {
        let remaining = deadline.remaining()?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(map_io_err)?;
        let remaining_slice = buf.get_mut(offset..).ok_or(IpcError::EmptyResponse)?;
        match stream.read(remaining_slice) {
            Ok(0) => {
                let received = already_received.saturating_add(offset);
                return Err(IpcError::TruncatedResponse {
                    expected: expected_total,
                    received,
                });
            }
            Ok(n) => {
                offset = offset.saturating_add(n);
            }
            // Retrying is bounded: the next iteration re-checks the deadline.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(map_io_err(e)),
        }
    }
    Ok(())
}

/// Writes all of `bytes` before `deadline` (same per-syscall re-arming as reads).
fn write_all_before_deadline(
    stream: &mut UnixStream,
    bytes: &[u8],
    deadline: ExchangeDeadline,
) -> Result<(), IpcError> {
    let mut offset = 0usize;
    while offset < bytes.len() {
        let remaining = deadline.remaining()?;
        stream
            .set_write_timeout(Some(remaining))
            .map_err(map_io_err)?;
        let pending = bytes.get(offset..).ok_or(IpcError::EmptyResponse)?;
        match stream.write(pending) {
            Ok(0) => {
                return Err(IpcError::Io(std::io::Error::from(
                    std::io::ErrorKind::WriteZero,
                )))
            }
            Ok(n) => {
                offset = offset.saturating_add(n);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(map_io_err(e)),
        }
    }
    Ok(())
}

/// RAII guard ensuring a raw file descriptor is closed on drop unless disarmed via `into_raw()`.
struct FdGuard(RawFd);

impl FdGuard {
    fn into_raw(mut self) -> RawFd {
        let fd = self.0;
        self.0 = -1;
        fd
    }
}

impl Drop for FdGuard {
    fn drop(&mut self) {
        if self.0 >= 0 {
            // SAFETY: self.0 is an owned, open file descriptor.
            unsafe { libc::close(self.0) };
        }
    }
}

/// Connects to a Unix domain socket at `path` within a non-blocking timeout budget.
///
/// Implements non-blocking `connect()` with POSIX `poll` to ensure frozen or
/// saturated daemon listening sockets do not block indefinitely. Upon successful
/// connection, the socket is switched back to blocking mode for subsequent timed I/O.
pub fn connect_with_timeout(path: &Path, timeout: Duration) -> Result<UnixStream, IpcError> {
    let path_bytes = path.as_os_str().as_bytes();
    if path_bytes.len() >= 108 {
        return Err(IpcError::Connect(std::io::Error::from_raw_os_error(
            libc::ENAMETOOLONG,
        )));
    }

    // SAFETY: Creating a non-blocking, close-on-exec UNIX domain stream socket.
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if fd < 0 {
        return Err(IpcError::Connect(std::io::Error::last_os_error()));
    }
    let guard = FdGuard(fd);

    let mut sun = libc::sockaddr_un {
        sun_family: libc::sa_family_t::try_from(libc::AF_UNIX).unwrap_or(0),
        sun_path: [0; 108],
    };

    for (dest, src) in sun.sun_path.iter_mut().zip(path_bytes.iter()) {
        *dest = i8::from_ne_bytes([*src]);
    }

    let sun_len = std::mem::size_of::<libc::sa_family_t>()
        .saturating_add(path_bytes.len())
        .saturating_add(1);

    // SAFETY: guard.0 is a valid non-blocking socket descriptor; sun is an initialized sockaddr_un struct.
    let ret = unsafe {
        libc::connect(
            guard.0,
            &sun as *const libc::sockaddr_un as *const libc::sockaddr,
            libc::socklen_t::try_from(sun_len).unwrap_or(0),
        )
    };

    if ret < 0 {
        let err = std::io::Error::last_os_error();
        let raw_err = err.raw_os_error().unwrap_or(0);
        if raw_err == libc::EINPROGRESS {
            let start = Instant::now();
            loop {
                let remaining = remaining_budget(start, timeout)?;
                let timeout_ms = poll_timeout_ms(remaining);

                let mut pfd = libc::pollfd {
                    fd: guard.0,
                    events: libc::POLLOUT,
                    revents: 0,
                };

                // SAFETY: pfd points to 1 valid stack-allocated pollfd.
                let poll_ret = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
                if poll_ret == 0 {
                    return Err(IpcError::Timeout);
                }
                if poll_ret < 0 {
                    let poll_err = std::io::Error::last_os_error();
                    if poll_err.raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    return Err(IpcError::Io(poll_err));
                }

                // Poll returned > 0: inspect SO_ERROR via getsockopt
                let mut so_err: libc::c_int = 0;
                let mut so_err_len =
                    libc::socklen_t::try_from(std::mem::size_of::<libc::c_int>()).unwrap_or(0);
                // SAFETY: guard.0 is valid; so_err points to a stack-allocated c_int.
                let opt_ret = unsafe {
                    libc::getsockopt(
                        guard.0,
                        libc::SOL_SOCKET,
                        libc::SO_ERROR,
                        &mut so_err as *mut _ as *mut libc::c_void,
                        &mut so_err_len,
                    )
                };
                if opt_ret < 0 {
                    return Err(IpcError::Io(std::io::Error::last_os_error()));
                }
                if so_err != 0 {
                    return Err(IpcError::Connect(std::io::Error::from_raw_os_error(so_err)));
                }

                if (pfd.revents & (libc::POLLERR | libc::POLLHUP)) != 0
                    && (pfd.revents & libc::POLLOUT) == 0
                {
                    return Err(IpcError::Connect(std::io::Error::from_raw_os_error(
                        libc::ECONNREFUSED,
                    )));
                }

                break;
            }
        } else {
            return Err(IpcError::Connect(err));
        }
    }

    let raw_fd = guard.into_raw();
    // SAFETY: raw_fd is an owned, valid, connected UNIX domain socket file descriptor.
    let stream = unsafe { UnixStream::from_raw_fd(raw_fd) };

    // Restore blocking mode so that subsequent read/write calls adhere to set_read_timeout/set_write_timeout
    stream.set_nonblocking(false).map_err(map_io_err)?;

    Ok(stream)
}

/// Sends an authentication request to the daemon and awaits the verification verdict and reason.
///
/// The budget is the clamped `timeout_ms`, starting now; see [`authenticate_before`].
pub fn authenticate(config: &PamConfig, uid: u32) -> Result<(Verdict, ReasonClass), IpcError> {
    authenticate_before(config, uid, ExchangeDeadline::start(config.timeout_ms))
}

/// [`authenticate`], calling `on_connected` once the daemon socket is connected and
/// before the request is sent (GitHub #221).
///
/// `on_connected` is never called when the connection fails (daemon not installed or not
/// running), so the caller only announces a face lookup that can actually happen. It
/// runs inside the cumulative deadline: time it spends is deducted from the budget left
/// for the exchange, which therefore stays bounded by the clamped `timeout_ms`.
pub fn authenticate_with_progress<F: FnOnce()>(
    config: &PamConfig,
    uid: u32,
    on_connected: F,
) -> Result<(Verdict, ReasonClass), IpcError> {
    authenticate_before_with_progress(
        config,
        uid,
        ExchangeDeadline::start(config.timeout_ms),
        on_connected,
    )
}

/// Runs one authentication exchange bounded by an already started `deadline`.
///
/// Connect, write and read all consume the same budget, and the request carries exactly
/// [`ExchangeDeadline::monotonic_deadline_ns`], so the daemon never works past the instant
/// the client gives up (GitHub #222).
pub fn authenticate_before(
    config: &PamConfig,
    uid: u32,
    deadline: ExchangeDeadline,
) -> Result<(Verdict, ReasonClass), IpcError> {
    authenticate_before_with_progress(config, uid, deadline, || {})
}

/// [`authenticate_before`], calling `on_connected` once the daemon socket is connected
/// (see [`authenticate_with_progress`]; GitHub #221 / #222).
pub fn authenticate_before_with_progress<F: FnOnce()>(
    config: &PamConfig,
    uid: u32,
    deadline: ExchangeDeadline,
    on_connected: F,
) -> Result<(Verdict, ReasonClass), IpcError> {
    let connect_timeout = deadline.remaining()?;
    let mut stream = connect_with_timeout(&config.socket_path, connect_timeout)?;
    on_connected();

    // Generate single-use cryptographic 256-bit nonce
    let mut request_id = Zeroizing::new([0u8; REQUEST_ID_LEN]);
    getrandom::fill(&mut *request_id).map_err(IpcError::Random)?;

    // Captured before connect together with the client's own budget (GitHub #222).
    let deadline_monotonic_ns = deadline.monotonic_deadline_ns();

    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: *request_id,
        uid_hint: uid,
        service: config.service.clone(),
        deadline_monotonic_ns,
    };

    let encoded = Zeroizing::new(encode_request(&req).map_err(IpcError::Codec)?);
    write_all_before_deadline(&mut stream, &encoded, deadline)?;
    drop(encoded);

    // Read 4-byte big-endian length prefix with byte-counted completeness validation;
    // every read() re-arms SO_RCVTIMEO with the cumulative budget left.
    let mut len_buf = Zeroizing::new([0u8; 4]);
    read_exact_before_deadline(&mut stream, &mut *len_buf, 4, 0, deadline)?;

    let declared_size =
        usize::try_from(u32::from_be_bytes(*len_buf)).map_err(|_| IpcError::OversizedMessage {
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
    let mut full_buf = Zeroizing::new(vec![0u8; total_capacity]);
    let prefix_slice = full_buf.get_mut(..4).ok_or(IpcError::EmptyResponse)?;
    prefix_slice.copy_from_slice(&*len_buf);

    let body_slice = full_buf.get_mut(4..).ok_or(IpcError::EmptyResponse)?;
    // Completeness: the counted read returns `TruncatedResponse` on any early EOF, so a
    // successful read always holds the whole declared frame (GitHub #264).
    read_exact_before_deadline(&mut stream, body_slice, total_capacity, 4, deadline)?;

    let resp: Response = decode(&full_buf).map_err(IpcError::Codec)?;

    if resp.version != CURRENT_VERSION {
        return Err(IpcError::UnsupportedVersion {
            version: resp.version,
        });
    }

    // Replay protection: the fresh single-use 256-bit nonce must match bit-for-bit. The
    // response timestamps are informational and deliberately not validated here (ADR
    // 2026-09-30 "Response Timestamps Are Informational", GitHub #219). The predicate is
    // owned by the protocol crate (`Response::matches_request`, GitHub #264).
    if !resp.matches_request(&req.request_id) {
        return Err(IpcError::RequestIdMismatch);
    }

    // Explicit drop: socket is closed immediately after response receipt
    drop(stream);

    // A verdict that completed after the deadline is never honored (fail closed).
    deadline.remaining()?;

    Ok((resp.verdict, resp.reason_class))
}

/// Transmits a best-effort telemetry event notification to the daemon within 20ms.
pub fn notify_event(config: &PamConfig, uid: u32, event_kind: EventKind) -> Result<(), IpcError> {
    let deadline = ExchangeDeadline::start(EVENT_TIMEOUT_MS);

    let connect_timeout = deadline.remaining()?;
    let mut stream = connect_with_timeout(&config.socket_path, connect_timeout)?;

    let event = Event {
        version: CURRENT_VERSION,
        kind: event_kind,
        request_id: None,
        uid: Some(uid),
        service: config.service.clone(),
        timestamp_monotonic_ns: monotonic_nanos(),
    };

    let encoded = Zeroizing::new(encode_event(&event).map_err(IpcError::Codec)?);
    write_all_before_deadline(&mut stream, &encoded, deadline)?;

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

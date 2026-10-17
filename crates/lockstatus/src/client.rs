//! Bounded blocking `RequestKind::LockState` exchange (one request per connection).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use nix::time::{clock_gettime, ClockId};
use soos_protocol::{
    decode, encode_request, LockStateResponse, Request, RequestKind, Response, CURRENT_VERSION,
    MAX_MESSAGE_SIZE, REQUEST_ID_LEN,
};

use crate::view::LockView;
use crate::CLIENT_SERVICE;

/// Why one exchange produced no lock state.
#[derive(Debug, thiserror::Error)]
pub enum LockStatusError {
    /// The socket could not be reached (missing, permission denied, refused).
    #[error("daemon socket unreachable")]
    Connect(#[source] std::io::Error),
    /// A read or write failed (including an old daemon closing the connection).
    #[error("daemon socket I/O failed")]
    Io(#[source] std::io::Error),
    /// The cumulative deadline expired.
    #[error("lock state exchange timed out")]
    Timeout,
    /// The response length prefix is zero or above `MAX_MESSAGE_SIZE`.
    #[error("invalid response frame length {0}")]
    FrameLength(usize),
    /// The request could not be encoded.
    #[error("lock state request could not be encoded")]
    Encode,
    /// The response is neither a lock state nor a refusal.
    #[error("undecodable daemon response")]
    Decode,
    /// The daemon answered a standard `Response` (a refusal, whatever its verdict).
    #[error("daemon refused the lock state request")]
    Refused,
    /// The lock state carries another protocol version.
    #[error("unsupported lock state version {0}")]
    Version(u8),
    /// `CLOCK_MONOTONIC` could not be read.
    #[error("monotonic clock unavailable")]
    Clock,
    /// No random request identifier could be drawn.
    #[error("random request id unavailable")]
    Random,
}

/// One `LockState` exchange with `uid_hint = uid` on a fresh connection, under one cumulative
/// `budget` for connect, write and read; the connection is closed afterwards.
///
/// # Errors
///
/// Every failure is a [`LockStatusError`]; none is ever turned into a state.
pub fn query_lock_state(
    socket: &Path,
    uid: u32,
    budget: Duration,
) -> Result<LockStateResponse, LockStatusError> {
    let deadline = Deadline::start(budget);
    deadline.remaining_or_timeout()?;
    let deadline_monotonic_ns = monotonic_now_ns()
        .ok_or(LockStatusError::Clock)?
        .saturating_add(u64::try_from(budget.as_nanos()).unwrap_or(u64::MAX));
    let mut request_id = [0u8; REQUEST_ID_LEN];
    getrandom::fill(&mut request_id).map_err(|_| LockStatusError::Random)?;
    let request = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::LockState,
        request_id,
        uid_hint: uid,
        service: CLIENT_SERVICE.to_string(),
        deadline_monotonic_ns,
    };
    let frame = encode_request(&request).map_err(|_| LockStatusError::Encode)?;

    let mut stream = connect_before_deadline(socket, deadline)?;
    write_all_before_deadline(&mut stream, &frame, deadline)?;

    let mut len_bytes = [0u8; 4];
    read_exact_before_deadline(&mut stream, &mut len_bytes, deadline)?;
    let declared = usize::try_from(u32::from_be_bytes(len_bytes)).unwrap_or(usize::MAX);
    if declared == 0 || declared > MAX_MESSAGE_SIZE {
        return Err(LockStatusError::FrameLength(declared));
    }
    let mut full = vec![0u8; declared.saturating_add(len_bytes.len())];
    let (prefix, body) = full.split_at_mut(len_bytes.len());
    prefix.copy_from_slice(&len_bytes);
    read_exact_before_deadline(&mut stream, body, deadline)?;
    drop(stream);

    if let Ok(response) = decode::<LockStateResponse>(&full) {
        return if response.is_current() {
            Ok(response)
        } else {
            Err(LockStatusError::Version(response.version))
        };
    }
    match decode::<Response>(&full) {
        Ok(_) => Err(LockStatusError::Refused),
        Err(_) => Err(LockStatusError::Decode),
    }
}

/// [`query_lock_state`] mapped to a view: every error is [`LockView::UNAVAILABLE`].
#[must_use]
pub fn fetch_view(socket: &Path, uid: u32, budget: Duration) -> LockView {
    query_lock_state(socket, uid, budget).map_or(LockView::UNAVAILABLE, |response| {
        LockView::from_response(&response)
    })
}

/// `CLOCK_MONOTONIC` in milliseconds, `None` when unreadable.
#[must_use]
pub fn monotonic_now_ms() -> Option<u64> {
    monotonic_now_ns().map(|ns| ns / 1_000_000)
}

/// Reads `clock` in nanoseconds, `None` when unreadable or negative.
pub(crate) fn clock_now_ns(clock: ClockId) -> Option<u64> {
    let ts = clock_gettime(clock).ok()?;
    let secs = u64::try_from(ts.tv_sec()).ok()?;
    let nanos = u64::try_from(ts.tv_nsec()).ok()?;
    Some(secs.saturating_mul(1_000_000_000).saturating_add(nanos))
}

fn monotonic_now_ns() -> Option<u64> {
    clock_now_ns(ClockId::CLOCK_MONOTONIC)
}

/// Interval between two connection attempts while the daemon listen backlog is full.
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(5);

/// One cumulative deadline shared by every blocking operation of an exchange.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline {
    start: Instant,
    total: Duration,
}

impl Deadline {
    pub(crate) fn start(total: Duration) -> Self {
        Self {
            start: Instant::now(),
            total,
        }
    }

    /// Budget left, `None` once the deadline has passed (a zero budget never reaches
    /// `set_*_timeout`, which rejects it).
    pub(crate) fn remaining(&self) -> Option<Duration> {
        self.total
            .checked_sub(self.start.elapsed())
            .filter(|left| !left.is_zero())
    }

    fn remaining_or_timeout(&self) -> Result<Duration, LockStatusError> {
        self.remaining().ok_or(LockStatusError::Timeout)
    }
}

/// Connects without ever blocking past `deadline` (non-blocking socket, retried on a full
/// backlog), then switches the socket back to blocking mode.
fn connect_before_deadline(
    socket: &Path,
    deadline: Deadline,
) -> Result<UnixStream, LockStatusError> {
    use nix::errno::Errno;
    use nix::sys::socket::{
        connect, socket as new_socket, AddressFamily, SockFlag, SockType, UnixAddr,
    };
    use std::os::fd::AsRawFd;

    let addr = UnixAddr::new(socket).map_err(|e| LockStatusError::Connect(e.into()))?;
    let fd = new_socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(|e| LockStatusError::Connect(e.into()))?;
    loop {
        match connect(fd.as_raw_fd(), &addr) {
            Ok(()) | Err(Errno::EISCONN) => break,
            Err(Errno::EAGAIN | Errno::EINTR | Errno::EINPROGRESS | Errno::EALREADY) => {
                let left = deadline.remaining_or_timeout()?;
                std::thread::sleep(CONNECT_RETRY_INTERVAL.min(left));
            }
            Err(e) => return Err(LockStatusError::Connect(e.into())),
        }
    }
    deadline.remaining_or_timeout()?;
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false).map_err(LockStatusError::Io)?;
    Ok(stream)
}

fn is_timeout(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn write_all_before_deadline(
    stream: &mut UnixStream,
    bytes: &[u8],
    deadline: Deadline,
) -> Result<(), LockStatusError> {
    let mut offset = 0usize;
    while let Some(rest) = bytes.get(offset..).filter(|rest| !rest.is_empty()) {
        stream
            .set_write_timeout(Some(deadline.remaining_or_timeout()?))
            .map_err(LockStatusError::Io)?;
        match stream.write(rest) {
            Ok(0) => {
                return Err(LockStatusError::Io(std::io::Error::from(
                    std::io::ErrorKind::WriteZero,
                )))
            }
            Ok(n) => offset = offset.saturating_add(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if is_timeout(&e) => return Err(LockStatusError::Timeout),
            Err(e) => return Err(LockStatusError::Io(e)),
        }
    }
    Ok(())
}

fn read_exact_before_deadline(
    stream: &mut UnixStream,
    buf: &mut [u8],
    deadline: Deadline,
) -> Result<(), LockStatusError> {
    let mut offset = 0usize;
    while let Some(rest) = buf.get_mut(offset..).filter(|rest| !rest.is_empty()) {
        stream
            .set_read_timeout(Some(deadline.remaining_or_timeout()?))
            .map_err(LockStatusError::Io)?;
        match stream.read(rest) {
            Ok(0) => {
                return Err(LockStatusError::Io(std::io::Error::from(
                    std::io::ErrorKind::UnexpectedEof,
                )))
            }
            Ok(n) => offset = offset.saturating_add(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if is_timeout(&e) => return Err(LockStatusError::Timeout),
            Err(e) => return Err(LockStatusError::Io(e)),
        }
    }
    Ok(())
}

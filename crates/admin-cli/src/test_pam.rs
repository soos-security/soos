//! Simulated PAM authentication cycle runner and latency benchmark.
//!
//! Connects to `/run/soos/daemon.sock` out-of-band and performs a complete
//! authentication request cycle, benchmarking socket connection latency,
//! daemon processing latency, and returning the rendered verdict.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use soos_protocol::codec::decode;
use soos_protocol::message::encode_request;
use soos_protocol::types::{
    ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION, MAX_MESSAGE_SIZE,
    MAX_RESPONSE_FUTURE_SKEW_NS, REQUEST_ID_LEN,
};

use crate::args::{MAX_TIMEOUT_MS, MIN_TIMEOUT_MS};
use crate::error::AdminCliError;

/// Clamps a requested timeout to the PAM module range `MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS`.
///
/// `pam_soos.so` clamps its `timeout_ms=` argument the same way, so the diagnostic reproduces
/// the budget PAM really applies, and a zero timeout never reaches `set_read_timeout`
/// (which rejects a zero duration with `EINVAL`).
#[must_use]
pub const fn effective_timeout_ms(requested_ms: u64) -> u64 {
    if requested_ms < MIN_TIMEOUT_MS {
        MIN_TIMEOUT_MS
    } else if requested_ms > MAX_TIMEOUT_MS {
        MAX_TIMEOUT_MS
    } else {
        requested_ms
    }
}

/// Sub-millisecond latency breakdown for simulated authentication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencyMetrics {
    /// Time required to establish the Unix domain socket connection (ms).
    pub connect_ms: f64,
    /// Time from request transmission until complete response receipt (ms).
    pub response_ms: f64,
    /// Total simulated authentication latency (ms).
    pub total_ms: f64,
}

/// Why `pam_soos.so` would reject the daemon response whatever its verdict (GitHub #291).
///
/// Serialized as a stable snake_case string (`request_id_mismatch`, `stale_response`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PamTestRejection {
    /// The response does not echo the request nonce (`Response::matches_request`); checked
    /// first, like `IpcError::RequestIdMismatch` in the PAM module.
    RequestIdMismatch,
    /// The response completed after the cumulative deadline started before `connect`
    /// (GitHub #312, STO-NEW-4); `pam_soos.so` never honors such a verdict.
    DeadlineExceeded,
    /// The response stamps are outside their `CLOCK_MONOTONIC` validity window
    /// (`Response::check_freshness`), or the clock could not be read.
    StaleResponse,
}

impl PamTestRejection {
    /// The stable snake_case name, as serialized in the JSON report.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RequestIdMismatch => "request_id_mismatch",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::StaleResponse => "stale_response",
        }
    }
}

/// Simulated PAM authentication cycle report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PamTestReport {
    /// Target UID verified.
    pub uid: u32,
    /// Declared PAM service name.
    pub service: String,
    /// Raw verdict rendered by the daemon, shown even when the response is not accepted (see
    /// [`Self::accepted`]).
    pub verdict: Verdict,
    /// `true` when the response passed the nonce binding and the freshness check, so that
    /// `verdict` is what `pam_soos.so` acts on; `false` means the PAM fallback whatever the
    /// verdict (GitHub #291).
    pub accepted: bool,
    /// The first check the response failed, `None` when it is accepted.
    pub rejected_reason: Option<PamTestRejection>,
    /// Diagnostic classification reason.
    pub reason_class: ReasonClass,
    /// Latency breakdown.
    pub latency: LatencyMetrics,
    /// Resulting PAM stack interpretation.
    pub pam_result: String,
}

impl PamTestReport {
    /// Process exit code of `soos-admin test-pam` (GitHub #312, STO-NEW-3): `0` only when
    /// the response was accepted (nonce binding, deadline, freshness) AND its verdict is
    /// `Allow`, i.e. exactly when `pam_soos.so` would return `PAM_SUCCESS`; `1` otherwise.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        if self.accepted && self.verdict == Verdict::Allow {
            0
        } else {
            1
        }
    }

    /// Formats the report as a pretty-printed JSON document.
    ///
    /// Produced by `serde_json`, so every string field is escaped (GitHub #232).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self)
            .unwrap_or_else(|_| "{\"error\": \"report serialization failed\"}".to_string())
    }

    /// Formats the report as an aligned terminal summary table.
    #[must_use]
    pub fn format_table(&self) -> String {
        let mut out = String::new();
        out.push_str("====================================================\n");
        out.push_str("          SOOS SIMULATED PAM AUTHENTICATION         \n");
        out.push_str("====================================================\n");
        out.push_str(&format!("Target UID:          {}\n", self.uid));
        out.push_str(&format!("Service:             {}\n", self.service));
        out.push_str(&format!("Daemon Verdict:      {:?}\n", self.verdict));
        match self.rejected_reason {
            Some(reason) => {
                out.push_str(&format!("Response Accepted:   no ({})\n", reason.as_str()))
            }
            None if self.accepted => out.push_str("Response Accepted:   yes\n"),
            None => out.push_str("Response Accepted:   no\n"),
        }
        out.push_str(&format!("Reason Class:        {:?}\n", self.reason_class));
        out.push_str(&format!("PAM Action:          {}\n", self.pam_result));
        out.push_str("----------------------------------------------------\n");
        out.push_str("Latency Metrics:\n");
        out.push_str(&format!(
            "  Socket Connect:    {:.2} ms\n",
            self.latency.connect_ms
        ));
        out.push_str(&format!(
            "  Daemon Response:   {:.2} ms\n",
            self.latency.response_ms
        ));
        out.push_str(&format!(
            "  Total Roundtrip:   {:.2} ms\n",
            self.latency.total_ms
        ));
        out.push_str("====================================================\n");
        out
    }
}

/// Simulates a PAM authentication cycle by issuing an IPC verification request.
///
/// `timeout_ms` is clamped with [`effective_timeout_ms`]; the request deadline is expressed
/// on `CLOCK_MONOTONIC`, the clock `soos-daemon` compares it against (GitHub #231).
///
/// Like `pam_soos.so` (GitHub #312, STO-NEW-4), one cumulative deadline is started before
/// `connect` and bounds the whole exchange: the connection is attempted without blocking,
/// `SO_SNDTIMEO` / `SO_RCVTIMEO` are re-armed with the budget left before every `write()` /
/// `read()`, and a response completed after the deadline is reported as
/// [`PamTestRejection::DeadlineExceeded`] whatever its verdict.
///
/// # Errors
///
/// Returns `AdminCliError` on socket, codec, or random number generator failures, and
/// [`AdminCliError::Timeout`] when the deadline expires before the response is complete.
pub fn simulate_pam_auth(
    socket_path: &Path,
    uid: u32,
    service: &str,
    timeout_ms: u64,
) -> Result<PamTestReport, AdminCliError> {
    let timeout_ms = effective_timeout_ms(timeout_ms);
    let deadline = ExchangeDeadline::start(Duration::from_millis(timeout_ms));
    // The CLOCK_MONOTONIC deadline sent to the daemon is fixed together with the local one.
    let deadline_monotonic_ns =
        monotonic_now_ns()?.saturating_add(timeout_ms.saturating_mul(1_000_000));

    let connect_start = Instant::now();
    let mut stream = connect_before_deadline(socket_path, deadline)?;
    let connect_ms = connect_start.elapsed().as_secs_f64() * 1000.0;

    let mut request_id = [0u8; REQUEST_ID_LEN];
    getrandom::fill(&mut request_id)?;

    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id,
        uid_hint: uid,
        service: service.to_string(),
        deadline_monotonic_ns,
    };

    let req_start = Instant::now();
    let encoded_req = encode_request(&req)?;
    write_all_before_deadline(&mut stream, &encoded_req, deadline)?;

    let mut len_bytes = [0u8; 4];
    read_exact_before_deadline(&mut stream, &mut len_bytes, deadline)?;
    let declared_size = usize::try_from(u32::from_be_bytes(len_bytes))
        .map_err(|_| AdminCliError::UnexpectedResponse("overflow in length prefix".to_string()))?;

    if declared_size > MAX_MESSAGE_SIZE || declared_size == 0 {
        return Err(AdminCliError::UnexpectedResponse(format!(
            "invalid declared response size {declared_size} (max {MAX_MESSAGE_SIZE})"
        )));
    }

    let mut body = vec![0u8; declared_size];
    read_exact_before_deadline(&mut stream, &mut body, deadline)?;
    // Checked once the last byte is in, like `pam_soos.so` (never honored when late).
    let within_deadline = deadline.remaining().is_some();
    let response_ms = req_start.elapsed().as_secs_f64() * 1000.0;
    let total_ms = connect_ms + response_ms;
    drop(stream);

    let total_capacity = declared_size.saturating_add(4);
    let mut full = Vec::with_capacity(total_capacity);
    full.extend_from_slice(&len_bytes);
    full.extend_from_slice(&body);

    let resp: Response = decode(&full)?;

    // Same replay protection as pam_soos.so (GitHub #289): the response must echo the fresh
    // nonce bit for bit (`Response::matches_request`, checked before the stamps exactly like
    // `IpcError::RequestIdMismatch`). A mismatch is the PAM fallback whatever the verdict; the
    // nonce itself is never printed.
    let bound_to_request = resp.matches_request(&request_id);

    // Same staleness guard as pam_soos.so (GitHub #287): the stamps are checked against
    // CLOCK_MONOTONIC read after the response arrived, with the shared skew bound. A stale
    // response is interpreted as the PAM fallback whatever its verdict; a clock read failure
    // passes 0 and is reported as stale too (fail closed).
    let freshness =
        resp.check_freshness(monotonic_now_ns().unwrap_or(0), MAX_RESPONSE_FUTURE_SKEW_NS);

    let rejected_reason = first_rejection(bound_to_request, within_deadline, freshness.is_ok());

    let pam_result = if !bound_to_request {
        format!(
            "PAM_IGNORE (response request_id does not match the request nonce; the PAM module \
             falls back to the password whatever the {:?} verdict)",
            resp.verdict
        )
    } else if !within_deadline {
        format!(
            "PAM_IGNORE (response completed after the {timeout_ms} ms deadline; the PAM module \
             falls back to the password whatever the {:?} verdict)",
            resp.verdict
        )
    } else if let Err(stale) = freshness {
        format!(
            "PAM_IGNORE (stale daemon response: {stale}; the PAM module falls back to the \
             password whatever the {:?} verdict)",
            resp.verdict
        )
    } else if resp.verdict == Verdict::Allow {
        "PAM_SUCCESS (Authentication authorized)".to_string()
    } else {
        format!(
            "PAM_IGNORE (Fallback to standard password authentication: {:?})",
            resp.verdict
        )
    };

    Ok(PamTestReport {
        uid,
        service: service.to_string(),
        verdict: resp.verdict,
        accepted: rejected_reason.is_none(),
        rejected_reason,
        reason_class: resp.reason_class,
        latency: LatencyMetrics {
            connect_ms,
            response_ms,
            total_ms,
        },
        pam_result,
    })
}

/// The first check a response fails, in the order of `pam_soos.so`: nonce binding, then the
/// cumulative deadline, then the freshness of the stamps.
const fn first_rejection(
    bound_to_request: bool,
    within_deadline: bool,
    fresh: bool,
) -> Option<PamTestRejection> {
    if !bound_to_request {
        Some(PamTestRejection::RequestIdMismatch)
    } else if !within_deadline {
        Some(PamTestRejection::DeadlineExceeded)
    } else if !fresh {
        Some(PamTestRejection::StaleResponse)
    } else {
        None
    }
}

/// Interval between two connection attempts while the daemon listen backlog is full.
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(5);

/// One cumulative deadline shared by every blocking operation of the exchange.
#[derive(Debug, Clone, Copy)]
struct ExchangeDeadline {
    start: Instant,
    total: Duration,
}

impl ExchangeDeadline {
    fn start(total: Duration) -> Self {
        Self {
            start: Instant::now(),
            total,
        }
    }

    /// Budget left, `None` once the deadline has passed (a zero budget never reaches
    /// `set_*_timeout`, which rejects it with `EINVAL`).
    fn remaining(&self) -> Option<Duration> {
        self.total
            .checked_sub(self.start.elapsed())
            .filter(|left| !left.is_zero())
    }

    fn remaining_or_timeout(&self) -> Result<Duration, AdminCliError> {
        self.remaining().ok_or(AdminCliError::Timeout)
    }
}

/// Connects to the daemon socket without ever blocking past `deadline`.
///
/// The socket is created non-blocking: a full listen backlog (`EAGAIN`) is retried until the
/// deadline instead of blocking in `connect()`. The connected socket is switched back to
/// blocking mode; every later operation is bounded by its re-armed timeout.
fn connect_before_deadline(
    socket_path: &Path,
    deadline: ExchangeDeadline,
) -> Result<UnixStream, AdminCliError> {
    use nix::errno::Errno;
    use nix::sys::socket::{connect, socket, AddressFamily, SockFlag, SockType, UnixAddr};
    use std::os::fd::AsRawFd;

    let connect_error = |source: std::io::Error| AdminCliError::SocketConnect {
        path: socket_path.display().to_string(),
        source,
    };
    let addr = UnixAddr::new(socket_path).map_err(|e| connect_error(e.into()))?;
    let fd = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(|e| connect_error(e.into()))?;
    loop {
        match connect(fd.as_raw_fd(), &addr) {
            Ok(()) | Err(Errno::EISCONN) => break,
            Err(Errno::EAGAIN | Errno::EINTR | Errno::EINPROGRESS | Errno::EALREADY) => {
                let left = deadline.remaining_or_timeout()?;
                std::thread::sleep(CONNECT_RETRY_INTERVAL.min(left));
            }
            Err(e) => return Err(connect_error(e.into())),
        }
    }
    deadline.remaining_or_timeout()?;
    let stream = UnixStream::from(fd);
    stream
        .set_nonblocking(false)
        .map_err(AdminCliError::SocketIo)?;
    Ok(stream)
}

/// Whether an I/O error is the expiry of a socket timeout.
fn is_timeout(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Writes all of `bytes`, re-arming `SO_SNDTIMEO` with the budget left before every write.
fn write_all_before_deadline(
    stream: &mut UnixStream,
    bytes: &[u8],
    deadline: ExchangeDeadline,
) -> Result<(), AdminCliError> {
    let mut offset = 0usize;
    while let Some(rest) = bytes.get(offset..).filter(|rest| !rest.is_empty()) {
        stream
            .set_write_timeout(Some(deadline.remaining_or_timeout()?))
            .map_err(AdminCliError::SocketIo)?;
        match stream.write(rest) {
            Ok(0) => {
                return Err(AdminCliError::SocketIo(std::io::Error::from(
                    std::io::ErrorKind::WriteZero,
                )))
            }
            Ok(n) => offset = offset.saturating_add(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if is_timeout(&e) => return Err(AdminCliError::Timeout),
            Err(e) => return Err(AdminCliError::SocketIo(e)),
        }
    }
    Ok(())
}

/// Fills `buf`, re-arming `SO_RCVTIMEO` with the budget left before every read.
fn read_exact_before_deadline(
    stream: &mut UnixStream,
    buf: &mut [u8],
    deadline: ExchangeDeadline,
) -> Result<(), AdminCliError> {
    let mut offset = 0usize;
    while let Some(rest) = buf.get_mut(offset..).filter(|rest| !rest.is_empty()) {
        stream
            .set_read_timeout(Some(deadline.remaining_or_timeout()?))
            .map_err(AdminCliError::SocketIo)?;
        match stream.read(rest) {
            Ok(0) => {
                return Err(AdminCliError::SocketIo(std::io::Error::from(
                    std::io::ErrorKind::UnexpectedEof,
                )))
            }
            Ok(n) => offset = offset.saturating_add(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if is_timeout(&e) => return Err(AdminCliError::Timeout),
            Err(e) => return Err(AdminCliError::SocketIo(e)),
        }
    }
    Ok(())
}

/// Reads `CLOCK_MONOTONIC` in nanoseconds (the clock of `deadline_monotonic_ns`).
///
/// # Errors
///
/// [`AdminCliError::SocketIo`] when the clock cannot be read or reports a negative value.
fn monotonic_now_ns() -> Result<u64, AdminCliError> {
    let ts = nix::time::clock_gettime(nix::time::ClockId::CLOCK_MONOTONIC)
        .map_err(|e| AdminCliError::SocketIo(e.into()))?;
    match (u64::try_from(ts.tv_sec()), u64::try_from(ts.tv_nsec())) {
        (Ok(secs), Ok(nanos)) => Ok(secs.saturating_mul(1_000_000_000).saturating_add(nanos)),
        _ => Err(AdminCliError::SocketIo(std::io::Error::other(
            "CLOCK_MONOTONIC returned a negative timestamp",
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{first_rejection, PamTestRejection};

    #[test]
    fn test_312_rejection_order_matches_pam_module() {
        assert_eq!(first_rejection(true, true, true), None);
        assert_eq!(
            first_rejection(false, false, false),
            Some(PamTestRejection::RequestIdMismatch)
        );
        assert_eq!(
            first_rejection(true, false, true),
            Some(PamTestRejection::DeadlineExceeded),
            "a late Allow is never accepted"
        );
        assert_eq!(
            first_rejection(true, false, false),
            Some(PamTestRejection::DeadlineExceeded)
        );
        assert_eq!(
            first_rejection(true, true, false),
            Some(PamTestRejection::StaleResponse)
        );
    }
}

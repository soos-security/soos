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

/// Simulated PAM authentication cycle report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PamTestReport {
    /// Target UID verified.
    pub uid: u32,
    /// Declared PAM service name.
    pub service: String,
    /// Verdict rendered by daemon.
    pub verdict: Verdict,
    /// Diagnostic classification reason.
    pub reason_class: ReasonClass,
    /// Latency breakdown.
    pub latency: LatencyMetrics,
    /// Resulting PAM stack interpretation.
    pub pam_result: String,
}

impl PamTestReport {
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
/// # Errors
///
/// Returns `AdminCliError` on socket, codec, timeout, or random number generator failures.
pub fn simulate_pam_auth(
    socket_path: &Path,
    uid: u32,
    service: &str,
    timeout_ms: u64,
) -> Result<PamTestReport, AdminCliError> {
    let timeout_ms = effective_timeout_ms(timeout_ms);
    let connect_start = Instant::now();
    let mut stream =
        UnixStream::connect(socket_path).map_err(|e| AdminCliError::SocketConnect {
            path: socket_path.display().to_string(),
            source: e,
        })?;
    let connect_ms = connect_start.elapsed().as_secs_f64() * 1000.0;

    let timeout = Duration::from_millis(timeout_ms);
    stream
        .set_read_timeout(Some(timeout))
        .map_err(AdminCliError::SocketIo)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(AdminCliError::SocketIo)?;

    let mut request_id = [0u8; REQUEST_ID_LEN];
    getrandom::fill(&mut request_id)?;

    let now_monotonic_ns = monotonic_now_ns()?;
    let deadline_monotonic_ns =
        now_monotonic_ns.saturating_add(timeout_ms.saturating_mul(1_000_000));

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
    stream
        .write_all(&encoded_req)
        .map_err(AdminCliError::SocketIo)?;
    stream.flush().map_err(AdminCliError::SocketIo)?;

    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .map_err(AdminCliError::SocketIo)?;
    let declared_size = usize::try_from(u32::from_be_bytes(len_bytes))
        .map_err(|_| AdminCliError::UnexpectedResponse("overflow in length prefix".to_string()))?;

    if declared_size > MAX_MESSAGE_SIZE || declared_size == 0 {
        return Err(AdminCliError::UnexpectedResponse(format!(
            "invalid declared response size {declared_size} (max {MAX_MESSAGE_SIZE})"
        )));
    }

    let mut body = vec![0u8; declared_size];
    stream
        .read_exact(&mut body)
        .map_err(AdminCliError::SocketIo)?;
    let response_ms = req_start.elapsed().as_secs_f64() * 1000.0;
    let total_ms = connect_ms + response_ms;

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

    let pam_result = if !bound_to_request {
        format!(
            "PAM_IGNORE (response request_id does not match the request nonce; the PAM module \
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
        reason_class: resp.reason_class,
        latency: LatencyMetrics {
            connect_ms,
            response_ms,
            total_ms,
        },
        pam_result,
    })
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

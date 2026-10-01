//! `soos-admin test-pam` applies the PAM response staleness guard (GitHub #287, row PRE11).
//!
//! The diagnostic must interpret a daemon response exactly as `pam_soos.so` does: an
//! `Allow` whose `CLOCK_MONOTONIC` window is closed or unset is reported as a `PAM_IGNORE`
//! fallback naming the stale response, while the daemon verdict itself is still shown.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

#[path = "../../pam/tests/common/stamps.rs"]
mod stamps;

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;

use soos_admin_cli::test_pam::{simulate_pam_auth, PamTestReport};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, Response, Verdict, CURRENT_VERSION};
use stamps::{monotonic_now_ns, FIXTURE_VALIDITY_NS};
use tempfile::tempdir;

/// Runs `test-pam` against a one-shot daemon answering `Allow` with the stamps returned by
/// `stamps_at(now)`, `now` being CLOCK_MONOTONIC when the response is built.
fn report_with(stamps_at: fn(u64) -> (u64, u64)) -> PamTestReport {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("test_pam_freshness.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let mut body = vec![0u8; u32::from_be_bytes(len_buf) as usize];
        stream.read_exact(&mut body).expect("read body");
        let mut full = len_buf.to_vec();
        full.extend_from_slice(&body);
        let req: Request = decode(&full).expect("decode request");
        let (issued, expires) = stamps_at(monotonic_now_ns());
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Allow,
            reason_class: ReasonClass::FaceMatch,
            issued_monotonic_ns: issued,
            expires_monotonic_ns: expires,
        };
        stream
            .write_all(&encode(&resp).expect("encode"))
            .expect("write response");
    });
    let report =
        simulate_pam_auth(&socket_path, 1000, "soos-admin", 5000).expect("test-pam completes");
    server.join().expect("join server");
    report
}

/// PRE11: an expired `Allow` is reported as the PAM fallback, naming the stale response.
#[test]
fn test_pre_test_pam_reports_expired_allow_as_pam_ignore() {
    let report = report_with(|now| (now - 3_000_000_000, now - 1_000_000_000));
    assert_eq!(
        report.verdict,
        Verdict::Allow,
        "the daemon verdict is still shown"
    );
    assert!(
        report.pam_result.starts_with("PAM_IGNORE"),
        "an expired Allow must be interpreted as PAM_IGNORE: {}",
        report.pam_result
    );
    assert!(
        report
            .pam_result
            .contains("stale daemon response: response expired"),
        "the report must name the stale response: {}",
        report.pam_result
    );
}

/// PRE11: an unstamped `Allow` (`0 / 0`) is reported as the PAM fallback.
#[test]
fn test_pre_test_pam_reports_unstamped_allow_as_pam_ignore() {
    let report = report_with(|_| (0, 0));
    assert!(
        report.pam_result.starts_with("PAM_IGNORE"),
        "{}",
        report.pam_result
    );
    assert!(
        report
            .pam_result
            .contains("stale daemon response: response timestamps are unset"),
        "{}",
        report.pam_result
    );
}

/// PRE11: an `Allow` dated beyond the shared skew bound is reported as the PAM fallback.
#[test]
fn test_pre_test_pam_reports_future_dated_allow_as_pam_ignore() {
    let report = report_with(|now| {
        let issued = now + soos_protocol::MAX_RESPONSE_FUTURE_SKEW_NS + 1_000_000_000;
        (issued, issued + FIXTURE_VALIDITY_NS)
    });
    assert!(
        report.pam_result.starts_with("PAM_IGNORE"),
        "{}",
        report.pam_result
    );
    assert!(
        report.pam_result.contains("issued in the future"),
        "{}",
        report.pam_result
    );
}

/// PRE11 (control): a daemon-like stamped `Allow` is still `PAM_SUCCESS`.
#[test]
fn test_pre_test_pam_reports_fresh_allow_as_pam_success() {
    let report = report_with(|now| (now, now + FIXTURE_VALIDITY_NS));
    assert!(
        report.pam_result.starts_with("PAM_SUCCESS"),
        "{}",
        report.pam_result
    );
}

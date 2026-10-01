//! `soos-admin test-pam` says explicitly whether the daemon response was accepted (GitHub #291,
//! matrix row CDF2).
//!
//! `verdict` keeps showing the raw daemon verdict, also for a response `pam_soos.so` would
//! reject (bound to another nonce, or stale). A JSON consumer must not have to parse
//! `pam_result` to know that: `accepted` is `true` only when the response passed the nonce
//! binding and the freshness check (the verdict is then what the PAM module acts on), and
//! `rejected_reason` names the first failed check with a stable snake_case value
//! (`request_id_mismatch`, checked first like the PAM module, then `stale_response`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing,
    reason = "Contractual test suites use assertions, unwrap, expect and JSON indexing"
)]

#[path = "../../pam/tests/common/stamps.rs"]
mod stamps;

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;

use soos_admin_cli::test_pam::{simulate_pam_auth, PamTestRejection, PamTestReport};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, Response, Verdict, CURRENT_VERSION};
use stamps::{fresh_stamps, monotonic_now_ns};
use tempfile::tempdir;

/// One-shot daemon answering `verdict` with `request_id = id_for(nonce)` and the stamps
/// returned by `stamps_at(now)`.
fn report_with(
    verdict: Verdict,
    id_for: fn([u8; 32]) -> [u8; 32],
    stamps_at: fn(u64) -> (u64, u64),
) -> PamTestReport {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("test_pam_acceptance.sock");
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
            request_id: id_for(req.request_id),
            verdict,
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

fn same(id: [u8; 32]) -> [u8; 32] {
    id
}

fn flip_last_bit(mut id: [u8; 32]) -> [u8; 32] {
    id[31] ^= 0x01;
    id
}

fn fresh(_now: u64) -> (u64, u64) {
    fresh_stamps()
}

fn expired(now: u64) -> (u64, u64) {
    (now - 3_000_000_000, now - 1_000_000_000)
}

fn json_of(report: &PamTestReport) -> serde_json::Value {
    serde_json::from_str(&report.to_json()).expect("to_json is valid JSON")
}

/// CDF2: a bound, fresh `Allow` is accepted, without a rejection reason.
#[test]
fn test_cdf_bound_fresh_response_is_accepted() {
    let report = report_with(Verdict::Allow, same, fresh);
    assert!(report.accepted);
    assert_eq!(report.rejected_reason, None);
    let json = json_of(&report);
    assert_eq!(json["accepted"], serde_json::Value::Bool(true));
    assert!(json["rejected_reason"].is_null());
    // The existing fields are kept.
    assert_eq!(json["verdict"], serde_json::json!("Allow"));
    assert!(json["pam_result"]
        .as_str()
        .unwrap()
        .starts_with("PAM_SUCCESS"));
    assert!(
        report.format_table().contains("Response Accepted:   yes"),
        "{}",
        report.format_table()
    );

    // A bound, fresh `Deny` is accepted too: the PAM module acts on that verdict.
    let report = report_with(Verdict::Deny, same, fresh);
    assert!(report.accepted);
    assert_eq!(report.rejected_reason, None);
}

/// CDF2: an `Allow` bound to another nonce is not accepted (`request_id_mismatch`), while the
/// raw verdict is still shown.
#[test]
fn test_cdf_mismatched_response_is_rejected_with_a_stable_reason() {
    let report = report_with(Verdict::Allow, flip_last_bit, fresh);
    assert_eq!(
        report.verdict,
        Verdict::Allow,
        "the raw verdict is still shown"
    );
    assert!(!report.accepted);
    assert_eq!(
        report.rejected_reason,
        Some(PamTestRejection::RequestIdMismatch)
    );
    let json = json_of(&report);
    assert_eq!(json["accepted"], serde_json::Value::Bool(false));
    assert_eq!(
        json["rejected_reason"],
        serde_json::json!("request_id_mismatch")
    );
    assert!(
        report
            .format_table()
            .contains("Response Accepted:   no (request_id_mismatch)"),
        "{}",
        report.format_table()
    );
}

/// CDF2: an expired `Allow` is not accepted (`stale_response`); a mismatched and expired one
/// reports the mismatch (checked first, like `pam_soos.so`).
#[test]
fn test_cdf_stale_response_is_rejected_and_mismatch_wins() {
    let report = report_with(Verdict::Allow, same, expired);
    assert!(!report.accepted);
    assert_eq!(
        report.rejected_reason,
        Some(PamTestRejection::StaleResponse)
    );
    assert_eq!(
        json_of(&report)["rejected_reason"],
        serde_json::json!("stale_response")
    );
    assert!(report
        .format_table()
        .contains("Response Accepted:   no (stale_response)"));

    let report = report_with(Verdict::Allow, flip_last_bit, expired);
    assert_eq!(
        report.rejected_reason,
        Some(PamTestRejection::RequestIdMismatch)
    );
}

/// CDF2: the reason vocabulary is stable and round-trips through JSON.
#[test]
fn test_cdf_rejection_reasons_are_stable_snake_case() {
    assert_eq!(
        PamTestRejection::RequestIdMismatch.as_str(),
        "request_id_mismatch"
    );
    assert_eq!(PamTestRejection::StaleResponse.as_str(), "stale_response");
    let report = report_with(Verdict::Allow, same, expired);
    let parsed: PamTestReport = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(parsed, report);
}

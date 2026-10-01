//! `soos-admin test-pam` binds the daemon response to its request nonce (GitHub #289, row DGP1).
//!
//! `pam_soos.so` refuses a response whose `request_id` differs from the fresh 256-bit nonce it
//! sent (`Response::matches_request`, `IpcError::RequestIdMismatch` → `PAM_IGNORE`). The
//! diagnostic must interpret such a response the same way: the daemon verdict is still shown,
//! but the PAM action is the password fallback, with a reason that never prints the nonce.

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
use stamps::fresh_stamps;
use tempfile::tempdir;

/// Runs `test-pam` against a one-shot daemon answering a freshly stamped `verdict` whose
/// `request_id` is `id_for(sent_nonce)`. Returns the report and the nonce that was sent.
fn report_with(verdict: Verdict, id_for: fn([u8; 32]) -> [u8; 32]) -> (PamTestReport, [u8; 32]) {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("test_pam_request_id.sock");
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
        let (issued, expires) = fresh_stamps();
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
        req.request_id
    });
    let report =
        simulate_pam_auth(&socket_path, 1000, "soos-admin", 5000).expect("test-pam completes");
    let sent = server.join().expect("join server");
    (report, sent)
}

fn flip_last_bit(mut id: [u8; 32]) -> [u8; 32] {
    id[31] ^= 0x01;
    id
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// DGP1: a fresh `Allow` bound to another nonce (one bit differs) is the PAM fallback.
#[test]
fn test_dgp_test_pam_reports_request_id_mismatch_as_pam_ignore() {
    let (report, sent) = report_with(Verdict::Allow, flip_last_bit);
    assert_eq!(
        report.verdict,
        Verdict::Allow,
        "the daemon verdict is still shown"
    );
    assert!(
        report.pam_result.starts_with("PAM_IGNORE"),
        "a response bound to another nonce must be interpreted as PAM_IGNORE: {}",
        report.pam_result
    );
    assert!(
        report
            .pam_result
            .contains("request_id does not match the request nonce"),
        "the report must name the mismatch: {}",
        report.pam_result
    );
    let json = report.to_json();
    for text in [report.pam_result.as_str(), json.as_str()] {
        assert!(
            !text.contains(&hex(&sent)) && !text.contains(&hex(&flip_last_bit(sent))),
            "neither nonce may be printed: {text}"
        );
    }
}

/// DGP1: an all-zero `request_id` (a replayed or forged frame) is the PAM fallback too, and a
/// mismatched `Deny` keeps the fallback wording of the mismatch rather than of the verdict.
#[test]
fn test_dgp_test_pam_reports_zero_request_id_as_pam_ignore() {
    let (report, _) = report_with(Verdict::Allow, |_| [0u8; 32]);
    assert!(
        report.pam_result.starts_with("PAM_IGNORE")
            && report
                .pam_result
                .contains("request_id does not match the request nonce"),
        "{}",
        report.pam_result
    );
    let (report, _) = report_with(Verdict::Deny, flip_last_bit);
    assert!(
        report
            .pam_result
            .contains("request_id does not match the request nonce"),
        "{}",
        report.pam_result
    );
}

/// DGP1 (control): the echoed nonce with fresh stamps is still `PAM_SUCCESS`.
#[test]
fn test_dgp_test_pam_matching_request_id_is_pam_success() {
    let (report, _) = report_with(Verdict::Allow, |id| id);
    assert!(
        report.pam_result.starts_with("PAM_SUCCESS"),
        "{}",
        report.pam_result
    );
}

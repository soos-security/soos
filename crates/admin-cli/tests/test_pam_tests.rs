//! Contractual tests for simulated PAM authentication cycle (Sub-issue #11.3).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

#[path = "../../pam/tests/common/stamps.rs"]
mod stamps;

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

use soos_admin_cli::test_pam::simulate_pam_auth;
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};

#[test]
fn test_simulate_pam_auth_allow() {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("test_pam_allow.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).expect("read body");

        let mut full = len_buf.to_vec();
        full.extend_from_slice(&body);

        let req: Request = decode(&full).expect("decode request");
        assert_eq!(req.kind, RequestKind::Auth);
        assert_eq!(req.uid_hint, 1000);
        assert_eq!(req.service, "soos-admin");

        // Artificial delay for latency measurement
        thread::sleep(Duration::from_millis(5));

        let (issued, expires) = stamps::fresh_stamps();
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Allow,
            reason_class: ReasonClass::FaceMatch,
            issued_monotonic_ns: issued,
            expires_monotonic_ns: expires,
        };

        let encoded = encode(&resp).expect("encode response");
        stream.write_all(&encoded).expect("write response");
    });

    // Maximum PAM timeout: the assertions check the verdict, not the timing (user-approved
    // 2026-10-01, GitHub #285).
    let report = simulate_pam_auth(&socket_path, 1000, "soos-admin", 5000)
        .expect("simulated PAM auth must succeed");
    server_handle.join().expect("join server");

    assert_eq!(report.uid, 1000);
    assert_eq!(report.service, "soos-admin");
    assert_eq!(report.verdict, Verdict::Allow);
    assert_eq!(report.reason_class, ReasonClass::FaceMatch);
    assert!(report.pam_result.contains("PAM_SUCCESS"));
    assert!(report.latency.connect_ms >= 0.0);
    assert!(report.latency.response_ms >= 1.0);
    assert!(report.latency.total_ms >= report.latency.response_ms);
}

#[test]
fn test_simulate_pam_auth_deny_yields_pam_ignore() {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("test_pam_deny.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).expect("read body");

        let mut full = len_buf.to_vec();
        full.extend_from_slice(&body);

        let req: Request = decode(&full).expect("decode request");

        let (issued, expires) = stamps::fresh_stamps();
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Deny,
            reason_class: ReasonClass::ScoreBelowThreshold,
            issued_monotonic_ns: issued,
            expires_monotonic_ns: expires,
        };

        let encoded = encode(&resp).expect("encode response");
        stream.write_all(&encoded).expect("write response");
    });

    // Maximum PAM timeout: the assertions check the verdict, not the timing (user-approved
    // 2026-10-01, GitHub #285).
    let report = simulate_pam_auth(&socket_path, 1000, "soos-admin", 5000)
        .expect("simulated PAM auth must complete");
    server_handle.join().expect("join server");

    assert_eq!(report.verdict, Verdict::Deny);
    assert_eq!(report.reason_class, ReasonClass::ScoreBelowThreshold);
    assert!(
        report.pam_result.contains("PAM_IGNORE"),
        "Deny verdict must yield PAM_IGNORE fallback: {}",
        report.pam_result
    );
}

#[test]
fn test_simulate_pam_auth_offline_socket_fails_closed() {
    let dir = tempdir().expect("tempdir");
    let non_existent = dir.path().join("offline.sock");

    let err = simulate_pam_auth(&non_existent, 1000, "soos-admin", 250)
        .expect_err("connecting to offline socket must fail");

    let err_str = err.to_string();
    assert!(
        err_str.contains("failed to connect") || err_str.contains("offline"),
        "Error message should explain connection failure: {err_str}"
    );
}

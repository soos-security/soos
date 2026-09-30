//! Contractual tests for `soos-admin test-pam` deadline semantics (GitHub #231, STO-15) and
//! escaped JSON output of `status` / `test-pam` (GitHub #232, STO-16).
//!
//! Contract:
//! - `deadline_monotonic_ns` is taken from `CLOCK_MONOTONIC` (the daemon's clock), so it lies
//!   within `[now_mono + timeout, later_mono + timeout]`.
//! - `timeout_ms` is clamped to the PAM module range `MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS`
//!   (10..=5000 ms), so `--timeout-ms 0` never reaches `set_read_timeout(0)` (`EINVAL`).
//! - `to_json` output is produced by `serde_json` and round-trips any string content.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::thread;
use tempfile::tempdir;

use nix::time::{clock_gettime, ClockId};
use soos_admin_cli::args::{MAX_TIMEOUT_MS, MIN_TIMEOUT_MS};
use soos_admin_cli::status::DaemonStatusReport;
use soos_admin_cli::test_pam::{
    effective_timeout_ms, simulate_pam_auth, LatencyMetrics, PamTestReport,
};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, Response, Verdict, CURRENT_VERSION};

fn monotonic_ns() -> u64 {
    let ts = clock_gettime(ClockId::CLOCK_MONOTONIC).expect("CLOCK_MONOTONIC");
    u64::try_from(ts.tv_sec()).unwrap() * 1_000_000_000 + u64::try_from(ts.tv_nsec()).unwrap()
}

/// Runs one simulated authentication with `timeout_ms` and returns
/// `(before_ns, deadline_ns, after_ns)`, all on `CLOCK_MONOTONIC`.
fn capture_deadline(timeout_ms: u64) -> (u64, u64, u64) {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("deadline.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind socket");
    let (tx, rx) = mpsc::channel();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).expect("read body");
        let mut full = len_buf.to_vec();
        full.extend_from_slice(&body);
        let req: Request = decode(&full).expect("decode request");
        tx.send(req.deadline_monotonic_ns).expect("send deadline");
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Deny,
            reason_class: ReasonClass::NoFace,
            issued_monotonic_ns: 0,
            expires_monotonic_ns: 0,
        };
        stream
            .write_all(&encode(&resp).expect("encode"))
            .expect("write response");
    });

    let before = monotonic_ns();
    simulate_pam_auth(&socket_path, 1000, "soos-admin", timeout_ms)
        .expect("simulated authentication must complete");
    let after = monotonic_ns();
    server.join().expect("join server");
    let deadline = rx.recv().expect("deadline received");
    (before, deadline, after)
}

#[test]
fn test_simulate_pam_auth_deadline_uses_monotonic_clock() {
    let (before, deadline, after) = capture_deadline(250);
    let budget = 250 * 1_000_000;
    assert!(
        deadline >= before + budget && deadline <= after + budget,
        "deadline {deadline} must lie in [{}, {}] on CLOCK_MONOTONIC",
        before + budget,
        after + budget
    );
}

#[test]
fn test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum() {
    let (before, deadline, after) = capture_deadline(0);
    let budget = MIN_TIMEOUT_MS * 1_000_000;
    assert!(
        deadline >= before + budget && deadline <= after + budget,
        "a zero timeout must be clamped to {MIN_TIMEOUT_MS} ms"
    );
}

#[test]
fn test_simulate_pam_auth_huge_timeout_is_clamped_to_pam_maximum() {
    let (before, deadline, after) = capture_deadline(u64::MAX);
    let budget = MAX_TIMEOUT_MS * 1_000_000;
    assert!(
        deadline >= before + budget && deadline <= after + budget,
        "an oversized timeout must be clamped to {MAX_TIMEOUT_MS} ms"
    );
}

#[test]
fn test_effective_timeout_matches_pam_clamp() {
    assert_eq!(MIN_TIMEOUT_MS, 10);
    assert_eq!(MAX_TIMEOUT_MS, 5000);
    assert_eq!(effective_timeout_ms(0), MIN_TIMEOUT_MS);
    assert_eq!(effective_timeout_ms(9), MIN_TIMEOUT_MS);
    assert_eq!(effective_timeout_ms(250), 250);
    assert_eq!(effective_timeout_ms(5001), MAX_TIMEOUT_MS);
    assert_eq!(effective_timeout_ms(u64::MAX), MAX_TIMEOUT_MS);
}

#[test]
fn test_pam_test_report_json_escapes_special_characters() {
    let report = PamTestReport {
        uid: 1000,
        service: "svc\"with\\quote\nand newline".to_string(),
        verdict: Verdict::Deny,
        reason_class: ReasonClass::ScoreBelowThreshold,
        latency: LatencyMetrics {
            connect_ms: 0.25,
            response_ms: 12.5,
            total_ms: 12.75,
        },
        pam_result: "PAM_IGNORE (\"quoted\")".to_string(),
    };
    let json = report.to_json();
    let parsed: PamTestReport = serde_json::from_str(&json).expect("to_json must be valid JSON");
    assert_eq!(parsed, report);
}

#[test]
fn test_status_report_json_escapes_socket_path_and_unit() {
    let report = DaemonStatusReport {
        socket_path: "/run/so\"os\\dir/daemon.sock".to_string(),
        socket_ready: true,
        camera_ready: false,
        models_verified: true,
        is_healthy: false,
        pid: None,
        uptime_secs: Some(42),
        systemd_unit: "soos\"-daemon.service".to_string(),
        systemd_active_state: "act\\ive".to_string(),
        systemd_sub_state: "run\"ning".to_string(),
    };
    let json = report.to_json();
    let parsed: DaemonStatusReport =
        serde_json::from_str(&json).expect("to_json must be valid JSON");
    assert_eq!(parsed, report);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value["pid"].is_null(), "an absent PID must stay JSON null");
}

//! `soos-admin status` shows the daemon swap protection state (GitHub #287, matrix DFU5).
//!
//! `StatusResponse::memory_locked` (GitHub #201) is surfaced as
//! `DaemonStatusReport::memory_locked`: `Some(true)` / `Some(false)` from a reachable daemon,
//! `None` (unknown, never "locked") when the daemon cannot be contacted.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use tempfile::tempdir;

use soos_admin_cli::status::{query_status, DaemonStatusReport};
use soos_protocol::codec::encode;
use soos_protocol::types::{StatusResponse, CURRENT_VERSION};

/// Serves one `StatusResponse` carrying `memory_locked` and returns the queried report.
fn report_from_mock_daemon(memory_locked: bool) -> DaemonStatusReport {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("dfu5.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length prefix");
        let mut body = vec![0u8; u32::from_be_bytes(len_buf) as usize];
        stream.read_exact(&mut body).expect("read body");
        let resp = StatusResponse {
            version: CURRENT_VERSION,
            socket_ready: true,
            camera_ready: true,
            models_verified: true,
            is_healthy: true,
            pid: 4242,
            uptime_secs: 7,
            memory_locked,
        };
        stream
            .write_all(&encode(&resp).expect("encode"))
            .expect("write status response");
    });
    let report = query_status(&socket_path, "soos-daemon").expect("query status");
    server.join().expect("join server");
    report
}

#[test]
fn test_status_reports_memory_locked_from_daemon() {
    let report = report_from_mock_daemon(true);
    assert_eq!(report.memory_locked, Some(true));
    let table = report.format_table();
    assert!(
        table.contains("Swap Protection:   LOCKED"),
        "the table must show the swap protection state: {table}"
    );
    assert!(report.to_json().contains("\"memory_locked\": true"));
}

#[test]
fn test_status_reports_memory_unlocked_from_daemon() {
    let report = report_from_mock_daemon(false);
    assert_eq!(report.memory_locked, Some(false));
    let table = report.format_table();
    assert!(
        table.contains("Swap Protection:   NOT LOCKED"),
        "an mlockall refusal must be visible: {table}"
    );
    assert!(report.to_json().contains("\"memory_locked\": false"));
}

#[test]
fn test_offline_status_never_claims_memory_locked() {
    let dir = tempdir().expect("tempdir");
    let report = query_status(
        &dir.path().join("absent.sock"),
        "non_existent_soos_daemon.service",
    )
    .expect("offline report");
    assert_eq!(
        report.memory_locked, None,
        "an unreachable daemon has an unknown swap protection state"
    );
    let table = report.format_table();
    assert!(table.contains("Swap Protection:   N/A"), "{table}");
    let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert!(value["memory_locked"].is_null());
}

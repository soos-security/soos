//! Contractual tests for daemon status querying (Sub-issue #11.2).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use tempfile::tempdir;

use soos_admin_cli::status::{query_status, DaemonStatusReport};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{Request, RequestKind, StatusResponse, CURRENT_VERSION};

#[test]
fn test_status_query_mock_daemon_healthy() {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("test_daemon.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");

        // Read framed request
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length prefix");
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).expect("read body");

        let mut full = len_buf.to_vec();
        full.extend_from_slice(&body);

        let req: Request = decode(&full).expect("decode request");
        assert_eq!(req.kind, RequestKind::Status);

        // Send StatusResponse
        let resp = StatusResponse {
            version: CURRENT_VERSION,
            socket_ready: true,
            camera_ready: true,
            models_verified: true,
            is_healthy: true,
            pid: 12345,
            uptime_secs: 120,
            memory_locked: true,
        };

        let encoded = encode(&resp).expect("encode status response");
        stream.write_all(&encoded).expect("write status response");
    });

    let report = query_status(&socket_path, "soos-daemon").expect("query status must succeed");
    server_handle.join().expect("join server thread");

    assert!(report.socket_ready);
    assert!(report.camera_ready);
    assert!(report.models_verified);
    assert!(report.is_healthy);
    assert_eq!(report.pid, Some(12345));
    assert_eq!(report.uptime_secs, Some(120));
    assert_eq!(report.systemd_unit, "soos-daemon");
}

#[test]
fn test_status_query_mock_daemon_component_unready() {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("test_daemon_unready.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length prefix");
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).expect("read body");

        let resp = StatusResponse {
            version: CURRENT_VERSION,
            socket_ready: true,
            camera_ready: false, // Camera offline
            models_verified: true,
            is_healthy: false,
            pid: 9999,
            uptime_secs: 5,
            memory_locked: false,
        };

        let encoded = encode(&resp).expect("encode status response");
        stream.write_all(&encoded).expect("write status response");
    });

    let report = query_status(&socket_path, "soos-daemon").expect("query status must succeed");
    server_handle.join().expect("join server thread");

    assert!(report.socket_ready);
    assert!(!report.camera_ready);
    assert!(report.models_verified);
    assert!(!report.is_healthy);
    assert_eq!(report.pid, Some(9999));
}

#[test]
fn test_status_query_offline_daemon_does_not_panic() {
    let dir = tempdir().expect("tempdir");
    let non_existent_socket = dir.path().join("non_existent.sock");

    let report = query_status(&non_existent_socket, "non_existent_soos_daemon.service").expect(
        "query status on offline daemon should return offline report rather than hard error",
    );

    assert!(!report.socket_ready);
    assert!(!report.camera_ready);
    assert!(!report.models_verified);
    assert!(!report.is_healthy);
    assert_eq!(report.pid, None);
    assert_eq!(report.uptime_secs, None);
}

#[test]
fn test_status_report_json_serialization() {
    let report = DaemonStatusReport {
        socket_path: "/run/soos/daemon.sock".to_string(),
        socket_ready: true,
        camera_ready: true,
        models_verified: true,
        is_healthy: true,
        pid: Some(1234),
        uptime_secs: Some(3600),
        memory_locked: Some(true),
        systemd_unit: "soos-daemon".to_string(),
        systemd_active_state: "active".to_string(),
        systemd_sub_state: "running".to_string(),
    };

    let json_str = report.to_json();
    assert!(json_str.contains("\"is_healthy\": true"));
    assert!(json_str.contains("\"pid\": 1234"));
    assert!(json_str.contains("\"uptime_secs\": 3600"));
    assert!(json_str.contains("\"socket_ready\": true"));
}

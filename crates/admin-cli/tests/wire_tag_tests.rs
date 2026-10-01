//! GitHub #204 (DMN-15): `soos-admin` requests carry an explicit message tag, so the
//! daemon classifies them by protocol rule rather than by a UID heuristic.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contract tests use assertions, unwrap and expect"
)]

#[path = "../../pam/tests/common/stamps.rs"]
mod stamps;

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;
use tempfile::tempdir;

use soos_admin_cli::status::query_status;
use soos_admin_cli::test_pam::simulate_pam_auth;
use soos_protocol::codec::encode;
use soos_protocol::message::{
    decode_client_message, ClientMessage, FrameFormat, MESSAGE_TAG_REQUEST,
};
use soos_protocol::types::{
    ReasonClass, Request, Response, StatusResponse, Verdict, CURRENT_VERSION,
};

fn read_tagged_request(stream: &mut UnixStream) -> (Request, FrameFormat, Option<u8>) {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).expect("read length");
    let mut body = vec![0u8; u32::from_be_bytes(len_buf) as usize];
    stream.read_exact(&mut body).expect("read body");
    let (msg, format) = decode_client_message(&body).expect("classified");
    let ClientMessage::Request(req) = msg else {
        panic!("admin frames must classify as a Request");
    };
    (req, format, body.last().copied())
}

#[test]
fn test_204_admin_status_request_frame_is_tagged() {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("tagged_status.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let (_req, format, last) = read_tagged_request(&mut stream);
        let resp = StatusResponse {
            version: CURRENT_VERSION,
            socket_ready: true,
            camera_ready: true,
            models_verified: true,
            is_healthy: true,
            pid: 1,
            uptime_secs: 1,
            memory_locked: false,
        };
        stream
            .write_all(&encode(&resp).expect("encode"))
            .expect("write");
        (format, last)
    });

    let report = query_status(&socket_path, "soos-daemon").expect("status");
    let (format, last) = server.join().expect("server");
    assert!(report.is_healthy);
    assert_eq!(format, FrameFormat::Tagged);
    assert_eq!(last, Some(MESSAGE_TAG_REQUEST));
}

#[test]
fn test_204_admin_test_pam_request_frame_is_tagged() {
    let dir = tempdir().expect("tempdir");
    let socket_path = dir.path().join("tagged_test_pam.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let (req, format, last) = read_tagged_request(&mut stream);
        let (issued, expires) = stamps::fresh_stamps();
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Deny,
            reason_class: ReasonClass::NoFace,
            issued_monotonic_ns: issued,
            expires_monotonic_ns: expires,
        };
        stream
            .write_all(&encode(&resp).expect("encode"))
            .expect("write");
        (format, last)
    });

    let report = simulate_pam_auth(&socket_path, 1000, "soos-admin", 1000).expect("test-pam");
    let (format, last) = server.join().expect("server");
    assert_eq!(report.verdict, Verdict::Deny);
    assert_eq!(format, FrameFormat::Tagged);
    assert_eq!(last, Some(MESSAGE_TAG_REQUEST));
}

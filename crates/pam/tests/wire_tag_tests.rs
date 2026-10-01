//! GitHub #204 (DMN-15): every frame the PAM module sends carries an explicit message tag,
//! so the daemon never has to guess whether it is a `Request` or an `Event`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contract tests use assertions, unwrap and expect"
)]

#[path = "common/stamps.rs"]
mod stamps;

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use tempfile::tempdir;

use soos_protocol::codec::encode;
use soos_protocol::message::{
    decode_client_message, ClientMessage, FrameFormat, MESSAGE_TAG_EVENT, MESSAGE_TAG_REQUEST,
};
use soos_protocol::types::{EventKind, ReasonClass, Response, Verdict, CURRENT_VERSION};

fn read_payload(stream: &mut std::os::unix::net::UnixStream) -> Vec<u8> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).expect("read length");
    let size = u32::from_be_bytes(len_buf) as usize;
    let mut body = vec![0u8; size];
    stream.read_exact(&mut body).expect("read body");
    body
}

#[test]
fn test_204_pam_auth_request_frame_is_tagged() {
    let tmp = tempdir().expect("tempdir");
    let sock_path = tmp.path().join("tagged_auth.sock");
    let listener = UnixListener::bind(&sock_path).expect("bind");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let payload = read_payload(&mut stream);
        let (msg, format) = decode_client_message(&payload).expect("classified");
        let ClientMessage::Request(req) = msg else {
            panic!("PAM auth frame must classify as a Request");
        };
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
        (format, payload.last().copied())
    });

    let config = pam_soos::config::PamConfig {
        socket_path: sock_path.clone(),
        timeout_ms: 1000,
        ..Default::default()
    };
    let result = pam_soos::ipc::authenticate(&config, 1000);
    let (format, last) = server.join().expect("server");
    assert_eq!(format, FrameFormat::Tagged);
    assert_eq!(last, Some(MESSAGE_TAG_REQUEST));
    assert!(matches!(result, Ok((Verdict::Deny, _))), "got {result:?}");
}

#[test]
fn test_204_pam_event_frame_is_tagged() {
    let tmp = tempdir().expect("tempdir");
    let sock_path = tmp.path().join("tagged_event.sock");
    let listener = UnixListener::bind(&sock_path).expect("bind");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let payload = read_payload(&mut stream);
        let (msg, format) = decode_client_message(&payload).expect("classified");
        let ClientMessage::Event(event) = msg else {
            panic!("PAM event frame must classify as an Event");
        };
        assert_eq!(event.kind, EventKind::PasswordFailed);
        assert_eq!(event.uid, Some(1001));
        (format, payload.last().copied())
    });

    let config = pam_soos::config::PamConfig {
        socket_path: sock_path.clone(),
        ..Default::default()
    };
    pam_soos::ipc::notify_event(&config, 1001, EventKind::PasswordFailed).expect("sent");
    let (format, last) = server.join().expect("server");
    assert_eq!(format, FrameFormat::Tagged);
    assert_eq!(last, Some(MESSAGE_TAG_EVENT));
}

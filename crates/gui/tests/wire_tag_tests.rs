//! GitHub #204 (DMN-15): `soos-gui` preview requests carry an explicit message tag.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contract tests use assertions, unwrap and expect"
)]

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;

use soos_gui::IpcCameraManager;
use soos_protocol::codec::encode_preview;
use soos_protocol::message::{
    decode_client_message, ClientMessage, FrameFormat, MESSAGE_TAG_REQUEST,
};
use soos_protocol::types::{PreviewResponse, RequestKind, CURRENT_VERSION};

#[test]
fn test_204_gui_preview_request_frame_is_tagged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock_path = dir.path().join("tagged_preview.sock");
    let listener = UnixListener::bind(&sock_path).expect("bind");

    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let mut body = vec![0u8; u32::from_be_bytes(len_buf) as usize];
        stream.read_exact(&mut body).expect("read body");
        let (msg, format) = decode_client_message(&body).expect("classified");
        let ClientMessage::Request(req) = msg else {
            panic!("preview frame must classify as a Request");
        };
        assert_eq!(req.kind, RequestKind::PreviewFrame);
        let empty = PreviewResponse {
            version: CURRENT_VERSION,
            sequence: 0,
            width: 0,
            height: 0,
            format: 255,
            timestamp_monotonic_ns: 0,
            data: Vec::new(),
        };
        stream
            .write_all(&encode_preview(&empty).expect("encode"))
            .expect("write");
        stream.flush().expect("flush");
        (format, body.last().copied())
    });

    assert_eq!(IpcCameraManager::probe_preview(&sock_path), Ok(()));
    let (format, last) = server.join().expect("server");
    assert_eq!(format, FrameFormat::Tagged);
    assert_eq!(last, Some(MESSAGE_TAG_REQUEST));
}

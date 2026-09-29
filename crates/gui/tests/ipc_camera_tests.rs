//! Contractual tests for `IpcCameraManager` authorization handling (GitHub #143, CAM-01).
//!
//! Contract:
//! - Every preview request carries a fresh random nonce and the caller's real UID as `uid_hint`.
//! - A daemon denial (`Response` with `ProtocolError`) is surfaced as `IpcPreviewError::Unauthorized`
//!   and stops the polling worker (no reconnect storm against the privileged daemon).
//! - A rate-limit reply is surfaced as `IpcPreviewError::RateLimited` and polling continues.
//! - `probe_preview` performs a single authorization round-trip before the GUI commits to IPC.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use soos_camera_v4l::CameraManager;
use soos_gui::{IpcCameraManager, IpcPreviewError};
use soos_protocol::codec::{decode, encode, encode_preview};
use soos_protocol::types::{
    PreviewResponse, ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION,
};

fn temp_socket(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("soos_gui_{tag}_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn read_request(stream: &mut UnixStream) -> Option<Request> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).ok()?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + len];
    buf[..4].copy_from_slice(&len_bytes);
    stream.read_exact(&mut buf[4..]).ok()?;
    decode::<Request>(&buf).ok()
}

fn denial(request_id: [u8; 32], reason: ReasonClass) -> Vec<u8> {
    let resp = Response {
        version: CURRENT_VERSION,
        request_id,
        verdict: Verdict::ProtocolError,
        reason_class: reason,
        issued_monotonic_ns: 1,
        expires_monotonic_ns: 2,
    };
    encode(&resp).unwrap()
}

fn frame(seq: u64) -> Vec<u8> {
    let resp = PreviewResponse {
        version: CURRENT_VERSION,
        sequence: seq,
        width: 32,
        height: 16,
        format: 0,
        timestamp_monotonic_ns: 1000 + seq,
        data: vec![90u8; 32 * 16 * 3],
    };
    encode_preview(&resp).unwrap()
}

#[test]
fn test_ipc_camera_manager_stops_on_unauthorized_response() {
    let sock_path = temp_socket("unauthorized");
    let listener = UnixListener::bind(&sock_path).unwrap();
    listener
        .set_nonblocking(false)
        .expect("blocking listener for first accept");

    let expected_uid = nix::unistd::getuid().as_raw();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let req = read_request(&mut stream).expect("First preview request");
        assert_eq!(req.kind, RequestKind::PreviewFrame);
        assert_eq!(req.uid_hint, expected_uid, "uid_hint must be the real UID");
        assert_ne!(
            req.request_id, [0x5A; 32],
            "Nonce must not be the historical constant"
        );
        stream
            .write_all(&denial(req.request_id, ReasonClass::UidMismatch))
            .unwrap();
        stream.flush().unwrap();

        // The client must stop: no further request on this stream (clean EOF)...
        let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
        let mut one = [0u8; 1];
        let n = stream.read(&mut one).unwrap_or(0);
        assert_eq!(n, 0, "Unauthorized client must not send further requests");

        // ...and no reconnection attempt either.
        listener.set_nonblocking(true).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            listener.accept().is_err(),
            "Unauthorized client must not reconnect"
        );
    });

    let manager = IpcCameraManager::spawn(&sock_path);
    let start = Instant::now();
    while manager.last_error().is_none() && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(manager.last_error(), Some(IpcPreviewError::Unauthorized));
    assert!(!manager.is_ready());
    assert!(manager.latest_frame().is_none());

    server.join().unwrap();
    let _ = std::fs::remove_file(&sock_path);
}

#[test]
fn test_ipc_camera_manager_keeps_polling_after_rate_limit() {
    let sock_path = temp_socket("ratelimit");
    let listener = UnixListener::bind(&sock_path).unwrap();

    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let first = read_request(&mut stream).expect("First request");
        stream
            .write_all(&denial(first.request_id, ReasonClass::RateLimited))
            .unwrap();
        stream.flush().unwrap();

        let second = read_request(&mut stream).expect("Client must retry after rate limit");
        assert_ne!(
            first.request_id, second.request_id,
            "Each request must carry a fresh nonce"
        );
        stream.write_all(&frame(1)).unwrap();
        stream.flush().unwrap();
        // Keep the stream open until the client observed the frame.
        std::thread::sleep(Duration::from_millis(300));
    });

    let manager = IpcCameraManager::spawn(&sock_path);
    let start = Instant::now();
    while !manager.is_ready() && start.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        manager.is_ready(),
        "Manager must recover after a rate-limit reply"
    );
    let f = manager.latest_frame().expect("Frame must exist");
    assert_eq!(f.width, 32);
    assert_eq!(f.height, 16);
    assert_eq!(
        manager.last_error(),
        None,
        "A served frame must clear the last error"
    );

    drop(manager);
    server.join().unwrap();
    let _ = std::fs::remove_file(&sock_path);
}

#[test]
fn test_probe_preview_reports_unauthorized() {
    let sock_path = temp_socket("probe_denied");
    let listener = UnixListener::bind(&sock_path).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let req = read_request(&mut stream).expect("Probe request");
        stream
            .write_all(&denial(req.request_id, ReasonClass::UidMismatch))
            .unwrap();
        stream.flush().unwrap();
    });

    let result = IpcCameraManager::probe_preview(&sock_path);
    assert_eq!(result, Err(IpcPreviewError::Unauthorized));

    server.join().unwrap();
    let _ = std::fs::remove_file(&sock_path);
}

#[test]
fn test_probe_preview_accepts_authorized_reply() {
    let sock_path = temp_socket("probe_ok");
    let listener = UnixListener::bind(&sock_path).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream).expect("Probe request");
        // A daemon without a warm camera answers with an empty preview (format 255).
        let empty = PreviewResponse {
            version: CURRENT_VERSION,
            sequence: 0,
            width: 0,
            height: 0,
            format: 255,
            timestamp_monotonic_ns: 0,
            data: Vec::new(),
        };
        stream.write_all(&encode_preview(&empty).unwrap()).unwrap();
        stream.flush().unwrap();
    });

    assert_eq!(IpcCameraManager::probe_preview(&sock_path), Ok(()));

    server.join().unwrap();
    let _ = std::fs::remove_file(&sock_path);
}

#[test]
fn test_probe_preview_reports_io_error_when_socket_absent() {
    let sock_path = temp_socket("probe_absent");
    assert_eq!(
        IpcCameraManager::probe_preview(&sock_path),
        Err(IpcPreviewError::Io)
    );
}

#[test]
fn test_ipc_preview_error_display_is_english_and_specific() {
    assert!(IpcPreviewError::Unauthorized
        .to_string()
        .contains("not authorized"));
    assert!(IpcPreviewError::RateLimited.to_string().contains("rate"));
}

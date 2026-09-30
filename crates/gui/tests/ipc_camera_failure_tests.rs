//! Hermetic GUI camera failure-path contract (GitHub #198 / CAM-16).
//!
//! The existing IPC suite covers the authorized happy path, the unauthorized stop and the
//! rate-limit retry. This suite covers the remaining failure modes named by the review, each
//! against a fake daemon socket or a mock camera (no daemon, no camera, no display):
//! - oversized and zero-length preview replies are rejected before any allocation of the
//!   declared size and surface as `SourceProtocol`, and the worker reconnects;
//! - a daemon without a camera (`Verdict::Unavailable`) surfaces as `SourceUnavailable` and the
//!   worker keeps polling until frames resume;
//! - an unreadable socket (`EACCES`) surfaces as `SourceUnreachable`;
//! - direct-mode `EACCES` / `EBUSY` from the V4L2 device reach the banner with distinct,
//!   actionable titles.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_camera_v4l::{
    CameraError, CameraErrorKind, CameraManager, CameraStatus, MockCameraManager,
};
use soos_gui::camera_status::{camera_status_banner, BannerSeverity};
use soos_gui::{IpcCameraManager, IpcPreviewError};
use soos_protocol::codec::{decode, encode, encode_preview};
use soos_protocol::types::{
    PreviewResponse, ReasonClass, Request, Response, Verdict, CURRENT_VERSION,
    MAX_PREVIEW_MESSAGE_SIZE,
};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn socket_in(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("daemon.sock")
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

fn unavailable(request_id: [u8; 32]) -> Vec<u8> {
    encode(&Response {
        version: CURRENT_VERSION,
        request_id,
        verdict: Verdict::Unavailable,
        reason_class: ReasonClass::CameraUnavailable,
        issued_monotonic_ns: 1,
        expires_monotonic_ns: 2,
    })
    .unwrap()
}

fn frame(seq: u64) -> Vec<u8> {
    encode_preview(&PreviewResponse {
        version: CURRENT_VERSION,
        sequence: seq,
        width: 8,
        height: 4,
        format: 0,
        timestamp_monotonic_ns: 1000 + seq,
        data: vec![7u8; 8 * 4 * 3],
    })
    .unwrap()
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    condition()
}

/// Matrix CHT5: a reply whose length prefix exceeds `MAX_PREVIEW_MESSAGE_SIZE` is refused as a
/// protocol error from the 4-byte header alone (the client never waits for the body).
#[test]
fn test_probe_preview_rejects_oversized_declared_length() {
    let dir = tempfile::tempdir().unwrap();
    let sock = socket_in(&dir);
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream).expect("probe request");
        let oversized = u32::try_from(MAX_PREVIEW_MESSAGE_SIZE + 1).unwrap();
        stream.write_all(&oversized.to_be_bytes()).unwrap();
        stream.flush().unwrap();
        // Keep the stream open: a client waiting for the body would hit its read timeout.
        std::thread::sleep(Duration::from_millis(600));
    });

    let start = Instant::now();
    let result = IpcCameraManager::probe_preview(&sock);
    assert_eq!(result, Err(IpcPreviewError::Protocol));
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "an oversized header must be rejected without reading the body"
    );
    assert_eq!(
        IpcPreviewError::Protocol.kind(),
        CameraErrorKind::SourceProtocol
    );
    server.join().unwrap();
}

/// Matrix CHT5: a zero-length frame is a protocol error, not an empty preview.
#[test]
fn test_probe_preview_rejects_zero_length_reply() {
    let dir = tempfile::tempdir().unwrap();
    let sock = socket_in(&dir);
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream).expect("probe request");
        stream.write_all(&0u32.to_be_bytes()).unwrap();
        stream.flush().unwrap();
    });
    assert_eq!(
        IpcCameraManager::probe_preview(&sock),
        Err(IpcPreviewError::Protocol)
    );
    server.join().unwrap();
}

/// Matrix CHT5: a truncated reply (length prefix promises more bytes than the daemon sends
/// before closing) is a transport failure, never a partially decoded frame.
#[test]
fn test_probe_preview_reports_io_on_truncated_reply() {
    let dir = tempfile::tempdir().unwrap();
    let sock = socket_in(&dir);
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream).expect("probe request");
        let full = frame(1);
        stream.write_all(&full[..full.len() / 2]).unwrap();
        stream.flush().unwrap();
    });
    assert_eq!(
        IpcCameraManager::probe_preview(&sock),
        Err(IpcPreviewError::Io)
    );
    server.join().unwrap();
}

/// Matrix CHT5: after an oversized reply the worker drops the connection, reports
/// `SourceProtocol`, reconnects and recovers when the daemon serves a valid frame.
#[test]
fn test_ipc_camera_manager_reconnects_after_oversized_reply() {
    let dir = tempfile::tempdir().unwrap();
    let sock = socket_in(&dir);
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        let _ = read_request(&mut first).expect("first request");
        let oversized = u32::try_from(MAX_PREVIEW_MESSAGE_SIZE + 1).unwrap();
        first.write_all(&oversized.to_be_bytes()).unwrap();
        first.flush().unwrap();

        let (mut second, _) = listener.accept().unwrap();
        let _ = read_request(&mut second).expect("request after reconnect");
        second.write_all(&frame(2)).unwrap();
        second.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        drop(first);
    });

    let manager = IpcCameraManager::spawn(&sock);
    assert!(
        wait_until(Duration::from_secs(5), || manager.is_ready()),
        "the worker must reconnect and recover after a protocol violation"
    );
    assert_eq!(manager.latest_frame().unwrap().sequence, 2);
    assert_eq!(manager.last_error(), None);
    drop(manager);
    server.join().unwrap();
}

/// Matrix CHT6: a daemon that has no camera frame (`Verdict::Unavailable`) is reported as
/// `SourceUnavailable`; the worker keeps polling on the same connection and recovers.
#[test]
fn test_ipc_camera_manager_reports_unavailable_and_keeps_polling() {
    let dir = tempfile::tempdir().unwrap();
    let sock = socket_in(&dir);
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let first = read_request(&mut stream).expect("first request");
        stream.write_all(&unavailable(first.request_id)).unwrap();
        stream.flush().unwrap();
        // Wait until the client observed the unavailable status before serving frames.
        let _ = rx.recv_timeout(Duration::from_secs(5));
        loop {
            let Some(req) = read_request(&mut stream) else {
                break;
            };
            let _ = req;
            if stream.write_all(&frame(3)).is_err() {
                break;
            }
            let _ = stream.flush();
        }
    });

    let manager = IpcCameraManager::spawn(&sock);
    assert!(wait_until(Duration::from_secs(5), || {
        manager.last_error() == Some(IpcPreviewError::Unavailable)
    }));
    assert!(!manager.is_ready());
    assert!(matches!(
        manager.status(),
        CameraStatus::Error {
            kind: CameraErrorKind::SourceUnavailable,
            ..
        }
    ));
    let banner = camera_status_banner(&manager.status());
    assert_eq!(banner.title, "Daemon camera unavailable");
    assert!(banner.detail.contains("retrying automatically"));

    tx.send(()).unwrap();
    assert!(
        wait_until(Duration::from_secs(5), || manager.is_ready()),
        "polling must continue after an unavailable reply"
    );
    assert_eq!(manager.status(), CameraStatus::Ready);
    drop(manager);
    server.join().unwrap();
}

/// Matrix CHT6: a socket the user may not write (`EACCES` on `connect(2)`) is reported as an
/// unreachable daemon, never as authorized. Root bypasses file permissions, so the check is
/// skipped when the suite runs as root.
#[test]
fn test_probe_preview_reports_io_when_socket_permission_denied() {
    if nix::unistd::geteuid().is_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let sock = socket_in(&dir);
    let _listener = UnixListener::bind(&sock).unwrap();
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o000)).unwrap();

    let err = UnixStream::connect(&sock).expect_err("connect must be refused");
    assert_eq!(err.raw_os_error(), Some(libc_eacces()));
    assert_eq!(
        IpcCameraManager::probe_preview(&sock),
        Err(IpcPreviewError::Io)
    );
    assert_eq!(
        IpcPreviewError::Io.kind(),
        CameraErrorKind::SourceUnreachable
    );
}

/// `EACCES` without a `libc` dev-dependency (value fixed by the Linux ABI).
fn libc_eacces() -> i32 {
    nix::errno::Errno::EACCES as i32
}

/// Matrix CHT6: direct-mode device failures (`EACCES`, `EBUSY`) from the V4L2 layer reach the
/// banner with distinct, actionable, retrying error titles.
#[test]
fn test_direct_mode_permission_and_busy_errors_reach_the_banner() {
    let camera = MockCameraManager::new_default();

    for (code, kind, title, hint) in [
        (
            nix::errno::Errno::EACCES as i32,
            CameraErrorKind::PermissionDenied,
            "Camera permission denied",
            "video",
        ),
        (
            nix::errno::Errno::EBUSY as i32,
            CameraErrorKind::DeviceBusy,
            "Camera is busy",
            "Pause the daemon",
        ),
    ] {
        let error = CameraError::from_io_error(
            PathBuf::from("/dev/video0"),
            std::io::Error::from_raw_os_error(code),
        );
        assert_eq!(error.kind(), kind);
        camera.set_error(Some(error));
        assert!(!camera.is_ready());
        let status = camera.status();
        assert!(
            matches!(status, CameraStatus::Error { kind: k, .. } if k == kind),
            "mock status {status:?} must report {kind:?}"
        );
        let banner = camera_status_banner(&status);
        assert_eq!(banner.severity, BannerSeverity::Error);
        assert_eq!(banner.title, title);
        assert!(banner.detail.contains(hint), "{}", banner.detail);
        assert!(banner.detail.contains("retrying automatically"));
    }
    camera.set_error(None);
}

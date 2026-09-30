//! Contractual tests for the GUI camera-mode decision (GitHub #150, review finding CAM-02).
//!
//! The root daemon is the exclusive owner of `/dev/video*`. The GUI may open the camera directly
//! only when the daemon is provably not running; otherwise it streams through the daemon IPC or
//! shows an actionable "blocked" state. It never silently falls back to direct V4L2 access (which
//! caused the EBUSY ping-pong between the GUI and the daemon).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite uses assertions and unwrap"
)]

use soos_camera_v4l::CameraManager;
use soos_gui::camera_mode::{
    classify_socket_error, decide_camera_mode, probe_daemon_socket, CameraBlockReason, CameraMode,
    DaemonSocketProbe, UnavailableCameraManager,
};
use soos_gui::IpcPreviewError;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

fn temp_socket(tag: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("soos_gui_mode_{tag}_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn test_probe_classifies_eacces_as_permission_denied() {
    for errno in [libc_eacces(), libc_eperm()] {
        assert_eq!(
            classify_socket_error(&std::io::Error::from_raw_os_error(errno)),
            DaemonSocketProbe::PermissionDenied,
            "errno {errno} must be classified as a missing 'soos' group membership"
        );
    }
}

#[test]
fn test_probe_classifies_missing_and_stale_socket_as_not_running() {
    let missing = temp_socket("missing");
    assert_eq!(probe_daemon_socket(&missing), DaemonSocketProbe::NotRunning);

    // A socket file left behind by a stopped daemon refuses connections (ECONNREFUSED).
    let stale = temp_socket("stale");
    drop(UnixListener::bind(&stale).unwrap());
    assert_eq!(probe_daemon_socket(&stale), DaemonSocketProbe::NotRunning);
    let _ = std::fs::remove_file(&stale);
}

#[test]
fn test_probe_classifies_listening_socket_as_reachable() {
    let path = temp_socket("reachable");
    let _listener = UnixListener::bind(&path).unwrap();
    assert_eq!(probe_daemon_socket(&path), DaemonSocketProbe::Reachable);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_direct_mode_refused_while_daemon_active() {
    // Daemon active but socket unreachable (starting up, or relocated): never go direct.
    assert_eq!(
        decide_camera_mode(DaemonSocketProbe::NotRunning, None, true),
        CameraMode::Blocked(CameraBlockReason::DaemonUnreachable)
    );
    assert_eq!(
        decide_camera_mode(DaemonSocketProbe::Other, None, true),
        CameraMode::Blocked(CameraBlockReason::DaemonUnreachable)
    );
    // EACCES means /run/soos exists and the daemon owns the camera: blocked regardless of systemd.
    for active in [true, false] {
        assert_eq!(
            decide_camera_mode(DaemonSocketProbe::PermissionDenied, None, active),
            CameraMode::Blocked(CameraBlockReason::PermissionDenied)
        );
    }
    // Daemon reachable but preview refused: the old code fell back to direct V4L2 here.
    for active in [true, false] {
        assert_eq!(
            decide_camera_mode(
                DaemonSocketProbe::Reachable,
                Some(Err(IpcPreviewError::Unauthorized)),
                active
            ),
            CameraMode::Blocked(CameraBlockReason::PreviewUnavailable(
                IpcPreviewError::Unauthorized
            ))
        );
    }
    // A reachable daemon whose preview probe was not performed is still the device owner.
    assert_eq!(
        decide_camera_mode(DaemonSocketProbe::Reachable, None, false),
        CameraMode::Blocked(CameraBlockReason::PreviewUnavailable(IpcPreviewError::Io))
    );
}

#[test]
fn test_ipc_and_direct_modes_selected_only_when_safe() {
    assert_eq!(
        decide_camera_mode(DaemonSocketProbe::Reachable, Some(Ok(())), true),
        CameraMode::DaemonIpc
    );
    assert_eq!(
        decide_camera_mode(DaemonSocketProbe::NotRunning, None, false),
        CameraMode::DirectV4l
    );
    assert_eq!(
        decide_camera_mode(DaemonSocketProbe::Other, None, false),
        CameraMode::DirectV4l
    );
}

#[test]
fn test_blocked_reasons_are_actionable() {
    let perm = CameraBlockReason::PermissionDenied.user_message();
    assert!(perm.contains("soos"), "{perm}");
    assert!(perm.contains("soos-admin add-user"), "{perm}");
    assert!(perm.contains("log out"), "{perm}");

    let unauthorized =
        CameraBlockReason::PreviewUnavailable(IpcPreviewError::Unauthorized).user_message();
    assert!(unauthorized.contains("[preview]"), "{unauthorized}");
    assert!(unauthorized.contains("allowed_uids"), "{unauthorized}");

    let unreachable = CameraBlockReason::DaemonUnreachable.user_message();
    assert!(unreachable.contains("soos-daemon"), "{unreachable}");

    for reason in [
        CameraBlockReason::PermissionDenied,
        CameraBlockReason::DaemonUnreachable,
        CameraBlockReason::PreviewUnavailable(IpcPreviewError::RateLimited),
    ] {
        assert!(
            reason
                .user_message()
                .contains("will not open the camera directly"),
            "every blocked state must explain why the GUI does not grab the device"
        );
    }
}

#[test]
fn test_unavailable_camera_manager_never_serves_frames() {
    let cam = UnavailableCameraManager;
    cam.notify_activity();
    assert!(!cam.is_ready());
    assert!(cam.latest_frame().is_none());
    cam.stop();
    assert!(!cam.is_ready());
}

const fn libc_eacces() -> i32 {
    13
}

const fn libc_eperm() -> i32 {
    1
}

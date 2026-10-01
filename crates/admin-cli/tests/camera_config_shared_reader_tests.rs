//! `soos-admin camera list` uses the shared `daemon.toml` reader of `soos-camera-v4l`
//! (GitHub #289, rows DGP4 and DGP7).
//!
//! The admin types are the shared types (one implementation); a symbolic link is followed like
//! soos-daemon follows it, and a FIFO in place of the configuration falls back to the
//! soos-daemon defaults with a note, promptly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite uses assertions and unwrap"
)]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use soos_admin_cli::camera::{resolve_list_settings, CameraListSettings, SettingOrigin};
use soos_camera_v4l::SensorPreference;

fn resolve_with_deadline(path: &Path) -> CameraListSettings {
    let (tx, rx) = mpsc::channel();
    let owned = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(resolve_list_settings(None, None, &owned));
    });
    rx.recv_timeout(Duration::from_secs(2))
        .expect("camera list must not block on the configuration path")
}

#[test]
fn test_dgp_admin_daemon_config_is_the_shared_reader() {
    // Same types, not look-alikes: a value of one is a value of the other.
    let shared: soos_camera_v4l::daemon_config::DaemonConfigError =
        soos_admin_cli::daemon_config::DaemonConfigError::NotFound;
    assert_eq!(
        shared,
        soos_camera_v4l::daemon_config::DaemonConfigError::NotFound
    );
    let settings: soos_camera_v4l::daemon_config::DaemonCameraSettings =
        soos_admin_cli::daemon_config::DaemonCameraSettings::default();
    assert_eq!(settings.camera_device, None);
    assert_eq!(
        soos_admin_cli::daemon_config::MAX_DAEMON_CONFIG_BYTES,
        soos_camera_v4l::daemon_config::MAX_DAEMON_CONFIG_BYTES
    );
    assert_eq!(
        soos_admin_cli::daemon_config::DEFAULT_DAEMON_CONFIG_PATH,
        soos_camera_v4l::daemon_config::DEFAULT_DAEMON_CONFIG_PATH
    );
}

#[test]
fn test_dgp_camera_list_fifo_config_falls_back_with_note() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("daemon.toml");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    let resolved = resolve_with_deadline(&fifo);
    assert_eq!(resolved.explicit_device, None);
    assert_eq!(resolved.sensor_preference, SensorPreference::PreferIr);
    assert_eq!(resolved.preference_origin, SettingOrigin::Default);
    let note = resolved.note();
    assert!(note.contains("is not a regular file"), "{note}");
    assert!(note.contains("soos-daemon defaults"), "{note}");
}

#[test]
fn test_dgp_camera_list_follows_a_symlinked_config_like_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("real.toml");
    std::fs::write(&target, "[pipeline]\ncamera_device = \"/dev/video4\"\n").unwrap();
    let link: PathBuf = dir.path().join("daemon.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let resolved = resolve_with_deadline(&link);
    assert_eq!(
        resolved.explicit_device,
        Some(PathBuf::from("/dev/video4")),
        "the link is followed: camera list reads the file soos-daemon reads"
    );
    assert_eq!(resolved.device_origin, SettingOrigin::DaemonConfig);
    assert!(resolved.note().contains("(loaded)"), "{}", resolved.note());
}

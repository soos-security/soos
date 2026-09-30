//! Contractual tests: `soos-enroll` and `soos-gui` (direct mode) resolve the camera exactly like
//! `soos-daemon` (GitHub #152, review finding CAM-04).
//!
//! `resolve_camera_device_from_config_with` is the enumerator-injectable form of the resolver used
//! by both binaries; it must delegate to `soos_camera_v4l::resolve_camera_device` so that a
//! missing `/etc/soos/daemon.toml`, a missing `camera_device` key, `"auto"` and `"default"` all
//! select the same node as the daemon (IR first by default), never the alphabetically first
//! `/dev/v4l/by-id/` entry.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite uses assertions and unwrap"
)]

use soos_camera_v4l::{
    resolve_camera_device, CameraDeviceInfo, CameraEnumerator, PixelFormat, SensorPreference,
};
use soos_enrollment_cli::service::resolve_camera_device_from_config_with;
use std::path::{Path, PathBuf};

struct FakeEnumerator;

impl CameraEnumerator for FakeEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        vec![
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video0"),
                card_name: "Integrated Camera: Integrated C".to_string(),
                supported_formats: vec![PixelFormat::Mjpeg, PixelFormat::Yuyv],
            },
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video2"),
                card_name: "Integrated Camera: Integrated I".to_string(),
                supported_formats: vec![PixelFormat::Grey],
            },
        ]
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        vec![
            // Alphabetically first entry is a metadata node: the legacy CLI resolver returned it.
            (
                PathBuf::from("/dev/v4l/by-id/usb-AAA_Cam-video-index1"),
                PathBuf::from("/dev/video1"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-BBB_Cam_RGB-video-index0"),
                PathBuf::from("/dev/video0"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-CCC_Cam_IR-video-index0"),
                PathBuf::from("/dev/video2"),
            ),
        ]
    }
}

const IR_ALIAS: &str = "/dev/v4l/by-id/usb-CCC_Cam_IR-video-index0";
const RGB_ALIAS: &str = "/dev/v4l/by-id/usb-BBB_Cam_RGB-video-index0";

fn write_config(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("daemon.toml");
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn test_no_config_prefers_ir_for_daemon_cli_and_gui() {
    let daemon_choice = resolve_camera_device(None, SensorPreference::default(), &FakeEnumerator);
    assert_eq!(daemon_choice.path, PathBuf::from(IR_ALIAS));

    // No daemon.toml at all.
    let tmp = tempfile::tempdir().unwrap();
    let absent = tmp.path().join("missing.toml");
    let cli_choice = resolve_camera_device_from_config_with(None, Some(&absent), &FakeEnumerator);
    assert_eq!(
        cli_choice, daemon_choice.path,
        "Without daemon.toml the CLI/GUI must pick the daemon's device"
    );

    // No config path passed at all.
    let no_path = resolve_camera_device_from_config_with(None, None, &FakeEnumerator);
    assert_eq!(no_path, daemon_choice.path);

    // daemon.toml present but without a camera_device key.
    let cfg = write_config(tmp.path(), "[pipeline]\nwarmup_frames = 0\n");
    let no_key = resolve_camera_device_from_config_with(None, Some(&cfg), &FakeEnumerator);
    assert_eq!(
        no_key, daemon_choice.path,
        "A daemon.toml without camera_device must not fall back to the first by-id entry"
    );
}

#[test]
fn test_cli_default_sentinel_handled_like_auto() {
    let tmp = tempfile::tempdir().unwrap();
    for value in ["auto", "default", ""] {
        let cfg = write_config(
            tmp.path(),
            &format!("[pipeline]\ncamera_device = \"{value}\"\n"),
        );
        let resolved = resolve_camera_device_from_config_with(None, Some(&cfg), &FakeEnumerator);
        assert_eq!(
            resolved,
            PathBuf::from(IR_ALIAS),
            "camera_device = '{value}' must auto-detect like the daemon"
        );
    }
    for cli in ["auto", "default"] {
        let resolved =
            resolve_camera_device_from_config_with(Some(PathBuf::from(cli)), None, &FakeEnumerator);
        assert_eq!(resolved, PathBuf::from(IR_ALIAS));
    }
}

#[test]
fn test_cli_sensor_preference_from_config_matches_daemon() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = write_config(
        tmp.path(),
        "[pipeline]\ncamera_device = \"auto\"\nsensor_preference = \"prefer_rgb\"\n",
    );
    let resolved = resolve_camera_device_from_config_with(None, Some(&cfg), &FakeEnumerator);
    let daemon_choice = resolve_camera_device(None, SensorPreference::PreferRgb, &FakeEnumerator);
    assert_eq!(resolved, PathBuf::from(RGB_ALIAS));
    assert_eq!(resolved, daemon_choice.path);
}

#[test]
fn test_cli_explicit_precedence_is_cli_then_config() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = write_config(tmp.path(), "[pipeline]\ncamera_device = \"/dev/video42\"\n");
    assert_eq!(
        resolve_camera_device_from_config_with(None, Some(&cfg), &FakeEnumerator),
        PathBuf::from("/dev/video42")
    );
    assert_eq!(
        resolve_camera_device_from_config_with(
            Some(PathBuf::from("/dev/video99")),
            Some(&cfg),
            &FakeEnumerator
        ),
        PathBuf::from("/dev/video99")
    );
}

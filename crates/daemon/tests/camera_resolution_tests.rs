//! Contractual tests: the daemon resolves its camera through the single shared resolver
//! (GitHub #152, review finding CAM-04).
//!
//! - No `camera_device` key, `"auto"`, `"default"` and `""` all keep the auto sentinel in the
//!   parsed configuration and select the same node as `soos-enroll` / `soos-gui`.
//! - An explicit `camera_device` is honored verbatim (no silent re-selection when the node is
//!   absent at startup).
//! - The mock camera never triggers hardware enumeration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite uses assertions and unwrap"
)]

use soos_camera_v4l::{
    resolve_camera_device, CameraDeviceInfo, CameraEnumerator, PixelFormat, SensorPreference,
    AUTO_CAMERA_DEVICE,
};
use soos_daemon::config::DaemonConfig;
use soos_daemon::pipeline::resolve_pipeline_camera;
use std::cell::Cell;
use std::path::PathBuf;

#[derive(Default)]
struct FakeEnumerator {
    calls: Cell<usize>,
}

impl CameraEnumerator for FakeEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        self.calls.set(self.calls.get().saturating_add(1));
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
            (
                PathBuf::from("/dev/v4l/by-id/usb-Cam_RGB-video-index0"),
                PathBuf::from("/dev/video0"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-Cam_IR-video-index0"),
                PathBuf::from("/dev/video2"),
            ),
        ]
    }
}

#[test]
fn test_daemon_default_sentinel_handled_like_auto() {
    for body in [
        "",
        "[pipeline]\nwarmup_frames = 0\n",
        "[pipeline]\ncamera_device = \"auto\"\n",
        "[pipeline]\ncamera_device = \"default\"\n",
        "[pipeline]\ncamera_device = \"\"\n",
    ] {
        let config = DaemonConfig::from_toml_str(body).unwrap();
        assert_eq!(
            config.pipeline.camera.device_path,
            PathBuf::from(AUTO_CAMERA_DEVICE),
            "config {body:?} must keep the auto sentinel"
        );
        let camera = resolve_pipeline_camera(&config.pipeline, &FakeEnumerator::default());
        assert_eq!(
            camera.device_path,
            PathBuf::from("/dev/v4l/by-id/usb-Cam_IR-video-index0"),
            "config {body:?} must auto-select the IR node like soos-enroll and soos-gui"
        );
    }
}

#[test]
fn test_daemon_resolution_matches_shared_resolver() {
    let config =
        DaemonConfig::from_toml_str("[pipeline]\nsensor_preference = \"prefer_rgb\"\n").unwrap();
    let enumerator = FakeEnumerator::default();
    let camera = resolve_pipeline_camera(&config.pipeline, &enumerator);
    let shared = resolve_camera_device(None, SensorPreference::PreferRgb, &enumerator);
    assert_eq!(camera.device_path, shared.path);
}

#[test]
fn test_daemon_explicit_device_honored_verbatim() {
    let config =
        DaemonConfig::from_toml_str("[pipeline]\ncamera_device = \"/dev/video42\"\n").unwrap();
    let camera = resolve_pipeline_camera(&config.pipeline, &FakeEnumerator::default());
    assert_eq!(camera.device_path, PathBuf::from("/dev/video42"));
}

#[test]
fn test_daemon_mock_camera_skips_enumeration() {
    let mut config = DaemonConfig::default();
    config.pipeline.use_mock_camera = true;
    let enumerator = FakeEnumerator::default();
    let camera = resolve_pipeline_camera(&config.pipeline, &enumerator);
    assert_eq!(enumerator.calls.get(), 0, "mock camera must not enumerate");
    assert_eq!(camera.device_path, PathBuf::from(AUTO_CAMERA_DEVICE));
}

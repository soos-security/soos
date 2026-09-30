//! Contractual tests for the single shared camera resolver (GitHub #152, review finding CAM-04).
//!
//! Contract:
//! - `soos-daemon`, `soos-enroll` and `soos-gui` resolve the camera through
//!   `soos_camera_v4l::resolve_camera_device`, with identical sentinel handling
//!   (`""`, `"auto"`, `"default"` and `/dev/v4l/by-id/default-camera` all mean "auto-detect").
//! - An explicit device path is honored verbatim.
//! - Auto-detection only ever considers enumerated V4L2 *capture* nodes, ranks them with
//!   `sensor_preference` (default `PreferIr`) and reports the stable `/dev/v4l/by-id/` alias of
//!   the selected node when one exists (Criterion C4). A by-id alias pointing to a metadata node
//!   (`...-video-index1`) is never selected.
//! - With no capture node at all the resolver returns the auto sentinel.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions and unwrap"
)]

use soos_camera_v4l::{
    is_auto_camera_device, parse_sensor_preference, resolve_camera_device, CameraConfigBuilder,
    CameraDeviceInfo, CameraEnumerator, CameraResolutionSource, PixelFormat, SensorPreference,
    SystemCameraEnumerator, AUTO_CAMERA_DEVICE,
};
use std::path::{Path, PathBuf};

/// Hermetic enumerator returning a fixed device and alias inventory.
struct FakeEnumerator {
    devices: Vec<CameraDeviceInfo>,
    aliases: Vec<(PathBuf, PathBuf)>,
}

impl CameraEnumerator for FakeEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        self.devices.clone()
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        self.aliases.clone()
    }
}

fn rgb(path: &str) -> CameraDeviceInfo {
    CameraDeviceInfo {
        path: PathBuf::from(path),
        card_name: "Integrated Camera: Integrated C".to_string(),
        supported_formats: vec![PixelFormat::Mjpeg, PixelFormat::Yuyv],
    }
}

fn ir(path: &str) -> CameraDeviceInfo {
    CameraDeviceInfo {
        path: PathBuf::from(path),
        card_name: "Integrated Camera: Integrated I".to_string(),
        supported_formats: vec![PixelFormat::Grey],
    }
}

fn alias(link: &str, target: &str) -> (PathBuf, PathBuf) {
    (PathBuf::from(link), PathBuf::from(target))
}

/// Typical Windows-Hello laptop: RGB on video0 (+ metadata video1), IR on video2 (+ metadata video3).
fn dual_sensor_laptop() -> FakeEnumerator {
    FakeEnumerator {
        devices: vec![rgb("/dev/video0"), ir("/dev/video2")],
        aliases: vec![
            alias("/dev/v4l/by-id/usb-Cam_RGB-video-index0", "/dev/video0"),
            alias("/dev/v4l/by-id/usb-Cam_RGB-video-index1", "/dev/video1"),
            alias("/dev/v4l/by-id/usb-Cam_IR-video-index0", "/dev/video2"),
            alias("/dev/v4l/by-id/usb-Cam_IR-video-index1", "/dev/video3"),
        ],
    }
}

#[test]
fn test_default_sentinel_handled_like_auto() {
    for sentinel in [
        "",
        "auto",
        "default",
        "AUTO",
        " Default ",
        AUTO_CAMERA_DEVICE,
    ] {
        assert!(
            is_auto_camera_device(Path::new(sentinel)),
            "'{sentinel}' must be treated as the auto-detection sentinel"
        );
    }
    for explicit in [
        "/dev/video0",
        "/dev/v4l/by-id/usb-Cam_IR-video-index0",
        "autocam",
    ] {
        assert!(
            !is_auto_camera_device(Path::new(explicit)),
            "'{explicit}' is an explicit device, not a sentinel"
        );
    }

    let enumerator = dual_sensor_laptop();
    let baseline = resolve_camera_device(None, SensorPreference::PreferIr, &enumerator);
    for sentinel in ["", "auto", "default", AUTO_CAMERA_DEVICE] {
        let resolved = resolve_camera_device(
            Some(Path::new(sentinel)),
            SensorPreference::PreferIr,
            &enumerator,
        );
        assert_eq!(
            resolved, baseline,
            "sentinel '{sentinel}' must resolve exactly like an absent camera_device"
        );
    }
}

#[test]
fn test_no_config_prefers_ir_by_id_alias() {
    let resolved = resolve_camera_device(None, SensorPreference::default(), &dual_sensor_laptop());
    assert_eq!(
        resolved.path,
        PathBuf::from("/dev/v4l/by-id/usb-Cam_IR-video-index0"),
        "Default resolution must pick the IR capture node through its stable by-id alias"
    );
    assert_eq!(resolved.source, CameraResolutionSource::AutoDetected);
}

#[test]
fn test_prefer_rgb_selects_rgb_by_id_alias() {
    let resolved = resolve_camera_device(None, SensorPreference::PreferRgb, &dual_sensor_laptop());
    assert_eq!(
        resolved.path,
        PathBuf::from("/dev/v4l/by-id/usb-Cam_RGB-video-index0")
    );
}

#[test]
fn test_by_id_metadata_index1_never_selected() {
    // The metadata alias sorts first alphabetically; the old CLI resolver would have returned it.
    let enumerator = FakeEnumerator {
        devices: vec![rgb("/dev/video2")],
        aliases: vec![
            alias("/dev/v4l/by-id/usb-AAA_Meta-video-index1", "/dev/video1"),
            alias("/dev/v4l/by-id/usb-ZZZ_Cam-video-index0", "/dev/video2"),
        ],
    };
    for pref in [
        SensorPreference::PreferIr,
        SensorPreference::PreferRgb,
        SensorPreference::Any,
    ] {
        let resolved = resolve_camera_device(None, pref, &enumerator);
        assert_eq!(
            resolved.path,
            PathBuf::from("/dev/v4l/by-id/usb-ZZZ_Cam-video-index0"),
            "a by-id alias of a non-capture node must never be selected ({pref:?})"
        );
    }
}

#[test]
fn test_selected_node_without_alias_is_returned_as_is() {
    let enumerator = FakeEnumerator {
        devices: vec![ir("/dev/video4")],
        aliases: vec![],
    };
    let resolved = resolve_camera_device(None, SensorPreference::PreferIr, &enumerator);
    assert_eq!(resolved.path, PathBuf::from("/dev/video4"));
    assert_eq!(resolved.source, CameraResolutionSource::AutoDetected);
}

#[test]
fn test_explicit_path_honored_verbatim() {
    let explicit = Path::new("/dev/video42");
    let resolved = resolve_camera_device(
        Some(explicit),
        SensorPreference::PreferIr,
        &dual_sensor_laptop(),
    );
    assert_eq!(resolved.path, explicit.to_path_buf());
    assert_eq!(resolved.source, CameraResolutionSource::Explicit);
}

#[test]
fn test_no_capture_device_falls_back_to_sentinel() {
    let enumerator = FakeEnumerator {
        devices: vec![],
        aliases: vec![alias("/dev/v4l/by-id/usb-Meta-video-index1", "/dev/video1")],
    };
    let resolved = resolve_camera_device(None, SensorPreference::PreferIr, &enumerator);
    assert_eq!(resolved.path, PathBuf::from(AUTO_CAMERA_DEVICE));
    assert_eq!(resolved.source, CameraResolutionSource::Fallback);
}

#[test]
fn test_parse_sensor_preference_shared_vocabulary() {
    assert_eq!(
        parse_sensor_preference("prefer_ir"),
        Some(SensorPreference::PreferIr)
    );
    assert_eq!(
        parse_sensor_preference("IR"),
        Some(SensorPreference::PreferIr)
    );
    assert_eq!(
        parse_sensor_preference("prefer_rgb"),
        Some(SensorPreference::PreferRgb)
    );
    assert_eq!(
        parse_sensor_preference(" rgb "),
        Some(SensorPreference::PreferRgb)
    );
    assert_eq!(parse_sensor_preference("any"), Some(SensorPreference::Any));
    assert_eq!(parse_sensor_preference("bogus"), None);
}

#[test]
fn test_camera_config_explicit_device_ignores_sentinel() {
    let default_cfg = CameraConfigBuilder::new().build();
    assert_eq!(default_cfg.explicit_device(), None);

    let auto_cfg = CameraConfigBuilder::new().device_path("default").build();
    assert_eq!(auto_cfg.explicit_device(), None);

    let explicit_cfg = CameraConfigBuilder::new()
        .device_path("/dev/video7")
        .build();
    assert_eq!(
        explicit_cfg.explicit_device(),
        Some(Path::new("/dev/video7"))
    );
}

#[test]
fn test_system_enumerator_canonicalizes_by_id_aliases() {
    let tmp = tempfile::tempdir().unwrap();
    let node = tmp.path().join("video0");
    std::fs::write(&node, b"").unwrap();
    let by_id = tmp.path().join("by-id");
    std::fs::create_dir(&by_id).unwrap();
    std::os::unix::fs::symlink("../video0", by_id.join("usb-Cam-video-index0")).unwrap();
    // A dangling alias must be skipped, never reported.
    std::os::unix::fs::symlink("../video9", by_id.join("usb-Gone-video-index0")).unwrap();

    let enumerator = SystemCameraEnumerator::with_by_id_dir(&by_id);
    let aliases = enumerator.by_id_aliases();
    assert_eq!(aliases.len(), 1, "dangling aliases must be skipped");
    assert_eq!(aliases[0].0, by_id.join("usb-Cam-video-index0"));
    assert_eq!(aliases[0].1, node.canonicalize().unwrap());

    let missing = SystemCameraEnumerator::with_by_id_dir(tmp.path().join("absent"));
    assert!(missing.by_id_aliases().is_empty());
}

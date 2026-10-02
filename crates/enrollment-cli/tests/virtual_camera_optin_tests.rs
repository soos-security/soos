//! Contract tests for GitHub #318 (row VCO3): `soos-enroll` honours `[pipeline]
//! allow_virtual_camera` of `daemon.toml`, read through the shared reader in the same call that
//! resolves the camera device, and opens the camera with `CameraConfig::allow_virtual_device`
//! set from it. Absent, mistyped or unusable configuration keeps virtual nodes refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use std::path::{Path, PathBuf};

use soos_camera_v4l::{CameraDeviceInfo, CameraEnumerator, PixelFormat};
use soos_enrollment_cli::service::{
    enrollment_camera_config, resolve_camera_device_from_config_reported, CameraDeviceChoice,
};

struct FakeEnumerator;

impl CameraEnumerator for FakeEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        vec![CameraDeviceInfo {
            path: PathBuf::from("/dev/video0"),
            card_name: "Integrated Camera".to_string(),
            supported_formats: vec![PixelFormat::Yuyv],
        }]
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        Vec::new()
    }
}

fn resolve(dir: &Path, body: &str) -> CameraDeviceChoice {
    let path = dir.join("daemon.toml");
    std::fs::write(&path, body).unwrap();
    resolve_camera_device_from_config_reported(None, Some(&path), &FakeEnumerator)
}

/// VCO3: `allow_virtual_camera = true` is reported by the camera resolution and reaches the
/// camera configuration `soos-enroll` opens.
#[test]
fn test_vco_enroll_honours_allow_virtual_camera() {
    let tmp = tempfile::tempdir().unwrap();
    let choice = resolve(
        tmp.path(),
        "[pipeline]\ncamera_device = \"/dev/video9\"\nallow_virtual_camera = true\n",
    );
    assert_eq!(choice.path, PathBuf::from("/dev/video9"));
    assert!(choice.allow_virtual_camera, "the opt-in must be read");
    assert!(choice.notes.is_empty(), "{:?}", choice.notes);

    let config = enrollment_camera_config(choice.path.clone(), choice.allow_virtual_camera);
    assert!(config.allow_virtual_device);
    assert_eq!(config.device_path, PathBuf::from("/dev/video9"));
}

/// VCO3: absent, `false`, mistyped and unusable configurations, and no configuration path,
/// keep virtual nodes refused.
#[test]
fn test_vco_enroll_allow_virtual_camera_defaults_false() {
    let tmp = tempfile::tempdir().unwrap();
    for body in [
        "",
        "[pipeline]\n",
        "[pipeline]\nallow_virtual_camera = false\n",
        "[pipeline]\nallow_virtual_camera = \"yes\"\n",
        "this is not toml = = =\n",
    ] {
        let choice = resolve(tmp.path(), body);
        assert!(!choice.allow_virtual_camera, "`{body}` must not opt in");
    }
    let mistyped = resolve(tmp.path(), "[pipeline]\nallow_virtual_camera = \"yes\"\n");
    assert!(
        mistyped
            .notes
            .iter()
            .any(|n| n.contains("allow_virtual_camera")),
        "a mistyped opt-in is reported by name: {:?}",
        mistyped.notes
    );

    let none = resolve_camera_device_from_config_reported(None, None, &FakeEnumerator);
    assert!(!none.allow_virtual_camera);

    let config = enrollment_camera_config(PathBuf::from("/dev/video0"), false);
    assert!(!config.allow_virtual_device);
}

/// VCO3: the production builder opens the camera through `enrollment_camera_config` with the
/// opt-in of its single configuration read (never a second read, never a hard-coded `false`).
#[test]
fn test_vco_enroll_full_service_uses_the_choice_opt_in() {
    let source = include_str!("../src/service.rs");
    let start = source
        .find("pub fn build_full_service_with_notes(")
        .expect("builder present");
    let body = &source[start..];
    let end = body.find("\n}\n").expect("builder end");
    let body = &body[..end];
    assert!(
        body.contains("enrollment_camera_config(device_path, choice.allow_virtual_camera)"),
        "build_full_service_with_notes must open the camera with the choice's opt-in"
    );
    assert!(
        !body.contains("read_daemon_camera_config"),
        "the opt-in must come from the single camera resolution read"
    );
}

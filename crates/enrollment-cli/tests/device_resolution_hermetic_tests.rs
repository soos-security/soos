//! Hermetic fixture-table contract for `soos-enroll` / `soos-gui` camera resolution
//! (GitHub #198 / CAM-16, #197 / CAM-15).
//!
//! `device_resolution_tests.rs` runs against the host enumerator and can only assert that the
//! result lives under `/dev`, which any camera (or none) satisfies. These tests pin the exact
//! device chosen for each configuration against injected inventories, so a wrong-camera
//! regression fails in CI on a host without any camera.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite uses assertions and unwrap"
)]

use soos_camera_v4l::{CameraDeviceInfo, CameraEnumerator, PixelFormat, AUTO_CAMERA_DEVICE};
use soos_enrollment_cli::service::resolve_camera_device_from_config_with;
use std::path::{Path, PathBuf};

const RGB_NODE: &str = "/dev/video0";
const META_NODE: &str = "/dev/video1";
const IR_NODE: &str = "/dev/video2";
const RGB_ALIAS: &str = "/dev/v4l/by-id/usb-Vendor_Cam-video-index0";
const META_ALIAS: &str = "/dev/v4l/by-id/usb-Vendor_Cam-video-index1";
const IR_ALIAS: &str = "/dev/v4l/by-id/usb-Vendor_Cam_IR-video-index0";

/// Injectable inventory: capture nodes plus by-id aliases.
#[derive(Clone, Default)]
struct Fixture {
    devices: Vec<CameraDeviceInfo>,
    aliases: Vec<(PathBuf, PathBuf)>,
}

impl CameraEnumerator for Fixture {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        self.devices.clone()
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        self.aliases.clone()
    }
}

fn rgb() -> CameraDeviceInfo {
    CameraDeviceInfo {
        path: PathBuf::from(RGB_NODE),
        card_name: "Integrated Camera: Integrated C".to_string(),
        supported_formats: vec![PixelFormat::Mjpeg, PixelFormat::Yuyv],
    }
}

fn ir() -> CameraDeviceInfo {
    CameraDeviceInfo {
        path: PathBuf::from(IR_NODE),
        card_name: "Integrated Camera: Integrated I".to_string(),
        supported_formats: vec![PixelFormat::Grey],
    }
}

fn aliases() -> Vec<(PathBuf, PathBuf)> {
    vec![
        (PathBuf::from(RGB_ALIAS), PathBuf::from(RGB_NODE)),
        (PathBuf::from(META_ALIAS), PathBuf::from(META_NODE)),
        (PathBuf::from(IR_ALIAS), PathBuf::from(IR_NODE)),
    ]
}

/// RGB + IR laptop camera, both with by-id aliases (and a metadata alias listed first).
fn dual_sensor() -> Fixture {
    Fixture {
        devices: vec![rgb(), ir()],
        aliases: aliases(),
    }
}

fn write_config(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path
}

/// Matrix CHT4: `camera_device = "default"` in `daemon.toml` is an auto-detection sentinel and
/// resolves to the exact IR capture node alias (the hermetic form of the historical
/// `test_resolve_camera_device_default_resolution` cited by walkthrough 75).
#[test]
fn test_resolve_camera_device_default_resolution() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = write_config(
        tmp.path(),
        "daemon.toml",
        "[pipeline]\ncamera_device = \"default\"\n",
    );
    let resolved = resolve_camera_device_from_config_with(None, Some(&cfg), &dual_sensor());
    assert_eq!(resolved, PathBuf::from(IR_ALIAS));
}

/// Matrix CHT4: fixture table covering every precedence and fallback branch.
#[test]
fn test_resolve_camera_device_fixture_table() {
    struct Case {
        name: &'static str,
        cli: Option<&'static str>,
        config: Option<&'static str>,
        fixture: Fixture,
        expected: &'static str,
    }

    let rgb_only = Fixture {
        devices: vec![rgb()],
        aliases: aliases(),
    };
    let ir_no_alias = Fixture {
        devices: vec![rgb(), ir()],
        aliases: vec![(PathBuf::from(RGB_ALIAS), PathBuf::from(RGB_NODE))],
    };

    let cases = vec![
        Case {
            name: "no config, dual sensor -> IR alias",
            cli: None,
            config: None,
            fixture: dual_sensor(),
            expected: IR_ALIAS,
        },
        Case {
            name: "auto + prefer_rgb -> RGB alias",
            cli: None,
            config: Some(
                "[pipeline]\ncamera_device = \"auto\"\nsensor_preference = \"prefer_rgb\"\n",
            ),
            fixture: dual_sensor(),
            expected: RGB_ALIAS,
        },
        Case {
            name: "short vocabulary rgb -> RGB alias",
            cli: None,
            config: Some("[pipeline]\nsensor_preference = \"rgb\"\n"),
            fixture: dual_sensor(),
            expected: RGB_ALIAS,
        },
        Case {
            name: "unknown sensor_preference keeps the IR default",
            cli: None,
            config: Some("[pipeline]\nsensor_preference = \"thermal\"\n"),
            fixture: dual_sensor(),
            expected: IR_ALIAS,
        },
        Case {
            name: "empty camera_device is a sentinel",
            cli: None,
            config: Some("[pipeline]\ncamera_device = \"\"\n"),
            fixture: dual_sensor(),
            expected: IR_ALIAS,
        },
        Case {
            name: "RGB-only host with IR preference falls back to RGB",
            cli: None,
            config: None,
            fixture: rgb_only,
            expected: RGB_ALIAS,
        },
        Case {
            name: "IR node without by-id alias -> /dev/videoN",
            cli: None,
            config: None,
            fixture: ir_no_alias,
            expected: IR_NODE,
        },
        Case {
            name: "no capture node -> auto sentinel",
            cli: None,
            config: None,
            fixture: Fixture::default(),
            expected: AUTO_CAMERA_DEVICE,
        },
        Case {
            name: "explicit config path is verbatim, even when absent from the inventory",
            cli: None,
            config: Some("[pipeline]\ncamera_device = \"/dev/video9\"\n"),
            fixture: dual_sensor(),
            expected: "/dev/video9",
        },
        Case {
            name: "explicit CLI path beats explicit config path",
            cli: Some("/dev/video5"),
            config: Some("[pipeline]\ncamera_device = \"/dev/video9\"\n"),
            fixture: dual_sensor(),
            expected: "/dev/video5",
        },
        Case {
            name: "CLI sentinel defers to explicit config path",
            cli: Some("auto"),
            config: Some("[pipeline]\ncamera_device = \"/dev/video9\"\n"),
            fixture: dual_sensor(),
            expected: "/dev/video9",
        },
        Case {
            name: "malformed daemon.toml behaves like no config",
            cli: None,
            config: Some("[pipeline\ncamera_device = /dev/video9\n"),
            fixture: dual_sensor(),
            expected: IR_ALIAS,
        },
    ];

    let tmp = tempfile::tempdir().unwrap();
    for (idx, case) in cases.into_iter().enumerate() {
        let cfg = case
            .config
            .map(|body| write_config(tmp.path(), &format!("daemon-{idx}.toml"), body));
        let resolved = resolve_camera_device_from_config_with(
            case.cli.map(PathBuf::from),
            cfg.as_deref(),
            &case.fixture,
        );
        assert_eq!(
            resolved,
            PathBuf::from(case.expected),
            "fixture case '{}' selected the wrong device",
            case.name
        );
    }
}

/// Matrix CHT4: the metadata node alias (`...-video-index1`) is never returned, even when it is
/// the only alias sorted first and the capture inventory is empty of aliases otherwise.
#[test]
fn test_resolve_camera_device_never_returns_metadata_alias() {
    let fixture = Fixture {
        devices: vec![ir()],
        aliases: vec![(PathBuf::from(META_ALIAS), PathBuf::from(META_NODE))],
    };
    let resolved = resolve_camera_device_from_config_with(None, None, &fixture);
    assert_eq!(resolved, PathBuf::from(IR_NODE));
}

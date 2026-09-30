//! Contract for `soos-admin camera list|probe` (GitHub #256 / CAM-17, matrix rows CDX7-CDX9).
//!
//! The reports are rendered from diagnostics collected through a fake V4L2 probe, so the exact
//! table and JSON output (the support "snapshot") is pinned without any camera on the host.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use clap::Parser;
use soos_admin_cli::args::{CameraAction, Cli, Commands};
use soos_admin_cli::camera::{
    collect_list_report, probe_report, CameraEnvironment, CameraListReport, CameraProbeReport,
};
use soos_camera_v4l::diagnostics::{
    collect_camera_diagnostics, probe_camera_node, ProbeFailure, V4lDeviceProbe, V4lNodeDetails,
    V4L2_CAP_DEVICE_CAPS, V4L2_CAP_EXT_PIX_FORMAT, V4L2_CAP_META_CAPTURE, V4L2_CAP_STREAMING,
    V4L2_CAP_VIDEO_CAPTURE,
};
use soos_camera_v4l::SensorPreference;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEV: &str = "/dev/soos-hermetic";
const CAPTURE_CAPS: u32 =
    V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_EXT_PIX_FORMAT | V4L2_CAP_STREAMING | V4L2_CAP_DEVICE_CAPS;
const META_CAPS: u32 =
    V4L2_CAP_META_CAPTURE | V4L2_CAP_EXT_PIX_FORMAT | V4L2_CAP_STREAMING | V4L2_CAP_DEVICE_CAPS;

#[derive(Default)]
struct FakeProbe {
    table: BTreeMap<PathBuf, Result<V4lNodeDetails, ProbeFailure>>,
}

impl FakeProbe {
    fn node(
        mut self,
        path: &Path,
        card: &str,
        caps: u32,
        fourccs: &[&[u8; 4]],
        sizes: &[(u32, u32)],
    ) -> Self {
        self.table.insert(
            path.to_path_buf(),
            Ok(V4lNodeDetails {
                driver: "uvcvideo".to_string(),
                card_name: card.to_string(),
                bus_info: "usb-0000:00:14.0-8".to_string(),
                device_caps: caps,
                fourccs: fourccs.iter().map(|f| **f).collect(),
                frame_sizes: sizes.to_vec(),
            }),
        );
        self
    }

    fn failing(mut self, path: &Path, failure: ProbeFailure) -> Self {
        self.table.insert(path.to_path_buf(), Err(failure));
        self
    }
}

impl V4lDeviceProbe for FakeProbe {
    fn details(&self, dev_path: &Path) -> Result<V4lNodeDetails, ProbeFailure> {
        self.table
            .get(dev_path)
            .cloned()
            .unwrap_or(Err(ProbeFailure::NotFound))
    }
}

fn dev(name: &str) -> PathBuf {
    Path::new(DEV).join(name)
}

fn fake_sysfs(entries: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in entries {
        std::fs::create_dir(dir.path().join(name)).unwrap();
    }
    dir
}

fn laptop_probe() -> FakeProbe {
    FakeProbe::default()
        .node(
            &dev("video0"),
            "Integrated Camera: Integrated C",
            CAPTURE_CAPS,
            &[b"MJPG", b"YUYV"],
            &[(1280, 720), (640, 480)],
        )
        .node(
            &dev("video1"),
            "Integrated Camera: Integrated C",
            META_CAPS,
            &[b"UVCH"],
            &[],
        )
        .node(
            &dev("video2"),
            "Integrated Camera: Integrated I",
            CAPTURE_CAPS,
            &[b"Y16 "],
            &[(400, 400)],
        )
        .failing(&dev("video3"), ProbeFailure::PermissionDenied)
}

fn laptop_aliases() -> Vec<(PathBuf, PathBuf)> {
    vec![
        (
            PathBuf::from("/dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index0"),
            dev("video0"),
        ),
        (
            PathBuf::from("/dev/v4l/by-id/usb-Chicony_Integrated_IR_Camera-video-index0"),
            dev("video2"),
        ),
    ]
}

fn laptop_report(preference: SensorPreference) -> CameraListReport {
    let sysfs = fake_sysfs(&["video0", "video1", "video2", "video3"]);
    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &laptop_aliases(),
        &laptop_probe(),
        preference,
        None,
    );
    CameraListReport::from_diagnostics(&diag)
}

// ---------------------------------------------------------------------------
// CDX7: command-line shape
// ---------------------------------------------------------------------------

#[test]
fn test_cdx_camera_list_args_parse() {
    let cli = Cli::try_parse_from([
        "soos-admin",
        "camera",
        "list",
        "--json",
        "--sensor-preference",
        "prefer_rgb",
        "--device",
        "/dev/video4",
    ])
    .unwrap();
    let Commands::Camera(args) = cli.command else {
        panic!("expected the camera subcommand");
    };
    let CameraAction::List(list) = args.action else {
        panic!("expected camera list");
    };
    assert!(list.json);
    assert_eq!(list.sensor_preference, SensorPreference::PreferRgb);
    assert_eq!(list.device, Some(PathBuf::from("/dev/video4")));

    let defaults = Cli::try_parse_from(["soos-admin", "camera", "list"]).unwrap();
    let Commands::Camera(args) = defaults.command else {
        panic!("expected the camera subcommand");
    };
    let CameraAction::List(list) = args.action else {
        panic!("expected camera list");
    };
    assert!(!list.json);
    assert_eq!(list.sensor_preference, SensorPreference::PreferIr);
    assert_eq!(list.device, None);

    for vocabulary in ["prefer_ir", "ir", "PREFER_RGB", "rgb", "any"] {
        assert!(
            Cli::try_parse_from([
                "soos-admin",
                "camera",
                "list",
                "--sensor-preference",
                vocabulary
            ])
            .is_ok(),
            "{vocabulary} is part of the daemon.toml vocabulary"
        );
    }
    assert!(Cli::try_parse_from([
        "soos-admin",
        "camera",
        "list",
        "--sensor-preference",
        "thermal"
    ])
    .is_err());
}

#[test]
fn test_cdx_camera_probe_args_parse() {
    let cli =
        Cli::try_parse_from(["soos-admin", "camera", "probe", "/dev/video2", "--json"]).unwrap();
    let Commands::Camera(args) = cli.command else {
        panic!("expected the camera subcommand");
    };
    let CameraAction::Probe(probe) = args.action else {
        panic!("expected camera probe");
    };
    assert_eq!(probe.device, PathBuf::from("/dev/video2"));
    assert!(probe.json);
    assert!(Cli::try_parse_from(["soos-admin", "camera", "probe"]).is_err());
    assert!(Cli::try_parse_from(["soos-admin", "camera"]).is_err());
}

// ---------------------------------------------------------------------------
// CDX8: snapshot of the list and probe reports
// ---------------------------------------------------------------------------

#[test]
fn test_cdx_camera_list_table_snapshot() {
    let expected = "\
Camera nodes: 4 scanned, 2 capture candidates

/dev/soos-hermetic/video0  [candidate]
  by-id:        /dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index0
  driver:       uvcvideo
  card:         Integrated Camera: Integrated C
  bus_info:     usb-0000:00:14.0-8
  device_caps:  0x84200001 (VIDEO_CAPTURE, EXT_PIX_FORMAT, STREAMING, DEVICE_CAPS)
  fourccs:      MJPG, YUYV
  frame sizes:  1280x720, 640x480
  sensor:       rgb (colour_formats)

/dev/soos-hermetic/video1  [not_video_capture]
  by-id:        -
  driver:       uvcvideo
  card:         Integrated Camera: Integrated C
  bus_info:     usb-0000:00:14.0-8
  device_caps:  0x84a00000 (EXT_PIX_FORMAT, META_CAPTURE, STREAMING, DEVICE_CAPS)
  fourccs:      UVCH
  frame sizes:  -

/dev/soos-hermetic/video2  [candidate]
  by-id:        /dev/v4l/by-id/usb-Chicony_Integrated_IR_Camera-video-index0
  driver:       uvcvideo
  card:         Integrated Camera: Integrated I
  bus_info:     usb-0000:00:14.0-8
  device_caps:  0x84200001 (VIDEO_CAPTURE, EXT_PIX_FORMAT, STREAMING, DEVICE_CAPS)
  fourccs:      Y16
  frame sizes:  400x400
  sensor:       infrared (by_id_ir_token)

/dev/soos-hermetic/video3  [probe_failed: permission_denied]

Selection (sensor preference: prefer_ir)
  selected:     /dev/v4l/by-id/usb-Chicony_Integrated_IR_Camera-video-index0
  node:         /dev/soos-hermetic/video2
  sensor:       infrared
  source:       auto_detected
  reason:       preferred_sensor_matched
";
    assert_eq!(
        laptop_report(SensorPreference::PreferIr).format_table(),
        expected
    );
}

#[test]
fn test_cdx_camera_list_json_snapshot() {
    let report = laptop_report(SensorPreference::PreferRgb);
    let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    let expected = serde_json::json!({
        "preference": "prefer_rgb",
        "selected": "/dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index0",
        "selected_node": "/dev/soos-hermetic/video0",
        "sensor_type": "rgb",
        "source": "auto_detected",
        "reason": "preferred_sensor_matched",
        "nodes": [
            {
                "node": "/dev/soos-hermetic/video0",
                "status": "candidate",
                "probe_error": null,
                "by_id": "/dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index0",
                "driver": "uvcvideo",
                "card": "Integrated Camera: Integrated C",
                "bus_info": "usb-0000:00:14.0-8",
                "device_caps": "0x84200001",
                "device_caps_flags": ["VIDEO_CAPTURE", "EXT_PIX_FORMAT", "STREAMING", "DEVICE_CAPS"],
                "fourccs": ["MJPG", "YUYV"],
                "formats": ["MJPG", "YUYV"],
                "frame_sizes": ["1280x720", "640x480"],
                "sensor_type": "rgb",
                "classification_reason": "colour_formats",
                "by_id_hint_ignored": false
            },
            {
                "node": "/dev/soos-hermetic/video1",
                "status": "not_video_capture",
                "probe_error": null,
                "by_id": null,
                "driver": "uvcvideo",
                "card": "Integrated Camera: Integrated C",
                "bus_info": "usb-0000:00:14.0-8",
                "device_caps": "0x84a00000",
                "device_caps_flags": ["EXT_PIX_FORMAT", "META_CAPTURE", "STREAMING", "DEVICE_CAPS"],
                "fourccs": ["UVCH"],
                "formats": [],
                "frame_sizes": [],
                "sensor_type": null,
                "classification_reason": null,
                "by_id_hint_ignored": false
            },
            {
                "node": "/dev/soos-hermetic/video2",
                "status": "candidate",
                "probe_error": null,
                "by_id": "/dev/v4l/by-id/usb-Chicony_Integrated_IR_Camera-video-index0",
                "driver": "uvcvideo",
                "card": "Integrated Camera: Integrated I",
                "bus_info": "usb-0000:00:14.0-8",
                "device_caps": "0x84200001",
                "device_caps_flags": ["VIDEO_CAPTURE", "EXT_PIX_FORMAT", "STREAMING", "DEVICE_CAPS"],
                "fourccs": ["Y16"],
                "formats": ["GREY"],
                "frame_sizes": ["400x400"],
                "sensor_type": "infrared",
                "classification_reason": "by_id_ir_token",
                "by_id_hint_ignored": false
            },
            {
                "node": "/dev/soos-hermetic/video3",
                "status": "probe_failed",
                "probe_error": "permission_denied",
                "by_id": null,
                "driver": null,
                "card": null,
                "bus_info": null,
                "device_caps": null,
                "device_caps_flags": [],
                "fourccs": [],
                "formats": [],
                "frame_sizes": [],
                "sensor_type": null,
                "classification_reason": null,
                "by_id_hint_ignored": false
            }
        ]
    });
    assert_eq!(value, expected);
}

#[test]
fn test_cdx_camera_probe_report_snapshot() {
    let probe = laptop_probe();
    let aliases = laptop_aliases();

    let ok = CameraProbeReport::from_node(&probe_camera_node(&dev("video2"), &aliases, &probe));
    let expected = "\
/dev/soos-hermetic/video2  [candidate]
  by-id:        /dev/v4l/by-id/usb-Chicony_Integrated_IR_Camera-video-index0
  driver:       uvcvideo
  card:         Integrated Camera: Integrated I
  bus_info:     usb-0000:00:14.0-8
  device_caps:  0x84200001 (VIDEO_CAPTURE, EXT_PIX_FORMAT, STREAMING, DEVICE_CAPS)
  fourccs:      Y16
  frame sizes:  400x400
  sensor:       infrared (by_id_ir_token)
";
    assert_eq!(ok.format_table(), expected);
    assert_eq!(ok.exit_code(), 0);
    let json: serde_json::Value = serde_json::from_str(&ok.to_json()).unwrap();
    assert_eq!(json["status"], "candidate");
    assert_eq!(json["sensor_type"], "infrared");
    assert_eq!(json["formats"], serde_json::json!(["GREY"]));

    let denied = CameraProbeReport::from_node(&probe_camera_node(&dev("video3"), &aliases, &probe));
    assert_eq!(
        denied.format_table(),
        "/dev/soos-hermetic/video3  [probe_failed: permission_denied]\n"
    );
    assert_eq!(denied.exit_code(), 1);
    let json: serde_json::Value = serde_json::from_str(&denied.to_json()).unwrap();
    assert_eq!(json["probe_error"], "permission_denied");

    let meta = CameraProbeReport::from_node(&probe_camera_node(&dev("video1"), &aliases, &probe));
    assert_eq!(meta.exit_code(), 1, "a metadata node is not usable by soos");
}

// ---------------------------------------------------------------------------
// CDX9: exit status, shared-stem note, sanitizing, environment wiring
// ---------------------------------------------------------------------------

#[test]
fn test_cdx_camera_list_exit_code_reflects_selection() {
    assert_eq!(laptop_report(SensorPreference::PreferIr).exit_code(), 0);

    let sysfs = fake_sysfs(&["video0"]);
    let probe = FakeProbe::default().failing(&dev("video0"), ProbeFailure::Busy);
    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &[],
        &probe,
        SensorPreference::PreferIr,
        None,
    );
    let report = CameraListReport::from_diagnostics(&diag);
    assert_eq!(report.exit_code(), 1, "no capture node means exit status 1");
    let table = report.format_table();
    assert!(table.contains("[probe_failed: busy]"), "{table}");
    assert!(table.contains("selected:     none"), "{table}");
    assert!(table.contains("reason:       no_capture_node"), "{table}");

    let explicit = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &[],
        &probe,
        SensorPreference::PreferIr,
        Some(Path::new("/dev/video7")),
    );
    let report = CameraListReport::from_diagnostics(&explicit);
    assert_eq!(
        report.exit_code(),
        0,
        "an explicit device is always a decision"
    );
    let table = report.format_table();
    assert!(table.contains("selected:     /dev/video7"), "{table}");
    assert!(table.contains("reason:       explicit_device"), "{table}");
}

#[test]
fn test_cdx_camera_list_reports_ignored_shared_by_id_token() {
    let sysfs = fake_sysfs(&["video0", "video2"]);
    let probe = FakeProbe::default()
        .node(
            &dev("video0"),
            "Acme HD Camera: Acme HD Camera",
            CAPTURE_CAPS,
            &[b"MJPG", b"YUYV"],
            &[(1280, 720)],
        )
        .node(
            &dev("video2"),
            "Acme HD Camera: Acme HD Camera",
            CAPTURE_CAPS,
            &[b"GREY"],
            &[(640, 480)],
        );
    let aliases = vec![
        (
            PathBuf::from("/dev/v4l/by-id/usb-Acme_HD_IR_Camera_0001-video-index0"),
            dev("video0"),
        ),
        (
            PathBuf::from("/dev/v4l/by-id/usb-Acme_HD_IR_Camera_0001-video-index2"),
            dev("video2"),
        ),
    ];
    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &aliases,
        &probe,
        SensorPreference::PreferIr,
        None,
    );
    let report = CameraListReport::from_diagnostics(&diag);
    let table = report.format_table();
    assert_eq!(
        table
            .matches(
                "  note:         by-id IR token ignored (stem shared by several capture nodes)"
            )
            .count(),
        2,
        "{table}"
    );
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(json["nodes"][0]["by_id_hint_ignored"], true);
    assert_eq!(
        json["selected"],
        "/dev/v4l/by-id/usb-Acme_HD_IR_Camera_0001-video-index2"
    );
}

#[test]
fn test_cdx_camera_output_sanitizes_control_characters() {
    let sysfs = fake_sysfs(&["video0"]);
    let probe = FakeProbe::default().node(
        &dev("video0"),
        "Cam\u{1b}]0;owned\u{7}",
        CAPTURE_CAPS,
        &[b"YUYV"],
        &[(640, 480)],
    );
    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &[(
            PathBuf::from("/dev/v4l/by-id/evil\u{1b}[2Jname"),
            dev("video0"),
        )],
        &probe,
        SensorPreference::PreferIr,
        None,
    );
    let table = CameraListReport::from_diagnostics(&diag).format_table();
    assert!(
        !table.chars().any(|c| c.is_control() && c != '\n'),
        "no control character may reach the terminal: {table:?}"
    );
}

/// The system environment wiring reads aliases through the shared bounded by-id scanner.
#[test]
fn test_cdx_camera_environment_uses_injected_directories() {
    let root = tempfile::tempdir().unwrap();
    // Canonical root so that the alias targets equal the probed node paths.
    let base = std::fs::canonicalize(root.path()).unwrap();
    let sysfs = base.join("sysfs");
    let devdir = base.join("dev");
    let by_id = base.join("by-id");
    for dir in [&sysfs, &devdir, &by_id] {
        std::fs::create_dir(dir).unwrap();
    }
    std::fs::create_dir(sysfs.join("video0")).unwrap();
    std::fs::write(devdir.join("video0"), b"").unwrap();
    std::os::unix::fs::symlink(
        devdir.join("video0"),
        by_id.join("usb-Vendor_IR_Cam-video-index0"),
    )
    .unwrap();

    let env = CameraEnvironment {
        sysfs_dir: sysfs.clone(),
        dev_dir: devdir.clone(),
        by_id_dir: by_id.clone(),
    };
    let probe = FakeProbe::default().node(
        &devdir.join("video0"),
        "Cam",
        CAPTURE_CAPS,
        &[b"YUYV"],
        &[(1280, 720)],
    );
    let list = collect_list_report(&env, &probe, SensorPreference::PreferIr, None);
    let json: serde_json::Value = serde_json::from_str(&list.to_json()).unwrap();
    assert_eq!(
        json["selected"],
        by_id
            .join("usb-Vendor_IR_Cam-video-index0")
            .display()
            .to_string()
    );
    assert_eq!(json["nodes"][0]["classification_reason"], "by_id_ir_token");

    let single = probe_report(&env, &probe, &by_id.join("usb-Vendor_IR_Cam-video-index0"));
    assert_eq!(
        single.exit_code(),
        0,
        "a by-id link is probed through its target node"
    );

    let default_env = CameraEnvironment::default();
    assert_eq!(
        default_env.sysfs_dir,
        PathBuf::from("/sys/class/video4linux")
    );
    assert_eq!(default_env.dev_dir, PathBuf::from("/dev"));
    assert_eq!(default_env.by_id_dir, PathBuf::from("/dev/v4l/by-id"));
}

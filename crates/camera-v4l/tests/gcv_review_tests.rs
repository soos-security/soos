//! Camera findings of the 2026-10-02 review (GitHub #307 CAM-NEW-3, #314 CAM-NEW-4; matrix
//! rows GCV4, GCV5, GCV6, GCV14).
//!
//! - Virtual capture nodes (v4l2loopback, vivid) and nodes that accept frames from user space
//!   (`VIDEO_OUTPUT*`, `VIDEO_M2M*`) are never auto-selected, and the diagnostics say why;
//! - a NUL byte in a device path is an error, never a `v4l` panic.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use soos_camera_v4l::daemon_config::{read_daemon_camera_config, DaemonConfigKey};
use soos_camera_v4l::diagnostics::{
    collect_camera_diagnostics, NodeStatus, ProbeFailure, V4lDeviceProbe, V4lNodeDetails,
};
use soos_camera_v4l::{
    enumerate_capture_devices_with, resolve_camera_device, virtual_node_rejection,
    CameraConfigBuilder, CameraDeviceInfo, CameraEnumerator, CameraHealth, CameraManager,
    CameraStatus, PixelFormat, SensorPreference, SystemV4lNodeProbe, V4lCameraManager,
    V4lNodeCapabilities, V4lNodeProbe, VirtualNodeRejection,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
const CAP_VIDEO_OUTPUT: u32 = 0x0000_0002;
const CAP_VIDEO_OUTPUT_MPLANE: u32 = 0x0000_2000;
const CAP_VIDEO_M2M: u32 = 0x0000_8000;
const CAP_STREAMING: u32 = 0x0400_0000;
const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
const UVC_CAPS: u32 = CAP_VIDEO_CAPTURE | CAP_STREAMING | CAP_DEVICE_CAPS;

#[derive(Default)]
struct FakeProbe {
    table: BTreeMap<PathBuf, V4lNodeCapabilities>,
}

impl FakeProbe {
    fn with(mut self, path: &str, driver: &str, caps: u32, formats: &[PixelFormat]) -> Self {
        self.table.insert(
            PathBuf::from(path),
            V4lNodeCapabilities {
                card_name: format!("{driver} card"),
                video_capture: caps & CAP_VIDEO_CAPTURE != 0,
                supported_formats: formats.to_vec(),
                driver: driver.to_string(),
                device_caps: caps,
            },
        );
        self
    }
}

impl V4lNodeProbe for FakeProbe {
    fn probe(&self, dev_path: &Path) -> Option<V4lNodeCapabilities> {
        self.table.get(dev_path).cloned()
    }
}

fn fake_sysfs(entries: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in entries {
        std::fs::create_dir(dir.path().join(name)).unwrap();
    }
    dir
}

struct Inventory(Vec<CameraDeviceInfo>);

impl CameraEnumerator for Inventory {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        self.0.clone()
    }
    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        Vec::new()
    }
}

/// GCV4: the rejection rule names the reason; real webcams pass.
#[test]
fn test_gcv_virtual_node_rejection_rule() {
    assert_eq!(virtual_node_rejection("uvcvideo", UVC_CAPS), None);
    assert_eq!(
        virtual_node_rejection("v4l2 loopback", UVC_CAPS),
        Some(VirtualNodeRejection::VirtualDriver)
    );
    assert_eq!(
        virtual_node_rejection("V4L2LOOPBACK", UVC_CAPS),
        Some(VirtualNodeRejection::VirtualDriver)
    );
    assert_eq!(
        virtual_node_rejection("vivid", UVC_CAPS),
        Some(VirtualNodeRejection::VirtualDriver)
    );
    assert_eq!(
        virtual_node_rejection("uvcvideo", UVC_CAPS | CAP_VIDEO_OUTPUT),
        Some(VirtualNodeRejection::OutputCapable)
    );
    assert_eq!(
        virtual_node_rejection("uvcvideo", UVC_CAPS | CAP_VIDEO_OUTPUT_MPLANE),
        Some(VirtualNodeRejection::OutputCapable)
    );
    assert_eq!(
        virtual_node_rejection("some-codec", CAP_VIDEO_M2M | CAP_STREAMING),
        Some(VirtualNodeRejection::MemToMem)
    );
    for reason in [
        VirtualNodeRejection::VirtualDriver,
        VirtualNodeRejection::OutputCapable,
        VirtualNodeRejection::MemToMem,
    ] {
        assert!(reason.as_str().starts_with("rejected_"));
        assert!(!reason.description().is_empty());
    }
}

/// GCV4: a GREY-only loopback node (classified Infrared) is never chosen under `PreferIr`;
/// the uvc RGB webcam is.
#[test]
fn test_gcv_loopback_grey_node_is_not_enumerated_and_uvc_is_chosen() {
    let sysfs = fake_sysfs(&["video0", "video1", "video2"]);
    let probe = FakeProbe::default()
        .with(
            "/dev/video0",
            "v4l2 loopback",
            UVC_CAPS,
            &[PixelFormat::Grey],
        )
        .with(
            "/dev/video1",
            "uvcvideo",
            UVC_CAPS,
            &[PixelFormat::Yuyv, PixelFormat::Mjpeg],
        )
        .with("/dev/video2", "vivid", UVC_CAPS, &[PixelFormat::Grey]);
    let devices = enumerate_capture_devices_with(sysfs.path(), Path::new("/dev"), &probe);
    let paths: Vec<_> = devices.iter().map(|d| d.path.clone()).collect();
    assert_eq!(paths, vec![PathBuf::from("/dev/video1")]);

    let resolution = resolve_camera_device(None, SensorPreference::PreferIr, &Inventory(devices));
    assert_eq!(resolution.path, PathBuf::from("/dev/video1"));
}

struct FakeDetails(BTreeMap<PathBuf, V4lNodeDetails>);

impl V4lDeviceProbe for FakeDetails {
    fn details(&self, dev_path: &Path) -> Result<V4lNodeDetails, ProbeFailure> {
        self.0.get(dev_path).cloned().ok_or(ProbeFailure::NotFound)
    }
}

fn details(driver: &str, caps: u32, fourcc: &[u8; 4]) -> V4lNodeDetails {
    V4lNodeDetails {
        driver: driver.to_string(),
        card_name: format!("{driver} card"),
        bus_info: "platform:test".to_string(),
        device_caps: caps,
        fourccs: vec![*fourcc],
        frame_sizes: vec![(640, 480)],
    }
}

/// GCV5: `soos-admin camera list` reports the loopback node as rejected, with the reason, and
/// selects the uvc webcam.
#[test]
fn test_gcv_diagnostics_report_virtual_node_rejection() {
    let sysfs = fake_sysfs(&["video0", "video1", "video2"]);
    let mut table = BTreeMap::new();
    table.insert(
        PathBuf::from("/dev/video0"),
        details("v4l2 loopback", UVC_CAPS, b"GREY"),
    );
    table.insert(
        PathBuf::from("/dev/video1"),
        details("uvcvideo", UVC_CAPS, b"YUYV"),
    );
    table.insert(
        PathBuf::from("/dev/video2"),
        details("uvcvideo", UVC_CAPS | CAP_VIDEO_OUTPUT, b"YUYV"),
    );
    let report = collect_camera_diagnostics(
        sysfs.path(),
        Path::new("/dev"),
        &[],
        &FakeDetails(table),
        SensorPreference::PreferIr,
        None,
    );
    assert_eq!(
        report.nodes[0].status,
        NodeStatus::Rejected(VirtualNodeRejection::VirtualDriver)
    );
    assert_eq!(report.nodes[0].status.as_str(), "rejected_virtual_driver");
    assert_eq!(report.nodes[1].status, NodeStatus::Candidate);
    assert_eq!(
        report.nodes[2].status,
        NodeStatus::Rejected(VirtualNodeRejection::OutputCapable)
    );
    assert_eq!(report.selected_node, Some(PathBuf::from("/dev/video1")));
    assert_eq!(report.resolution.path, PathBuf::from("/dev/video1"));
}

/// GCV6: `[pipeline] allow_virtual_camera` is the documented opt-in; absent or mistyped, it
/// stays off (fail closed).
#[test]
fn test_gcv_allow_virtual_camera_opt_in_is_read_and_defaults_off() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, body: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, body).unwrap();
        path
    };
    let off = write("off.toml", "[pipeline]\ncamera_device = \"/dev/video4\"\n");
    assert!(
        !read_daemon_camera_config(&off)
            .unwrap()
            .allow_virtual_camera
    );
    let on = write("on.toml", "[pipeline]\nallow_virtual_camera = true\n");
    assert!(read_daemon_camera_config(&on).unwrap().allow_virtual_camera);
    let bad = write("bad.toml", "[pipeline]\nallow_virtual_camera = \"yes\"\n");
    let config = read_daemon_camera_config(&bad).unwrap();
    assert!(!config.allow_virtual_camera);
    assert_eq!(
        config.mistyped_keys,
        vec![DaemonConfigKey::AllowVirtualCamera]
    );
    assert_eq!(
        DaemonConfigKey::AllowVirtualCamera.name(),
        "allow_virtual_camera"
    );

    let default = CameraConfigBuilder::new().build();
    assert!(
        !default.allow_virtual_device,
        "virtual devices are refused by default"
    );
    assert!(
        CameraConfigBuilder::new()
            .allow_virtual_device(true)
            .build()
            .allow_virtual_device
    );
}

/// GCV14: a NUL byte in `camera_device` is refused at config load (key ignored, auto-detection
/// applies) instead of reaching `v4l::Device::with_path`.
#[test]
fn test_gcv_nul_byte_camera_device_is_refused_at_config_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.toml");
    std::fs::write(
        &path,
        "[pipeline]\ncamera_device = \"/dev/vid\\u0000eo0\"\n",
    )
    .unwrap();
    let config = read_daemon_camera_config(&path).unwrap();
    assert_eq!(config.settings.camera_device, None);
    assert_eq!(config.mistyped_keys, vec![DaemonConfigKey::CameraDevice]);
}

/// GCV14: the system probe and the capture supervisor turn a NUL path into an error.
#[test]
fn test_gcv_nul_byte_device_path_never_panics() {
    let nul = Path::new("/dev/vid\0eo0");
    let probed = std::panic::catch_unwind(|| SystemV4lNodeProbe.probe(nul));
    assert!(matches!(probed, Ok(None)), "the probe must skip a NUL path");

    let config = CameraConfigBuilder::new()
        .device_path(nul)
        .backoff_limits(Duration::from_millis(20), Duration::from_millis(40))
        .build();
    let manager = V4lCameraManager::spawn(config).expect("spawn");
    let start = Instant::now();
    let mut status = manager.status();
    while start.elapsed() < Duration::from_secs(3) {
        status = manager.status();
        if matches!(status, CameraStatus::Error { .. }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        matches!(status, CameraStatus::Error { .. }),
        "a NUL path is a recoverable open error: {status:?}"
    );
    assert!(
        manager.health() != CameraHealth::Dead,
        "the supervisor must not die: {:?}",
        manager.health()
    );
    manager.stop();
}

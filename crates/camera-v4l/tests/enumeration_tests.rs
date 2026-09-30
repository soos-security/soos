//! Hermetic contract for V4L2 capture-node enumeration (GitHub #197 / CAM-15, #198 / CAM-16).
//!
//! `enumerate_capture_devices_with(sysfs_dir, dev_dir, probe)` is the injectable core of
//! `enumerate_capture_devices()`: the sysfs directory listing is a temporary directory and the
//! per-node V4L2 capability query is a fake [`V4lNodeProbe`], so the filtering, ordering and
//! bounds are proven without any camera on the host.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use soos_camera_v4l::{
    enumerate_capture_devices_with, resolve_camera_device, CameraDeviceInfo, CameraEnumerator,
    CameraResolutionSource, PixelFormat, SensorPreference, SensorType, V4lNodeCapabilities,
    V4lNodeProbe, MAX_VIDEO_NODES,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Fake capability probe backed by a fixture table keyed by `/dev/<node>` path.
#[derive(Default)]
struct FakeProbe {
    table: BTreeMap<PathBuf, V4lNodeCapabilities>,
    probed: RefCell<Vec<PathBuf>>,
}

impl FakeProbe {
    fn with(mut self, dev_path: &str, card: &str, capture: bool, formats: &[PixelFormat]) -> Self {
        self.table.insert(
            PathBuf::from(dev_path),
            V4lNodeCapabilities {
                card_name: card.to_string(),
                video_capture: capture,
                supported_formats: formats.to_vec(),
            },
        );
        self
    }
}

impl V4lNodeProbe for FakeProbe {
    fn probe(&self, dev_path: &Path) -> Option<V4lNodeCapabilities> {
        self.probed.borrow_mut().push(dev_path.to_path_buf());
        self.table.get(dev_path).cloned()
    }
}

/// Creates a fake `/sys/class/video4linux` directory holding the given entry names.
fn fake_sysfs(entries: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in entries {
        std::fs::create_dir(dir.path().join(name)).unwrap();
    }
    dir
}

fn paths(devices: &[CameraDeviceInfo]) -> Vec<String> {
    devices
        .iter()
        .map(|d| d.path.display().to_string())
        .collect()
}

/// Matrix CHT1: a capture node exposing no decodable pixel format (for example the uvcvideo
/// metadata node or a node whose formats are all unknown FourCCs) is never listed.
#[test]
fn test_enumerate_filters_empty_format_nodes() {
    let sysfs = fake_sysfs(&["video0", "video1", "video2"]);
    let probe = FakeProbe::default()
        .with("/dev/video0", "RGB Cam", true, &[PixelFormat::Yuyv])
        .with("/dev/video1", "RGB Cam", true, &[])
        .with("/dev/video2", "IR Cam: IR", true, &[PixelFormat::Grey]);

    let devices = enumerate_capture_devices_with(sysfs.path(), Path::new("/dev"), &probe);
    assert_eq!(paths(&devices), vec!["/dev/video0", "/dev/video2"]);
    assert_eq!(devices[0].card_name, "RGB Cam");
    assert_eq!(devices[1].supported_formats, vec![PixelFormat::Grey]);
}

/// Matrix CHT1: nodes without `V4L2_CAP_VIDEO_CAPTURE` and nodes that cannot be opened or
/// queried (`EACCES`, `ENODEV`, vanished) are skipped without aborting the scan.
#[test]
fn test_enumerate_skips_non_capture_and_unprobeable_nodes() {
    let sysfs = fake_sysfs(&["video0", "video1", "video2", "video3"]);
    let probe = FakeProbe::default()
        .with("/dev/video0", "M2M codec", false, &[PixelFormat::Nv12])
        // video1 is absent from the table: the probe fails (open or VIDIOC_QUERYCAP error).
        .with(
            "/dev/video2",
            "Integrated Camera: Integrated C",
            true,
            &[PixelFormat::Mjpeg],
        )
        .with(
            "/dev/video3",
            "Integrated Camera: Integrated I",
            true,
            &[PixelFormat::Grey],
        );

    let devices = enumerate_capture_devices_with(sysfs.path(), Path::new("/dev"), &probe);
    assert_eq!(paths(&devices), vec!["/dev/video2", "/dev/video3"]);
    assert_eq!(devices[1].sensor_type(), SensorType::Infrared);
}

/// Matrix CHT1: nodes are probed and listed in numeric order (`video2` before `video10`),
/// independent of the directory listing order.
#[test]
fn test_enumerate_orders_nodes_numerically() {
    let sysfs = fake_sysfs(&["video10", "video2", "video1"]);
    let probe = FakeProbe::default()
        .with("/dev/video1", "A", true, &[PixelFormat::Yuyv])
        .with("/dev/video2", "B", true, &[PixelFormat::Yuyv])
        .with("/dev/video10", "C", true, &[PixelFormat::Yuyv]);

    let devices = enumerate_capture_devices_with(sysfs.path(), Path::new("/dev"), &probe);
    assert_eq!(
        paths(&devices),
        vec!["/dev/video1", "/dev/video2", "/dev/video10"]
    );
    assert_eq!(
        *probe.probed.borrow(),
        vec![
            PathBuf::from("/dev/video1"),
            PathBuf::from("/dev/video2"),
            PathBuf::from("/dev/video10")
        ]
    );
}

/// Matrix CHT1: only `video<decimal>` entries are device candidates; sub-devices, radio, VBI
/// and malformed names are never opened.
#[test]
fn test_enumerate_ignores_non_video_sysfs_entries() {
    let sysfs = fake_sysfs(&[
        "v4l-subdev0",
        "radio0",
        "vbi0",
        "video",
        "videoX",
        "video-1",
        "video+1",
        "video4",
    ]);
    let probe = FakeProbe::default().with("/dev/video4", "Cam", true, &[PixelFormat::Yuyv]);

    let devices = enumerate_capture_devices_with(sysfs.path(), Path::new("/dev"), &probe);
    assert_eq!(paths(&devices), vec!["/dev/video4"]);
    assert_eq!(*probe.probed.borrow(), vec![PathBuf::from("/dev/video4")]);
}

/// Matrix CHT2: the number of nodes opened by one scan is bounded by `MAX_VIDEO_NODES`.
#[test]
fn test_enumerate_bounds_probed_nodes() {
    let names: Vec<String> = (0..MAX_VIDEO_NODES + 16)
        .map(|i| format!("video{i}"))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let sysfs = fake_sysfs(&refs);
    let mut probe = FakeProbe::default();
    for name in &names {
        probe = probe.with(&format!("/dev/{name}"), "Cam", true, &[PixelFormat::Yuyv]);
    }

    let devices = enumerate_capture_devices_with(sysfs.path(), Path::new("/dev"), &probe);
    assert_eq!(devices.len(), MAX_VIDEO_NODES);
    assert_eq!(probe.probed.borrow().len(), MAX_VIDEO_NODES);
    assert_eq!(devices[0].path, PathBuf::from("/dev/video0"));
}

/// Matrix CHT2: a host without `/sys/class/video4linux` (container, no V4L2 driver) yields an
/// empty inventory, never a panic, and nothing is probed.
#[test]
fn test_enumerate_missing_sysfs_dir_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let probe = FakeProbe::default().with("/dev/video0", "Cam", true, &[PixelFormat::Yuyv]);
    let devices =
        enumerate_capture_devices_with(&tmp.path().join("absent"), Path::new("/dev"), &probe);
    assert!(devices.is_empty());
    assert!(probe.probed.borrow().is_empty());
}

/// Matrix CHT2: device paths are built under the injected `dev_dir`.
#[test]
fn test_enumerate_builds_paths_under_dev_dir() {
    let sysfs = fake_sysfs(&["video7"]);
    let probe = FakeProbe::default().with("/chroot/dev/video7", "Cam", true, &[PixelFormat::Grey]);
    let devices = enumerate_capture_devices_with(sysfs.path(), Path::new("/chroot/dev"), &probe);
    assert_eq!(paths(&devices), vec!["/chroot/dev/video7"]);
}

/// Enumerator combining the hermetic node scan with a fixed alias table.
struct ScanEnumerator {
    sysfs: PathBuf,
    probe: FakeProbe,
    aliases: Vec<(PathBuf, PathBuf)>,
}

impl CameraEnumerator for ScanEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        enumerate_capture_devices_with(&self.sysfs, Path::new("/dev/soos-hermetic"), &self.probe)
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        self.aliases.clone()
    }
}

/// Matrix CHT3: end to end (scan -> classify -> select -> alias), the IR capture node is
/// selected by default even when a metadata node and an RGB node precede it, and it is reported
/// through its by-id alias.
#[test]
fn test_scan_then_resolve_prefers_ir_capture_node_alias() {
    let sysfs = fake_sysfs(&["video0", "video1", "video2", "video3"]);
    let probe = FakeProbe::default()
        .with(
            "/dev/soos-hermetic/video0",
            "Integrated Camera: Integrated C",
            true,
            &[PixelFormat::Mjpeg, PixelFormat::Yuyv],
        )
        .with(
            "/dev/soos-hermetic/video1",
            "Integrated Camera: Integrated C",
            true,
            &[],
        )
        .with(
            "/dev/soos-hermetic/video2",
            "Integrated Camera: Integrated I",
            true,
            &[PixelFormat::Grey],
        )
        .with(
            "/dev/soos-hermetic/video3",
            "Integrated Camera: Integrated I",
            true,
            &[],
        );
    let enumerator = ScanEnumerator {
        sysfs: sysfs.path().to_path_buf(),
        probe,
        aliases: vec![
            (
                PathBuf::from("/dev/v4l/by-id/usb-Cam-video-index0"),
                PathBuf::from("/dev/soos-hermetic/video0"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-Cam-video-index1"),
                PathBuf::from("/dev/soos-hermetic/video1"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-Cam_IR-video-index0"),
                PathBuf::from("/dev/soos-hermetic/video2"),
            ),
        ],
    };

    let resolved = resolve_camera_device(None, SensorPreference::default(), &enumerator);
    assert_eq!(
        resolved.path,
        PathBuf::from("/dev/v4l/by-id/usb-Cam_IR-video-index0")
    );
    assert_eq!(resolved.source, CameraResolutionSource::AutoDetected);
    assert_eq!(resolved.sensor_type, Some(SensorType::Infrared));

    let rgb = resolve_camera_device(None, SensorPreference::PreferRgb, &enumerator);
    assert_eq!(
        rgb.path,
        PathBuf::from("/dev/v4l/by-id/usb-Cam-video-index0")
    );
}

/// Matrix CHT3: without a by-id alias the auto-selected `/dev/videoN` node itself is returned
/// (ADR 2026-09-30 "Hermetic V4L2 Enumeration and `/dev/videoN` Auto-Selection").
#[test]
fn test_scan_then_resolve_without_alias_returns_video_node() {
    let sysfs = fake_sysfs(&["video0", "video2"]);
    let probe = FakeProbe::default()
        .with(
            "/dev/soos-hermetic/video0",
            "RGB",
            true,
            &[PixelFormat::Yuyv],
        )
        .with(
            "/dev/soos-hermetic/video2",
            "Cam: IR",
            true,
            &[PixelFormat::Grey],
        );
    let enumerator = ScanEnumerator {
        sysfs: sysfs.path().to_path_buf(),
        probe,
        aliases: Vec::new(),
    };
    let resolved = resolve_camera_device(None, SensorPreference::PreferIr, &enumerator);
    assert_eq!(resolved.path, PathBuf::from("/dev/soos-hermetic/video2"));
    assert_eq!(resolved.source, CameraResolutionSource::AutoDetected);
}

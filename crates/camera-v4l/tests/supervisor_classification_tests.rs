//! Capture supervisor vs resolver classification for shared by-id stems (GitHub #287,
//! matrix row CVF5, ADR 2026-10-01 "Capture Supervisor Keeps the By-Id IR Token on Shared
//! Stems").
//!
//! The resolver withholds a by-id name whose stem another capture node shares, so a composite
//! module whose USB product string carries `IR` does not make `PreferIr` pick the RGB node.
//! The capture supervisor deliberately keeps that name when it classifies the opened node:
//! its hints are a superset of the resolver's, and the scorer is monotonic in the by-id hint,
//! so the supervisor can only stamp a node *Infrared* (stricter IR PAD policy) where the
//! resolver said RGB, never the reverse.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use soos_camera_v4l::{
    classify_sensor_with_hints, explain_camera_resolution, plan_capture_with_hints,
    supervisor_sensor_hints, CameraConfig, CameraDeviceInfo, CameraEnumerator, PixelFormat,
    SensorHints, SensorPreference, SensorType,
};
use std::path::{Path, PathBuf};

const SHARED_RGB: &str = "/dev/v4l/by-id/usb-Acme_HD_IR_Camera_0001-video-index0";
const SHARED_IR: &str = "/dev/v4l/by-id/usb-Acme_HD_IR_Camera_0001-video-index2";

struct Composite;

impl CameraEnumerator for Composite {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        vec![
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video0"),
                card_name: "Acme HD Camera: Acme HD Camera".to_string(),
                supported_formats: vec![PixelFormat::Mjpeg, PixelFormat::Yuyv],
            },
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video2"),
                card_name: "Acme HD Camera: Acme HD Camera".to_string(),
                supported_formats: vec![PixelFormat::Grey],
            },
        ]
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        vec![
            (PathBuf::from(SHARED_RGB), PathBuf::from("/dev/video0")),
            (PathBuf::from(SHARED_IR), PathBuf::from("/dev/video2")),
        ]
    }
}

#[test]
fn test_cvf_supervisor_keeps_shared_stem_ir_token_decisive() {
    // The resolver ignores the shared IR token and classifies the colour node RGB...
    let report = explain_camera_resolution(None, SensorPreference::PreferRgb, &Composite);
    let rgb_candidate = report
        .candidates
        .iter()
        .find(|c| c.device.path == Path::new("/dev/video0"))
        .unwrap();
    assert!(rgb_candidate.by_id_hint_ignored);
    assert_eq!(rgb_candidate.sensor_type, SensorType::Rgb);
    assert_eq!(report.resolution.path, PathBuf::from(SHARED_RGB));

    // ...while the supervisor, opening that by-id path, keeps the token: the node is stamped
    // Infrared and its frames take the stricter IR PAD policy (availability cost, never a
    // weaker liveness check).
    let hints = supervisor_sensor_hints(Path::new(SHARED_RGB), vec![(1280, 720)]);
    assert_eq!(
        hints.by_id_name.as_deref(),
        Some("usb-Acme_HD_IR_Camera_0001-video-index0")
    );
    assert_eq!(hints.frame_sizes, vec![(1280, 720)]);
    let plan = plan_capture_with_hints(
        &rgb_candidate.device.card_name,
        &rgb_candidate.device.supported_formats,
        &hints,
        &CameraConfig::default(),
    )
    .unwrap();
    assert_eq!(plan.sensor_type, SensorType::Infrared);

    // A /dev/videoN path carries no by-id name (no alias lookup in the open path).
    let plain = supervisor_sensor_hints(Path::new("/dev/video0"), Vec::new());
    assert_eq!(plain.by_id_name, None);
}

#[test]
fn test_cvf_by_id_hint_never_downgrades_infrared() {
    let cards = [
        "Acme HD Camera: Acme HD Camera",
        "Integrated Camera: Integrated I",
        "Integrated_IR_Camera",
        "USB2.0 FHD UVC WebCam",
    ];
    let format_sets: [&[PixelFormat]; 5] = [
        &[PixelFormat::Mjpeg, PixelFormat::Yuyv],
        &[PixelFormat::Grey],
        &[PixelFormat::Yuyv, PixelFormat::Grey],
        &[PixelFormat::Rgb24],
        &[],
    ];
    let size_sets: [&[(u32, u32)]; 3] = [&[], &[(400, 400)], &[(1280, 720), (640, 480)]];
    let by_id_names = [
        "usb-Acme_HD_IR_Camera_0001-video-index0",
        "usb-Vendor_Integrated_Camera-video-index0",
        "usb-Vendor_infrared_cam-video-index0",
    ];
    let mut checked = 0usize;
    for card in cards {
        for formats in format_sets {
            for sizes in size_sets {
                let resolver_hints = SensorHints {
                    by_id_name: None,
                    frame_sizes: sizes.to_vec(),
                };
                let without = classify_sensor_with_hints(card, formats, &resolver_hints);
                for name in by_id_names {
                    let path = Path::new("/dev/v4l/by-id").join(name);
                    let supervisor = supervisor_sensor_hints(&path, sizes.to_vec());
                    assert_eq!(supervisor.by_id_name.as_deref(), Some(name));
                    let with = classify_sensor_with_hints(card, formats, &supervisor);
                    if without == SensorType::Infrared {
                        assert_eq!(
                            with,
                            SensorType::Infrared,
                            "{card} {formats:?} {sizes:?} {name}: adding the by-id hint must \
                             never turn an Infrared node into {with:?}"
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 4 * 5 * 3 * 3);
}

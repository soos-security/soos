//! Opt-in hardware smoke tests (GitHub #198 / CAM-16).
//!
//! These tests exercise the real `/sys/class/video4linux` scan, the real V4L2 capability probe
//! and a real MMAP stream. They are `#[ignore]`d so CI (which has no camera) never runs them, and
//! they additionally return early unless `SOOS_HW_TESTS=1`, so an accidental `--ignored` run on a
//! machine without a camera does not fail. Run on a workstation with a camera:
//!
//! ```text
//! SOOS_HW_TESTS=1 cargo test -p soos-camera-v4l --test hardware_smoke_tests -- --ignored
//! ```
//!
//! Nothing captured is written or logged: only frame dimensions and counts are asserted.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Hardware smoke tests use direct assertions"
)]

use soos_camera_v4l::{
    enumerate_capture_devices, resolve_camera_device, CameraConfigBuilder, CameraManager,
    CameraResolutionSource, SensorPreference, SystemCameraEnumerator, V4lCameraManager,
};
use std::time::{Duration, Instant};

/// Environment switch enabling the hardware smoke tests.
const HW_TESTS_ENV: &str = "SOOS_HW_TESTS";

fn hardware_tests_enabled() -> bool {
    std::env::var(HW_TESTS_ENV).is_ok_and(|v| v == "1")
}

#[test]
#[ignore = "requires a V4L2 camera; run with SOOS_HW_TESTS=1 and --ignored"]
fn test_hw_enumeration_lists_a_capture_node_with_formats() {
    if !hardware_tests_enabled() {
        return;
    }
    let devices = enumerate_capture_devices();
    assert!(
        !devices.is_empty(),
        "no V4L2 capture node with a decodable format was found"
    );
    for device in &devices {
        assert!(device.path.starts_with("/dev"));
        assert!(!device.supported_formats.is_empty());
    }
}

#[test]
#[ignore = "requires a V4L2 camera; run with SOOS_HW_TESTS=1 and --ignored"]
fn test_hw_resolved_camera_streams_a_frame() {
    if !hardware_tests_enabled() {
        return;
    }
    let resolution = resolve_camera_device(
        None,
        SensorPreference::default(),
        &SystemCameraEnumerator::default(),
    );
    assert_eq!(
        resolution.source,
        CameraResolutionSource::AutoDetected,
        "auto-detection found no capture node"
    );

    let config = CameraConfigBuilder::new()
        .device_path(&resolution.path)
        .warmup_frames(0)
        .build();
    let camera = V4lCameraManager::spawn(config).expect("spawn V4L2 capture manager");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut frame = None;
    while Instant::now() < deadline {
        camera.notify_activity();
        frame = camera.latest_frame();
        if frame.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    camera.stop();

    let frame = frame.expect("no frame received from the resolved camera within 10 s");
    assert!(frame.width > 0 && frame.height > 0);
}

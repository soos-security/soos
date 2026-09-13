//! Tests for Criterion C1: mock-camera feature provides functional MockCameraManager.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Unit tests use assertions, unwrap, and indexing"
)]

use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat};
use std::thread;
use std::time::Duration;

#[test]
fn test_mock_camera_generates_frames_and_readiness() {
    let config = CameraConfigBuilder::new()
        .resolution(640, 480)
        .format(PixelFormat::Yuyv)
        .fps(30)
        .warmup_frames(5) // shortened for test
        .build();

    let camera = MockCameraManager::new(config);

    // Should not be ready initially until warmup frames elapse
    assert!(
        !camera.is_ready(),
        "Camera should not be ready before warmup"
    );

    // Wait for warmup to complete (approx 5 frames at 30 fps = ~166ms)
    thread::sleep(Duration::from_millis(250));

    assert!(camera.is_ready(), "Camera should be ready after warmup");

    let frame = camera.latest_frame();
    assert!(frame.is_some(), "Latest frame must be available once ready");

    let frame = frame.unwrap();
    assert_eq!(frame.width, 640);
    assert_eq!(frame.height, 480);
    assert_eq!(frame.format, PixelFormat::Yuyv);
    assert!(!frame.data.is_empty());
    assert!(frame.timestamp_mono_ns > 0);

    // Verify timestamps advance monotonically
    let first_ts = frame.timestamp_mono_ns;
    thread::sleep(Duration::from_millis(50));
    let next_frame = camera.latest_frame().expect("Next frame should exist");
    assert!(
        next_frame.timestamp_mono_ns >= first_ts,
        "Timestamps must be monotonically increasing"
    );

    camera.stop();
}

#[test]
fn test_mock_camera_respects_custom_resolution() {
    let config = CameraConfigBuilder::new()
        .resolution(320, 240)
        .format(PixelFormat::Grey)
        .warmup_frames(1)
        .build();

    let camera = MockCameraManager::new(config);
    thread::sleep(Duration::from_millis(100));

    assert!(camera.is_ready());
    let frame = camera.latest_frame().expect("Frame must exist");
    assert_eq!(frame.width, 320);
    assert_eq!(frame.height, 240);
    assert_eq!(frame.format, PixelFormat::Grey);
    assert_eq!(frame.data.len(), 320 * 240);

    camera.stop();
}

#[test]
fn test_mock_camera_simulates_starvation() {
    let config = CameraConfigBuilder::new().warmup_frames(1).build();
    let camera = MockCameraManager::new(config);
    thread::sleep(Duration::from_millis(100));
    assert!(camera.is_ready());

    // Trigger starvation
    camera.set_starved(true);
    thread::sleep(Duration::from_millis(50));

    // When starved, is_ready must fail-closed
    assert!(
        !camera.is_ready(),
        "Camera must report not ready when starved"
    );
    assert!(
        camera.latest_frame().is_none(),
        "Latest frame must be None when starved"
    );

    // Clear starvation
    camera.set_starved(false);
    thread::sleep(Duration::from_millis(100));
    assert!(camera.is_ready());
    assert!(camera.latest_frame().is_some());

    camera.stop();
}

#[test]
fn test_mock_camera_idle_throttling_and_wake() {
    let config = CameraConfigBuilder::new()
        .fps(30)
        .idle_fps(5)
        .idle_timeout(Duration::from_millis(100)) // short idle timeout
        .warmup_frames(1)
        .build();

    let camera = MockCameraManager::new(config);
    thread::sleep(Duration::from_millis(50));
    assert!(camera.is_ready());

    // Wait for idle timeout to expire
    thread::sleep(Duration::from_millis(150));

    // Notify activity should immediately wake back to full speed
    camera.notify_activity();
    let frame = camera.latest_frame();
    assert!(frame.is_some(), "Frame must remain accessible during wake");

    camera.stop();
}

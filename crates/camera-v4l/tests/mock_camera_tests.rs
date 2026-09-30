//! Tests for Criterion C1: mock-camera feature provides functional MockCameraManager.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Unit tests use assertions, unwrap, and indexing"
)]

mod common;

use common::{wait_until, SETTLE_TIMEOUT};
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
    wait_until(SETTLE_TIMEOUT, || camera.is_ready());

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
    wait_until(SETTLE_TIMEOUT, || {
        camera.is_ready() && camera.latest_frame().is_some()
    });

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
    wait_until(SETTLE_TIMEOUT, || camera.is_ready());
    assert!(camera.is_ready());

    // Trigger starvation
    camera.set_starved(true);
    // Minimum dwell so that a worker ignoring starvation would have republished a frame,
    // then a bounded wait for the worker to settle (GitHub #280).
    thread::sleep(Duration::from_millis(50));
    wait_until(SETTLE_TIMEOUT, || {
        !camera.is_ready() && camera.latest_frame().is_none()
    });

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
    wait_until(SETTLE_TIMEOUT, || {
        camera.is_ready() && camera.latest_frame().is_some()
    });
    assert!(camera.is_ready());
    assert!(camera.latest_frame().is_some());

    camera.stop();
}

#[test]
fn test_mock_camera_idle_throttling_and_wake() {
    let config = CameraConfigBuilder::new()
        .fps(30)
        .idle_fps(5)
        // Idle window long enough that a loaded host never idles before the first frame
        // (test timing only, user-approved 2026-09-30, GitHub #280).
        .idle_timeout(Duration::from_millis(1000))
        .warmup_frames(1)
        .build();

    let camera = MockCameraManager::new(config);
    wait_until(SETTLE_TIMEOUT, || camera.is_ready());
    assert!(camera.is_ready());

    // Wait for idle timeout to expire
    thread::sleep(Duration::from_millis(1500));
    // ... and for the worker to have actually entered the suspended state (it clears the
    // frame slot there), so the wake below never races a late suspension (GitHub #280).
    wait_until(SETTLE_TIMEOUT, || camera.latest_frame().is_none());

    // Notify activity should immediately wake back to full speed
    camera.notify_activity();
    let frame = camera.latest_frame();
    assert!(frame.is_some(), "Frame must remain accessible during wake");

    camera.stop();
}

#[test]
fn test_mock_camera_auto_suspend_and_resume_lifecycle() {
    let config = CameraConfigBuilder::new()
        .fps(30)
        .idle_fps(5)
        // Test timing only (user-approved 2026-09-30, GitHub #280).
        .idle_timeout(Duration::from_millis(1000))
        .warmup_frames(2)
        .build();

    let camera = MockCameraManager::new(config);

    // Wait for initial warmup
    wait_until(SETTLE_TIMEOUT, || camera.is_ready());
    assert!(
        camera.is_ready(),
        "Camera must become ready after initial warmup"
    );

    // Wait for idle_timeout to expire without activity
    thread::sleep(Duration::from_millis(1500));
    assert!(
        !camera.is_ready(),
        "Camera must transition to Suspended (!is_ready) after idle_timeout to turn off LED"
    );

    // Notify activity: camera must re-initialize on-demand and become ready
    camera.notify_activity();
    wait_until(SETTLE_TIMEOUT, || camera.is_ready());
    assert!(
        camera.is_ready(),
        "Camera must resume from Suspended state upon notify_activity()"
    );
    assert!(camera.latest_frame().is_some());

    camera.stop();
}

#[test]
fn test_mock_camera_idle_timeout_zero_disables_auto_standby() {
    let config = CameraConfigBuilder::new()
        .fps(30)
        .idle_timeout(Duration::ZERO)
        .warmup_frames(0)
        .build();

    let camera = MockCameraManager::new(config);

    // Wait for camera to be ready
    wait_until(SETTLE_TIMEOUT, || camera.is_ready());
    assert!(camera.is_ready(), "Camera must be ready");

    // Sleep for 100ms without calling notify_activity()
    thread::sleep(Duration::from_millis(100));

    // When idle_timeout is Duration::ZERO, auto-standby is disabled and camera NEVER suspends
    assert!(
        camera.is_ready(),
        "Camera must remain ready indefinitely when idle_timeout is Duration::ZERO"
    );
    assert!(
        camera.latest_frame().is_some(),
        "Camera must continue serving frames when auto-standby is disabled"
    );

    camera.stop();
}

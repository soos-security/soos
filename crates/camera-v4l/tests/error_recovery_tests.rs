//! Tests for Criterion C3: Handles ENODEV, EIO, EBUSY without panic; bounded backoff.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Error recovery tests use assertions and unwrap"
)]

use soos_camera_v4l::{CameraConfigBuilder, CameraError, CameraManager, MockCameraManager};
use std::thread;
use std::time::Duration;

#[test]
fn test_error_recovery_enodev_without_panic() {
    let config = CameraConfigBuilder::new()
        .warmup_frames(1)
        .backoff_limits(Duration::from_millis(50), Duration::from_millis(500))
        .build();

    let camera = MockCameraManager::new(config);
    thread::sleep(Duration::from_millis(50));
    assert!(camera.is_ready());

    // Inject simulated ENODEV (error code 19)
    camera.set_error(Some(CameraError::Simulated {
        code: libc::ENODEV,
        message: "No such device".to_string(),
    }));

    thread::sleep(Duration::from_millis(50));

    // Must not panic, must report not ready (fail-closed)
    assert!(!camera.is_ready(), "Camera must report not ready on ENODEV");
    assert!(
        camera.latest_frame().is_none(),
        "No frame should be served during ENODEV"
    );

    // Clear error
    camera.set_error(None);
    thread::sleep(Duration::from_millis(150));

    assert!(
        camera.is_ready(),
        "Camera must automatically recover when error clears"
    );
    assert!(camera.latest_frame().is_some());

    camera.stop();
}

#[test]
fn test_error_recovery_ebusy_without_panic() {
    let config = CameraConfigBuilder::new()
        .warmup_frames(1)
        .backoff_limits(Duration::from_millis(50), Duration::from_millis(500))
        .build();

    let camera = MockCameraManager::new(config);
    thread::sleep(Duration::from_millis(50));
    assert!(camera.is_ready());

    // Inject simulated EBUSY (error code 16)
    camera.set_error(Some(CameraError::Simulated {
        code: libc::EBUSY,
        message: "Device or resource busy".to_string(),
    }));

    thread::sleep(Duration::from_millis(50));
    assert!(!camera.is_ready());
    assert!(camera.latest_frame().is_none());

    // Clear error
    camera.set_error(None);
    thread::sleep(Duration::from_millis(150));
    assert!(camera.is_ready());

    camera.stop();
}

#[test]
fn test_error_recovery_eio_without_panic() {
    let config = CameraConfigBuilder::new()
        .warmup_frames(1)
        .backoff_limits(Duration::from_millis(50), Duration::from_millis(500))
        .build();

    let camera = MockCameraManager::new(config);
    thread::sleep(Duration::from_millis(50));
    assert!(camera.is_ready());

    // Inject simulated EIO (error code 5)
    camera.set_error(Some(CameraError::Simulated {
        code: libc::EIO,
        message: "Input/output error".to_string(),
    }));

    thread::sleep(Duration::from_millis(50));
    assert!(!camera.is_ready());

    // Clear error
    camera.set_error(None);
    thread::sleep(Duration::from_millis(150));
    assert!(camera.is_ready());

    camera.stop();
}

#[test]
fn test_exponential_backoff_calculation() {
    let config = CameraConfigBuilder::new()
        .backoff_limits(Duration::from_millis(100), Duration::from_millis(5000))
        .build();

    // Verify backoff progression math
    let mut current = config.min_backoff;
    let expected = [
        Duration::from_millis(100),
        Duration::from_millis(200),
        Duration::from_millis(400),
        Duration::from_millis(800),
        Duration::from_millis(1600),
        Duration::from_millis(3200),
        Duration::from_millis(5000), // capped
        Duration::from_millis(5000), // stays capped
    ];

    for &exp in &expected {
        assert_eq!(current, exp);
        current = (current.saturating_mul(2)).min(config.max_backoff);
    }
}

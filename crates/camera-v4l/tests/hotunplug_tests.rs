//! Tests for Sub-issue #22.3: Graceful camera hot-unplug handling.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_camera_v4l::{CameraConfigBuilder, CameraError, CameraManager, MockCameraManager};
use std::thread;
use std::time::Duration;

#[test]
fn test_camera_hotunplug_recovery() {
    let config = CameraConfigBuilder::new()
        .warmup_frames(2)
        .backoff_limits(Duration::from_millis(20), Duration::from_millis(200))
        .build();

    let camera = MockCameraManager::new(config);
    // Allow warmup convergence
    thread::sleep(Duration::from_millis(100));
    assert!(camera.is_ready(), "Camera should be ready initially");
    assert!(
        camera.latest_frame().is_some(),
        "Initial frame must be available"
    );

    // Trigger simulated ENODEV disconnection (kernel reports no such device on unplug)
    camera.set_error(Some(CameraError::Simulated {
        code: libc::ENODEV,
        message: "No such device: camera unplugged".to_string(),
    }));

    thread::sleep(Duration::from_millis(50));

    // Must report not ready and clear latest frame
    assert!(
        !camera.is_ready(),
        "Camera must report not ready immediately upon hot-unplug"
    );
    assert!(
        camera.latest_frame().is_none(),
        "No frames should be served while camera is unplugged"
    );

    // Simulate device re-plugging by clearing the error
    camera.set_error(None);

    // Wait for reconnection and warmup convergence
    thread::sleep(Duration::from_millis(200));

    assert!(
        camera.is_ready(),
        "Camera must transparently recover and stabilize after device reappears"
    );
    assert!(
        camera.latest_frame().is_some(),
        "Fresh frames must be available after recovery"
    );

    camera.stop();
}

//! Tests for Criterion C5: Drops first 15-30 frames after startup for auto-exposure.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Warmup tests use assertions"
)]

mod common;

use common::{wait_until, SETTLE_TIMEOUT};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager};
use std::thread;
use std::time::Duration;

#[test]
fn test_warmup_frames_discard_before_ready() {
    // Test with default 20 warmup frames (within 15-30 bounds)
    let config = CameraConfigBuilder::new()
        .warmup_frames(20)
        .fps(60) // 60 fps for test speed (approx 16ms/frame)
        .build();

    let camera = MockCameraManager::new(config);

    // Initial state: not ready
    assert!(
        !camera.is_ready(),
        "Must not be ready immediately after spawn"
    );
    assert!(
        camera.latest_frame().is_none(),
        "Must not serve un-stabilized frames during warmup"
    );

    // Wait until halfway through warmup (~10 frames at 60 fps = ~160ms)
    thread::sleep(Duration::from_millis(150));
    assert!(!camera.is_ready(), "Must still not be ready mid-warmup");

    // Wait for warmup to finish (total 20 frames = ~330ms)
    wait_until(SETTLE_TIMEOUT, || {
        camera.is_ready() && camera.latest_frame().is_some()
    });
    assert!(camera.is_ready(), "Must be ready after 20 warmup frames");
    assert!(
        camera.latest_frame().is_some(),
        "Must serve frame once stabilized"
    );

    camera.stop();
}

#[test]
fn test_warmup_frames_configurable_bounds() {
    for warmup_count in [15, 20, 25, 30] {
        let config = CameraConfigBuilder::new()
            .warmup_frames(warmup_count)
            .fps(100)
            .build();

        let camera = MockCameraManager::new(config);
        assert!(!camera.is_ready());

        // Wait for warmup_count frames
        wait_until(SETTLE_TIMEOUT, || camera.is_ready());

        assert!(
            camera.is_ready(),
            "Camera must be ready after {} warmup frames",
            warmup_count
        );
        camera.stop();
    }
}

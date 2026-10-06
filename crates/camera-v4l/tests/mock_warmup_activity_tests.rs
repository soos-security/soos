//! Mock camera warmup vs `notify_activity` (Criterion C5).
//!
//! `soos-gui --mock` calls `notify_activity` for every analyzed frame. During warmup that call
//! used to publish a synthetic frame and the ready flag, which the warmup thread withdrew on its
//! next tick: the status flapped between `Ready` and `Starting` until warmup ended. Activity
//! during warmup must refresh the idle clock only; frames and readiness come from warmup alone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions"
)]

mod common;

use common::{wait_until, SETTLE_TIMEOUT};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, CameraStatus, MockCameraManager};
use std::thread;
use std::time::Duration;

#[test]
fn test_notify_activity_during_warmup_never_publishes_ready() {
    // 30 warmup frames at 30 fps: about one second of warmup.
    let config = CameraConfigBuilder::new().warmup_frames(30).fps(30).build();
    let camera = MockCameraManager::new(config);

    for _ in 0..10 {
        camera.notify_activity();
        assert!(
            !camera.is_ready(),
            "activity during warmup must not mark the camera ready"
        );
        assert!(
            camera.latest_frame().is_none(),
            "activity during warmup must not serve an un-stabilized frame"
        );
        assert_eq!(camera.status(), CameraStatus::Starting);
        thread::sleep(Duration::from_millis(20));
    }

    camera.stop();
}

#[test]
fn test_notify_activity_after_warmup_still_publishes_fresh_frame() {
    let config = CameraConfigBuilder::new().warmup_frames(2).fps(60).build();
    let camera = MockCameraManager::new(config);

    wait_until(SETTLE_TIMEOUT, || camera.is_ready());
    let before = camera.latest_frame().map(|f| f.sequence);
    camera.notify_activity();
    let after = camera.latest_frame().map(|f| f.sequence);

    assert!(camera.is_ready(), "a ready camera stays ready on activity");
    assert!(
        after.is_some() && after > before,
        "activity after warmup still restores a fresh frame immediately"
    );

    camera.stop();
}

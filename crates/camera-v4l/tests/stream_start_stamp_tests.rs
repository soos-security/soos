//! Contract tests of GitHub #331 for `CameraManager::stream_started_mono_ns` (spec
//! `AI/architect_spec_install_gdm_followups.md` §3.1 and §3.3, matrix IGF8).
//!
//! - The trait method is defaulted to `None`, so every existing implementor keeps compiling
//!   and keeps today's behaviour.
//! - `MockCameraManager::set_stream_started_mono_ns` is a test hook (0 = `None`) returned
//!   regardless of readiness; the default is `None`.
//!
//! The V4L supervisor stamp is covered in `src/v4l_impl/supervisor_tests.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::sync::Arc;

use soos_camera_v4l::{CameraManager, Frame, MockCameraManager};

/// A minimal implementor that only provides the required methods.
struct MinimalCamera;

impl CameraManager for MinimalCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        None
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn notify_activity(&self) {}
    fn stop(&self) {}
}

/// IGF8: the defaulted trait method returns `None` for an implementor that does not override it.
#[test]
fn test_igf_default_stream_started_mono_ns_is_none() {
    assert_eq!(MinimalCamera.stream_started_mono_ns(), None);
    let dynamic: &dyn CameraManager = &MinimalCamera;
    assert_eq!(dynamic.stream_started_mono_ns(), None);
}

/// IGF8: the mock camera reports no stream start until a test sets one.
#[test]
fn test_igf_mock_stream_started_defaults_to_none() {
    let camera = MockCameraManager::new_default();
    assert_eq!(camera.stream_started_mono_ns(), None);
    camera.stop();
}

/// IGF8: the mock setter round-trips, `Some(0)` reads as `None`, `None` clears it, and the
/// value is returned regardless of readiness.
#[test]
fn test_igf_mock_stream_started_setter_round_trips_regardless_of_readiness() {
    let camera = MockCameraManager::new_default();
    camera.set_stream_started_mono_ns(Some(42_000_000));
    assert_eq!(camera.stream_started_mono_ns(), Some(42_000_000));

    camera.set_ready(false);
    assert!(!camera.is_ready());
    assert_eq!(
        camera.stream_started_mono_ns(),
        Some(42_000_000),
        "the mock hook ignores readiness"
    );
    camera.set_starved(true);
    assert_eq!(camera.stream_started_mono_ns(), Some(42_000_000));

    camera.set_stream_started_mono_ns(Some(u64::MAX));
    assert_eq!(camera.stream_started_mono_ns(), Some(u64::MAX));

    camera.set_stream_started_mono_ns(Some(0));
    assert_eq!(camera.stream_started_mono_ns(), None, "0 means unknown");

    camera.set_stream_started_mono_ns(Some(7));
    camera.set_stream_started_mono_ns(None);
    assert_eq!(camera.stream_started_mono_ns(), None);

    let dynamic: Arc<dyn CameraManager> = Arc::new(MockCameraManager::new_default());
    assert_eq!(dynamic.stream_started_mono_ns(), None);
    dynamic.stop();
    camera.stop();
}

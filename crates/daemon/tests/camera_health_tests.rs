//! Contract tests for live camera health reporting (GitHub #153) and startup device
//! planning without silent substitution (GitHub #151).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use soos_camera_v4l::{
    CameraConfigBuilder, CameraError, CameraHealth, CameraManager, MockCameraManager,
    SensorPreference, V4lCameraManager,
};
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::{
    camera_device_resolver, plan_camera_device, AUTO_SELECT_DEVICE_SENTINEL,
};

fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    cond()
}

fn fully_started_health() -> HealthState {
    let health = HealthState::new();
    health.set_socket_ready(true);
    health.set_models_verified(true);
    health
}

#[test]
fn test_camera_ready_false_when_device_missing() {
    let dir = tempfile::tempdir().unwrap();
    let config = CameraConfigBuilder::new()
        .device_path(dir.path().join("video99"))
        .backoff_limits(Duration::from_millis(20), Duration::from_millis(40))
        .build();
    let camera: Arc<dyn CameraManager> = Arc::new(V4lCameraManager::spawn(config).unwrap());

    let health = fully_started_health();
    // Pre-fix startup behavior: the flag was forced to true right after spawn.
    health.set_camera_ready(true);
    health.attach_camera(Arc::clone(&camera));

    assert!(wait_until(Duration::from_secs(2), || camera.health()
        == CameraHealth::Recovering));
    let status = health.snapshot();
    assert!(
        !status.camera_ready,
        "camera_ready must be false while the device is missing"
    );
    assert!(!status.is_healthy);
}

#[test]
fn test_camera_ready_true_when_mock_streaming() {
    let camera = Arc::new(MockCameraManager::new(
        CameraConfigBuilder::new().warmup_frames(0).build(),
    ));
    let health = fully_started_health();
    health.attach_camera(camera.clone());

    assert!(wait_until(Duration::from_secs(2), || health
        .snapshot()
        .camera_ready));
    assert!(health.snapshot().is_healthy);
    assert_eq!(health.camera_health(), Some(CameraHealth::Streaming));
}

#[test]
fn test_camera_ready_follows_failure_and_recovery() {
    let camera = Arc::new(MockCameraManager::new(
        CameraConfigBuilder::new().warmup_frames(0).build(),
    ));
    let health = fully_started_health();
    health.attach_camera(camera.clone());
    assert!(wait_until(Duration::from_secs(2), || health
        .snapshot()
        .camera_ready));

    camera.set_error(Some(CameraError::Simulated {
        code: libc::EBUSY,
        message: "Device or resource busy".to_string(),
    }));
    assert!(!health.snapshot().camera_ready);
    assert!(!health.snapshot().is_healthy);

    camera.set_error(None);
    assert!(wait_until(Duration::from_secs(2), || health
        .snapshot()
        .camera_ready));
}

#[test]
fn test_camera_ready_stays_true_during_auto_standby() {
    let camera = Arc::new(MockCameraManager::new(
        CameraConfigBuilder::new()
            .warmup_frames(0)
            .idle_timeout(Duration::from_millis(100))
            .build(),
    ));
    let health = fully_started_health();
    health.attach_camera(camera.clone());

    assert!(wait_until(Duration::from_secs(2), || camera.health()
        == CameraHealth::Standby));
    assert!(!camera.is_ready());
    assert!(
        health.snapshot().camera_ready,
        "An idle daemon in auto-standby is healthy, not failed"
    );
}

#[test]
fn test_camera_ready_without_attached_camera_uses_flag() {
    let health = fully_started_health();
    assert!(!health.snapshot().camera_ready);
    assert_eq!(health.camera_health(), None);
    health.set_camera_ready(true);
    assert!(health.snapshot().camera_ready);
}

#[test]
fn test_pipeline_init_keeps_configured_by_id_when_missing_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    let configured = dir.path().join("by-id").join("usb-Vendor_IR-video-index0");
    let config = CameraConfigBuilder::new()
        .device_path(&configured)
        .backoff_limits(Duration::from_millis(20), Duration::from_millis(40))
        .build();

    // Another camera is present, but the configured by-id link has not appeared yet.
    let planned = plan_camera_device(&config, |_| Some(PathBuf::from("/dev/video7")));
    assert_eq!(
        planned.device_path, configured,
        "A configured device must never be silently replaced by a different camera"
    );
    assert!(
        camera_device_resolver(&planned).is_none(),
        "A configured device is retried as-is, never re-resolved to another camera"
    );

    // The supervisor keeps retrying the configured path and reports the camera as not ready.
    let camera = V4lCameraManager::spawn(planned).unwrap();
    assert!(wait_until(Duration::from_secs(2), || camera.health()
        == CameraHealth::Recovering));
    thread::sleep(Duration::from_millis(150));
    assert_eq!(camera.current_device_path(), configured);
}

#[test]
fn test_pipeline_auto_select_uses_sentinel_and_resolver() {
    let config = CameraConfigBuilder::new()
        .device_path(AUTO_SELECT_DEVICE_SENTINEL)
        .sensor_preference(SensorPreference::PreferIr)
        .build();

    let planned = plan_camera_device(&config, |pref| {
        assert_eq!(pref, SensorPreference::PreferIr);
        Some(PathBuf::from("/dev/v4l/by-id/usb-Vendor_IR-video-index0"))
    });
    assert_eq!(
        planned.device_path,
        PathBuf::from("/dev/v4l/by-id/usb-Vendor_IR-video-index0")
    );

    // Nothing detected at boot: keep the sentinel, the supervisor re-resolves later.
    let none = plan_camera_device(&config, |_| None);
    assert_eq!(none.device_path, PathBuf::from(AUTO_SELECT_DEVICE_SENTINEL));

    assert!(
        camera_device_resolver(&config).is_some(),
        "Auto-selection mode must re-enumerate on device loss"
    );
}

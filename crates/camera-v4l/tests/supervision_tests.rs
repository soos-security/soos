//! Contract tests for capture supervision (GitHub #151, #153): device re-resolution after
//! ENODEV, truthful lifecycle health (standby vs failure) and fail-closed supervisor panics.
//!
//! Hardware-free: every V4L2 path used here does not exist, so the production supervisor
//! deterministically hits `DeviceNotFound` and exercises its recovery loop.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests use assertions, unwrap, and expect"
)]

use soos_camera_v4l::{
    stable_device_path, CameraConfig, CameraConfigBuilder, CameraError, CameraHealth,
    CameraManager, DevicePathResolver, MockCameraManager, V4lCameraManager,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn missing_device_config(path: &Path) -> CameraConfig {
    CameraConfigBuilder::new()
        .device_path(path)
        .warmup_frames(0)
        .backoff_limits(Duration::from_millis(20), Duration::from_millis(40))
        .build()
}

/// Polls `cond` every 10 ms until it holds or `timeout` elapses.
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

#[test]
fn test_v4l_health_recovering_when_device_missing() {
    let dir = tempfile::tempdir().unwrap();
    let camera =
        V4lCameraManager::spawn(missing_device_config(&dir.path().join("video99"))).unwrap();

    assert!(
        wait_until(Duration::from_secs(2), || camera.health()
            == CameraHealth::Recovering),
        "A missing device must be reported as Recovering, got {:?}",
        camera.health()
    );
    assert!(!camera.health().is_operational());
    assert!(!camera.is_ready());
    assert!(camera.latest_frame().is_none());
}

#[test]
fn test_supervisor_without_resolver_keeps_fixed_path() {
    let dir = tempfile::tempdir().unwrap();
    let fixed = dir.path().join("by-id").join("usb-Vendor_IR-video-index0");
    let camera = V4lCameraManager::spawn(missing_device_config(&fixed)).unwrap();

    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        camera.current_device_path(),
        fixed,
        "An explicitly configured path must be retried as-is, never substituted"
    );
}

#[test]
fn test_supervisor_reresolves_device_after_enodev() {
    let dir = tempfile::tempdir().unwrap();
    let initial = dir.path().join("video0");
    let replugged = dir.path().join("video2");

    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = Arc::clone(&calls);
    let replugged_clone = replugged.clone();
    let resolver: Arc<dyn DevicePathResolver> = Arc::new(move || {
        // First two re-resolutions find nothing (device still re-enumerating), then the
        // camera reappears under a new kernel index.
        if calls_clone.fetch_add(1, Ordering::SeqCst) < 2 {
            None
        } else {
            Some(replugged_clone.clone())
        }
    });

    let camera =
        V4lCameraManager::spawn_with_resolver(missing_device_config(&initial), resolver).unwrap();

    assert!(
        wait_until(Duration::from_secs(3), || camera.current_device_path()
            == replugged),
        "Supervisor must switch to the re-resolved device path (resolver calls: {})",
        calls.load(Ordering::SeqCst)
    );
    assert!(calls.load(Ordering::SeqCst) >= 3);
    assert!(!camera.is_ready());
    assert!(!camera.health().is_operational());
}

#[test]
fn test_supervisor_reresolution_is_bounded_by_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = Arc::clone(&calls);
    let resolver: Arc<dyn DevicePathResolver> = Arc::new(move || {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        None
    });

    let config = CameraConfigBuilder::new()
        .device_path(dir.path().join("video0"))
        .warmup_frames(0)
        .backoff_limits(Duration::from_millis(50), Duration::from_millis(50))
        .build();
    let camera = V4lCameraManager::spawn_with_resolver(config, resolver).unwrap();

    thread::sleep(Duration::from_millis(600));
    let observed = calls.load(Ordering::SeqCst);
    drop(camera);

    assert!(
        observed >= 2,
        "Resolver must be consulted on every DeviceNotFound backoff (observed {observed})"
    );
    assert!(
        observed <= 20,
        "Re-resolution must be paced by the backoff, not a busy loop (observed {observed})"
    );
}

#[test]
fn test_supervisor_panic_marks_camera_dead() {
    let dir = tempfile::tempdir().unwrap();
    let resolver: Arc<dyn DevicePathResolver> =
        Arc::new(|| -> Option<PathBuf> { panic!("simulated capture supervisor fault") });

    let camera = V4lCameraManager::spawn_with_resolver(
        missing_device_config(&dir.path().join("video0")),
        resolver,
    )
    .unwrap();

    assert!(
        wait_until(Duration::from_secs(2), || camera.health()
            == CameraHealth::Dead),
        "A panicking supervisor must be reported as Dead, got {:?}",
        camera.health()
    );
    assert!(!camera.is_ready(), "A dead camera must never report ready");
    assert!(camera.latest_frame().is_none());

    // Dropping a manager whose worker died must not hang or propagate the panic.
    let start = Instant::now();
    drop(camera);
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn test_mock_health_tracks_streaming_failure_and_standby() {
    let config = CameraConfigBuilder::new()
        .warmup_frames(0)
        .idle_timeout(Duration::from_millis(150))
        .build();
    let camera = MockCameraManager::new(config);

    assert!(wait_until(Duration::from_secs(2), || camera.health()
        == CameraHealth::Streaming));

    camera.set_error(Some(CameraError::Simulated {
        code: libc::ENODEV,
        message: "No such device".to_string(),
    }));
    assert_eq!(camera.health(), CameraHealth::Recovering);
    camera.set_error(None);

    camera.notify_activity();
    assert!(
        wait_until(Duration::from_secs(2), || camera.health()
            == CameraHealth::Standby),
        "Idle auto-standby must be reported as Standby, got {:?}",
        camera.health()
    );
    assert!(!camera.is_ready(), "Standby releases the device");
    assert!(
        camera.health().is_operational(),
        "Standby is an idle but healthy state"
    );
    camera.stop();
}

#[test]
fn test_camera_health_operational_states() {
    assert!(CameraHealth::Streaming.is_operational());
    assert!(CameraHealth::Standby.is_operational());
    assert!(!CameraHealth::Starting.is_operational());
    assert!(!CameraHealth::Recovering.is_operational());
    assert!(!CameraHealth::Dead.is_operational());
    assert_eq!(CameraHealth::Dead.to_string(), "dead");
}

#[test]
fn test_stable_device_path_prefers_by_id_link() {
    let dir = tempfile::tempdir().unwrap();
    let node = dir.path().join("video2");
    std::fs::write(&node, b"").unwrap();
    let other = dir.path().join("video0");
    std::fs::write(&other, b"").unwrap();

    let by_id = dir.path().join("by-id");
    std::fs::create_dir(&by_id).unwrap();
    std::os::unix::fs::symlink(&other, by_id.join("usb-Vendor_RGB-video-index0")).unwrap();
    std::os::unix::fs::symlink(&node, by_id.join("usb-Vendor_IR-video-index1")).unwrap();
    std::os::unix::fs::symlink(&node, by_id.join("usb-Vendor_IR-video-index0")).unwrap();

    assert_eq!(
        stable_device_path(&node, &by_id),
        by_id.join("usb-Vendor_IR-video-index0"),
        "The lexicographically first by-id link resolving to the node must be returned"
    );
    assert_eq!(
        stable_device_path(&other, &by_id),
        by_id.join("usb-Vendor_RGB-video-index0")
    );
}

#[test]
fn test_stable_device_path_falls_back_to_node() {
    let dir = tempfile::tempdir().unwrap();
    let node = dir.path().join("video4");
    std::fs::write(&node, b"").unwrap();

    // No by-id directory at all.
    assert_eq!(stable_device_path(&node, &dir.path().join("absent")), node);

    // By-id directory without a matching link (dangling link included).
    let by_id = dir.path().join("by-id");
    std::fs::create_dir(&by_id).unwrap();
    std::os::unix::fs::symlink(dir.path().join("gone"), by_id.join("usb-Gone-video-index0"))
        .unwrap();
    assert_eq!(stable_device_path(&node, &by_id), node);
}

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

// GitHub #150 (review finding CAM-02): a uvcvideo node already streamed by another process opens
// fine and only fails at VIDIOC_S_FMT / REQBUFS / STREAMON with EBUSY. Those ioctl failures must be
// classified as `DeviceBusy` (and ENODEV as `DeviceNotFound`) instead of an opaque string.

fn stream_create_fallback(reason: String) -> CameraError {
    CameraError::StreamCreate {
        path: std::path::PathBuf::from("/dev/video0"),
        reason,
    }
}

#[test]
fn test_set_format_ebusy_maps_to_device_busy() {
    let path = std::path::PathBuf::from("/dev/video0");
    let err = CameraError::from_ioctl_error(
        path.clone(),
        std::io::Error::from_raw_os_error(libc::EBUSY),
        stream_create_fallback,
    );
    match err {
        CameraError::DeviceBusy { path: p, source } => {
            assert_eq!(p, path);
            assert_eq!(source.raw_os_error(), Some(libc::EBUSY));
        }
        other => panic!("EBUSY from an ioctl must map to DeviceBusy, got {other:?}"),
    }
}

#[test]
fn test_ioctl_enodev_maps_to_device_not_found() {
    let err = CameraError::from_ioctl_error(
        std::path::PathBuf::from("/dev/video0"),
        std::io::Error::from_raw_os_error(libc::ENODEV),
        stream_create_fallback,
    );
    assert!(
        matches!(err, CameraError::DeviceNotFound { .. }),
        "ENODEV from an ioctl must map to DeviceNotFound, got {err:?}"
    );
}

#[test]
fn test_ioctl_other_errno_uses_contextual_fallback() {
    let err = CameraError::from_ioctl_error(
        std::path::PathBuf::from("/dev/video0"),
        std::io::Error::from_raw_os_error(libc::EINVAL),
        stream_create_fallback,
    );
    assert!(
        matches!(err, CameraError::StreamCreate { .. }),
        "Non-busy ioctl errors must keep their contextual variant, got {err:?}"
    );
}

#[test]
fn test_device_busy_is_reported_as_busy() {
    let busy = CameraError::from_io_error(
        std::path::PathBuf::from("/dev/video0"),
        std::io::Error::from_raw_os_error(libc::EBUSY),
    );
    assert!(busy.is_device_busy());
    assert!(!CameraError::Starved.is_device_busy());
}

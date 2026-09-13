//! Tests for Criterion C4: Hardware selection by /dev/v4l/by-id/ rather than index.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Hardware config tests use assertions and unwrap"
)]

use soos_camera_v4l::{CameraConfig, CameraConfigBuilder, CameraError, PixelFormat};
use std::path::PathBuf;

#[test]
fn test_config_by_id_path_selection() {
    let persistent_path = "/dev/v4l/by-id/usb-SunplusIT_Inc_Integrated_Camera-video-index0";
    let config = CameraConfigBuilder::new()
        .device_path(persistent_path)
        .build();

    assert_eq!(config.device_path, PathBuf::from(persistent_path));
    assert!(
        config
            .device_path
            .to_string_lossy()
            .starts_with("/dev/v4l/by-id/"),
        "Camera device path must support persistent /dev/v4l/by-id/ identifiers"
    );
}

#[test]
fn test_config_builder_defaults_and_customization() {
    let default_config = CameraConfig::default();
    assert_eq!(default_config.width, 640);
    assert_eq!(default_config.height, 480);
    assert_eq!(default_config.fps, 30);
    assert_eq!(default_config.idle_fps, 5);
    assert_eq!(default_config.warmup_frames, 20);
    assert_eq!(default_config.format, PixelFormat::Yuyv);

    let custom = CameraConfigBuilder::new()
        .device_path("/dev/v4l/by-id/custom-cam")
        .resolution(1920, 1080)
        .format(PixelFormat::Rgb24)
        .fps(60)
        .idle_fps(10)
        .warmup_frames(25)
        .build();

    assert_eq!(
        custom.device_path,
        PathBuf::from("/dev/v4l/by-id/custom-cam")
    );
    assert_eq!(custom.width, 1920);
    assert_eq!(custom.height, 1080);
    assert_eq!(custom.format, PixelFormat::Rgb24);
    assert_eq!(custom.fps, 60);
    assert_eq!(custom.idle_fps, 10);
    assert_eq!(custom.warmup_frames, 25);
}

#[test]
fn test_io_error_mapping_enodev_ebusy() {
    let path = PathBuf::from("/dev/v4l/by-id/test-camera");

    // ENODEV mapping
    let enodev_err = std::io::Error::from_raw_os_error(libc::ENODEV);
    let camera_err = CameraError::from_io_error(path.clone(), enodev_err);
    assert!(
        matches!(camera_err, CameraError::DeviceNotFound { .. }),
        "ENODEV must map to CameraError::DeviceNotFound"
    );

    // ENOENT mapping
    let enoent_err = std::io::Error::from_raw_os_error(libc::ENOENT);
    let camera_err = CameraError::from_io_error(path.clone(), enoent_err);
    assert!(
        matches!(camera_err, CameraError::DeviceNotFound { .. }),
        "ENOENT must map to CameraError::DeviceNotFound"
    );

    // EBUSY mapping
    let ebusy_err = std::io::Error::from_raw_os_error(libc::EBUSY);
    let camera_err = CameraError::from_io_error(path.clone(), ebusy_err);
    assert!(
        matches!(camera_err, CameraError::DeviceBusy { .. }),
        "EBUSY must map to CameraError::DeviceBusy"
    );

    // EIO mapping
    let eio_err = std::io::Error::from_raw_os_error(libc::EIO);
    let camera_err = CameraError::from_io_error(path, eio_err);
    assert!(
        matches!(camera_err, CameraError::Io { .. }),
        "EIO must map to CameraError::Io"
    );
}

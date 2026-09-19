//! Contractual tests for path validation, FHS enforcement, and root bypass removal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use clap::Parser;
use std::path::{Path, PathBuf};

use soos_enrollment_cli::args::{
    sanitize_path, validate_camera_device_path, validate_fhs_path, Cli,
};
use soos_enrollment_cli::error::EnrollmentCliError;

#[test]
fn test_sanitize_path_blocks_parent_dir_traversal() {
    let traversal_paths = [
        "/var/lib/soos/../../etc/shadow",
        "/var/lib/soos/../soos/../etc",
        "/tmp/../var/log",
        "/dev/../root/secret",
        "../../etc/passwd",
    ];

    for path_str in &traversal_paths {
        let res = sanitize_path(Path::new(path_str));
        match res {
            Err(EnrollmentCliError::InvalidPath(msg)) => {
                assert!(
                    msg.to_lowercase().contains("traversal") || msg.to_lowercase().contains(".."),
                    "Expected traversal rejection message for '{path_str}', got: {msg}"
                );
            }
            other => {
                panic!("Expected Err(InvalidPath) for traversal path '{path_str}', got: {other:?}")
            }
        }
    }
}

#[test]
fn test_sanitize_path_blocks_relative_paths() {
    let relative_paths = [
        "relative/path/to/key",
        "master.key",
        "./biometrics",
        "models/ultraface.onnx",
    ];

    for path_str in &relative_paths {
        let res = sanitize_path(Path::new(path_str));
        match res {
            Err(EnrollmentCliError::InvalidPath(msg)) => {
                assert!(
                    msg.to_lowercase().contains("absolute"),
                    "Expected absolute path error for '{path_str}', got: {msg}"
                );
            }
            other => {
                panic!("Expected Err(InvalidPath) for relative path '{path_str}', got: {other:?}")
            }
        }
    }
}

#[test]
fn test_validate_fhs_path_allows_valid_system_directories() {
    let valid_paths = [
        "/var/lib/soos/biometrics",
        "/var/lib/soos/master.key",
        "/run/soos/daemon.sock",
        "/etc/soos/config.toml",
        "/usr/share/soos/models",
        "/tmp/soos_test_123/bio",
        "/home/hadrien/Project/soos/models",
        "/dev/video0",
        "/opt/soos/models",
    ];

    for path_str in &valid_paths {
        let res = validate_fhs_path(Path::new(path_str));
        assert!(
            res.is_ok(),
            "Expected valid FHS path for '{path_str}', got error: {:?}",
            res.err()
        );
        assert_eq!(res.unwrap(), PathBuf::from(path_str));
    }
}

#[test]
fn test_validate_fhs_path_rejects_non_fhs_locations() {
    let invalid_locations = [
        "/root/secret_key",
        "/boot/efi/kernel",
        "/srv/unauthorized_store",
        "/sys/kernel/debug",
        "/proc/1/environ",
    ];

    for path_str in &invalid_locations {
        let res = validate_fhs_path(Path::new(path_str));
        match res {
            Err(EnrollmentCliError::InvalidPath(msg)) => {
                assert!(
                    msg.to_lowercase().contains("fhs")
                        || msg.to_lowercase().contains("hierarchy")
                        || msg.to_lowercase().contains("prefix"),
                    "Expected FHS hierarchy violation error for '{path_str}', got: {msg}"
                );
            }
            other => {
                panic!("Expected Err(InvalidPath) for non-FHS path '{path_str}', got: {other:?}")
            }
        }
    }
}

#[test]
fn test_validate_camera_device_path_requires_dev() {
    let valid_cameras = [
        "/dev/video0",
        "/dev/video1",
        "/dev/v4l/by-id/usb-camera-12345",
        "/dev/v4l/by-path/pci-0000:00:14.0-usb-0:1:1.0-video-index0",
    ];

    for dev in &valid_cameras {
        let res = validate_camera_device_path(Path::new(dev));
        assert!(
            res.is_ok(),
            "Expected valid camera device path for '{dev}', got error: {:?}",
            res.err()
        );
    }

    let invalid_cameras = [
        "/etc/shadow",
        "/var/lib/soos/master.key",
        "/tmp/fake_camera",
        "/home/user/video0",
        "/dev/../etc/passwd",
    ];

    for dev in &invalid_cameras {
        let res = validate_camera_device_path(Path::new(dev));
        match res {
            Err(EnrollmentCliError::InvalidPath(msg)) => {
                assert!(
                    msg.contains("/dev/"),
                    "Expected camera device path error mentioning /dev/ for '{dev}', got: {msg}"
                );
            }
            other => panic!("Expected Err(InvalidPath) for non-dev camera '{dev}', got: {other:?}"),
        }
    }
}

#[test]
fn test_cli_rejects_skip_root_check_argument() {
    let args = ["soos-enroll", "--skip-root-check", "list"];
    let res = Cli::try_parse_from(args);
    assert!(
        res.is_err(),
        "SECURITY INVARIANT VIOLATION: --skip-root-check must not be accepted by CLI parser"
    );
}

#[test]
fn test_build_store_only_rejects_traversal_paths() {
    use soos_enrollment_cli::args::{Commands, ListArgs};
    use soos_enrollment_cli::build_store_only;

    let cli = Cli {
        biometrics_dir: Some(PathBuf::from("/var/lib/soos/../../etc")),
        key_file: Some(PathBuf::from("/var/lib/soos/master.key")),
        models_dir: None,
        camera_device: None,
        mock: false,
        command: Commands::List(ListArgs::default()),
    };

    let res = build_store_only(&cli);
    assert!(
        matches!(res, Err(EnrollmentCliError::InvalidPath(_))),
        "Expected Err(InvalidPath) for traversal biometrics_dir in build_store_only"
    );
}

#[test]
fn test_build_full_service_rejects_non_dev_camera() {
    use soos_enrollment_cli::args::{Commands, VerifyArgs};
    use soos_enrollment_cli::build_full_service;

    let cli = Cli {
        biometrics_dir: Some(PathBuf::from("/var/lib/soos/biometrics")),
        key_file: Some(PathBuf::from("/var/lib/soos/master.key")),
        models_dir: Some(PathBuf::from("/var/lib/soos/models")),
        camera_device: Some(PathBuf::from("/etc/shadow")),
        mock: false,
        command: Commands::Verify(VerifyArgs {
            uid: Some(1000),
            username: None,
        }),
    };

    let res = build_full_service(&cli);
    assert!(
        matches!(res, Err(EnrollmentCliError::InvalidPath(_))),
        "Expected Err(InvalidPath) for non-dev camera in build_full_service"
    );
}

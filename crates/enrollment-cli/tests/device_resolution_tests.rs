//! Contractual tests for camera device auto-resolution from CLI and daemon configuration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Test suite uses assertions and unwraps"
)]

use soos_enrollment_cli::service::resolve_camera_device_from_config;
use std::path::{Path, PathBuf};

#[test]
fn test_resolve_camera_device_explicit_path() {
    let custom = PathBuf::from("/dev/video42");
    let resolved = resolve_camera_device_from_config(Some(custom.clone()), None);
    assert_eq!(resolved, custom);
}

#[test]
fn test_resolve_camera_device_auto_resolution_with_config() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config_path = temp_dir.path().join("daemon.toml");
    std::fs::write(
        &config_path,
        r#"
[pipeline]
camera_device = "auto"
sensor_preference = "prefer_ir"
"#,
    )
    .unwrap();

    let resolved = resolve_camera_device_from_config(None, Some(&config_path));
    // The resolved path must NOT be literal "auto", but must reside under /dev/
    assert_ne!(
        resolved,
        PathBuf::from("auto"),
        "Resolved device must not be literal 'auto'"
    );
    assert!(
        resolved.starts_with(Path::new("/dev")),
        "Resolved device '{}' must reside under /dev/",
        resolved.display()
    );
}

#[test]
fn test_resolve_camera_device_ignores_literal_auto_in_cli_arg() {
    let cli_arg = PathBuf::from("auto");
    let resolved = resolve_camera_device_from_config(Some(cli_arg), None);
    assert_ne!(
        resolved,
        PathBuf::from("auto"),
        "CLI argument 'auto' must trigger resolution"
    );
    assert!(
        resolved.starts_with(Path::new("/dev")),
        "Resolved device '{}' must reside under /dev/",
        resolved.display()
    );
}

#[test]
fn test_resolve_camera_device_ignores_literal_default_in_cli_arg() {
    let cli_arg = PathBuf::from("default");
    let resolved = resolve_camera_device_from_config(Some(cli_arg), None);
    assert_ne!(resolved, PathBuf::from("default"));
    assert!(
        resolved.starts_with(Path::new("/dev")),
        "Resolved device '{}' must reside under /dev/",
        resolved.display()
    );
}

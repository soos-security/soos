//! Contractual tests for daemon TOML configuration parsing.

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

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::tempdir;

use soos_daemon::config::DaemonConfig;
use soos_daemon::error::DaemonError;

#[test]
fn test_config_file_parsing_complete() {
    let toml_str = r#"
log_level = "debug"

[socket]
socket_path = "/tmp/custom_soos.sock"
socket_dir = "/tmp"
socket_mode = 432 # 0o660
enforce_root_owner = false

[dispatcher]
max_concurrent_connections = 16
connection_timeout_ms = 350

[pipeline]
camera_device = "/dev/video2"
use_mock_camera = true
models_dir = "/custom/models"
biometrics_dir = "/custom/biometrics"
master_key_path = "/custom/master.key"

[pipeline.evidence]
enabled = true
base_dir = "/custom/evidence"
key_path = "/custom/evidence.key"
retention_days = 14
daily_cap_per_uid = 5

[pipeline.thresholds]
match_threshold = 0.75
pad_threshold = 0.90

[pipeline.rate_limit]
max_attempts = 3
window_duration_secs = 120
"#;

    let config = DaemonConfig::from_toml_str(toml_str).expect("Failed to parse TOML config");

    assert_eq!(config.log_level, "debug");

    // Socket config
    assert_eq!(
        config.socket.socket_path,
        PathBuf::from("/tmp/custom_soos.sock")
    );
    assert_eq!(config.socket.socket_dir, PathBuf::from("/tmp"));
    assert_eq!(config.socket.socket_mode, 0o660);
    assert!(!config.socket.enforce_root_owner);

    // Dispatcher config
    assert_eq!(config.dispatcher.max_concurrent_connections, 16);
    assert_eq!(
        config.dispatcher.connection_timeout,
        Duration::from_millis(350)
    );

    // Pipeline config
    assert_eq!(
        config.pipeline.camera.device_path,
        PathBuf::from("/dev/video2")
    );
    assert!(config.pipeline.use_mock_camera);
    assert_eq!(config.pipeline.models_dir, PathBuf::from("/custom/models"));
    assert_eq!(
        config.pipeline.biometrics_dir,
        PathBuf::from("/custom/biometrics")
    );
    assert_eq!(
        config.pipeline.master_key_path,
        PathBuf::from("/custom/master.key")
    );

    // Evidence config
    assert!(config.pipeline.evidence.enabled);
    assert_eq!(
        config.pipeline.evidence.base_dir,
        PathBuf::from("/custom/evidence")
    );
    assert_eq!(
        config.pipeline.evidence.key_path,
        PathBuf::from("/custom/evidence.key")
    );
    assert_eq!(config.pipeline.evidence.retention_days, 14);
    assert_eq!(config.pipeline.evidence.daily_cap_per_uid, 5);

    // Thresholds
    assert!((config.pipeline.thresholds.match_threshold() - 0.75).abs() < 1e-6);
    assert!((config.pipeline.thresholds.pad_threshold() - 0.90).abs() < 1e-6);

    // Rate limits
    assert_eq!(config.pipeline.rate_limit.max_attempts, 3);
    assert_eq!(
        config.pipeline.rate_limit.window_duration_ns,
        120 * 1_000_000_000
    );

    // Test saving to file and loading from path
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("daemon.toml");
    let mut file = File::create(&file_path).expect("Failed to create file");
    file.write_all(toml_str.as_bytes())
        .expect("Failed to write toml");
    drop(file);

    let loaded_config = DaemonConfig::load_from_path(&file_path).expect("Failed to load from path");
    assert_eq!(loaded_config.log_level, "debug");
    assert_eq!(
        loaded_config.socket.socket_path,
        PathBuf::from("/tmp/custom_soos.sock")
    );
}

#[test]
fn test_config_defaults_when_file_absent() {
    let config = DaemonConfig::load_or_default(None).expect("Default config should succeed");

    assert_eq!(
        config.socket.socket_path,
        PathBuf::from("/run/soos/daemon.sock")
    );
    assert_eq!(config.socket.socket_dir, PathBuf::from("/run/soos"));
    assert_eq!(config.socket.socket_mode, 0o660);
    assert!(config.socket.enforce_root_owner);

    assert_eq!(config.dispatcher.max_concurrent_connections, 8);
    // User decision 2026-09-30: the daemon default is raised to 2500 ms so the GDM
    // `timeout_ms=2500` line really gets daemon time (sudo stays capped at 1000 ms by PAM).
    assert_eq!(
        config.dispatcher.connection_timeout,
        Duration::from_millis(2500)
    );

    assert_eq!(
        config.pipeline.models_dir,
        PathBuf::from("/var/lib/soos/models")
    );
    assert_eq!(
        config.pipeline.biometrics_dir,
        PathBuf::from("/var/lib/soos/biometrics")
    );
    assert_eq!(
        config.pipeline.master_key_path,
        PathBuf::from("/var/lib/soos/master.key")
    );
    assert!(!config.pipeline.use_mock_camera);
    assert!(!config.pipeline.evidence.enabled);
}

#[test]
fn test_config_file_invalid_syntax_fails_closed() {
    let invalid_toml = "socket_path = [ broken";
    let res = DaemonConfig::from_toml_str(invalid_toml);
    assert!(
        matches!(res, Err(DaemonError::Config(_))),
        "Invalid TOML must fail with DaemonError::Config"
    );
}

#[test]
fn test_decision_budget_calibrated_to_pam_deadline() {
    assert_eq!(
        soos_daemon::pipeline::DECISION_BUDGET_MS, 900,
        "Decision budget must be 900ms to allow camera cold-start while staying within 1000ms PAM deadline"
    );
}

#[test]
fn test_pipeline_default_sensor_preference_is_prefer_ir() {
    let config = DaemonConfig::default();
    assert_eq!(
        config.pipeline.camera.sensor_preference,
        soos_camera_v4l::SensorPreference::PreferIr,
        "Daemon pipeline camera config must default to PreferIr"
    );
    assert_eq!(
        config.pipeline.camera.warmup_frames, 20,
        "Camera warmup frames must default to 20 per Criterion C5"
    );
}

#[test]
fn test_pipeline_config_warmup_frames_from_toml() {
    let toml = r#"
[pipeline]
warmup_frames = 5
"#;
    let config = DaemonConfig::from_toml_str(toml).unwrap();
    assert_eq!(
        config.pipeline.camera.warmup_frames, 5,
        "Pipeline config must allow overriding warmup_frames from TOML"
    );
}

#[test]
fn test_pipeline_config_camera_device_auto_resolution() {
    let toml = r#"
[pipeline]
camera_device = "auto"
"#;
    let config = DaemonConfig::from_toml_str(toml).unwrap();
    assert_eq!(
        config.pipeline.camera.device_path,
        std::path::PathBuf::from("/dev/v4l/by-id/default-camera"),
        "camera_device = 'auto' must preserve default-camera sentinel for auto device resolution"
    );
}

#[test]
fn test_daemon_toml_default_warmup_frames_is_zero() {
    let toml = r#"
[pipeline]
camera_device = "auto"
"#;
    let config = DaemonConfig::from_toml_str(toml).unwrap();
    assert_eq!(
        config.pipeline.camera.warmup_frames, 0,
        "When running from daemon.toml, warmup_frames must default to 0 for instant wake"
    );
}

#[test]
fn test_pipeline_config_idle_timeout_zero_from_toml() {
    let toml = r#"
[pipeline]
idle_timeout_secs = 0
"#;
    let config = DaemonConfig::from_toml_str(toml).unwrap();
    assert_eq!(
        config.pipeline.camera.idle_timeout,
        Duration::ZERO,
        "idle_timeout_secs = 0 must configure Duration::ZERO to disable auto-standby"
    );
}

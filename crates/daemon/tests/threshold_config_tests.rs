//! Contractual tests: `[pipeline.thresholds]` from `daemon.toml` is validated and floored
//! before it reaches the policy engine or the PAD detector (GitHub #170, PAD-04).
//!
//! A typo such as `pad_threshold = 0` used to turn anti-spoofing off silently (fail-open by
//! configuration). The daemon must now refuse to start with `DaemonError::Config`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_daemon::config::DaemonConfig;
use soos_daemon::error::DaemonError;
use soos_policy::ThresholdConfig;

fn thresholds_toml(body: &str) -> String {
    format!("[pipeline]\nuse_mock_camera = true\n\n[pipeline.thresholds]\n{body}\n")
}

fn assert_config_error(body: &str, expected_setting: &str) {
    match DaemonConfig::from_toml_str(&thresholds_toml(body)) {
        Err(DaemonError::Config(msg)) => assert!(
            msg.contains(expected_setting),
            "error for `{body}` must name `{expected_setting}`: {msg}"
        ),
        Err(other) => panic!("`{body}` must yield DaemonError::Config, got {other:?}"),
        Ok(cfg) => panic!(
            "`{body}` must be refused, but was accepted (match={}, pad={})",
            cfg.pipeline.thresholds.match_threshold(),
            cfg.pipeline.thresholds.pad_threshold()
        ),
    }
}

#[test]
fn test_daemon_toml_rejects_pad_threshold_zero() {
    assert_config_error("pad_threshold = 0.0", "pad_threshold");
}

#[test]
fn test_daemon_toml_rejects_pad_threshold_integer_zero_typo() {
    assert_config_error("pad_threshold = 0", "pad_threshold");
}

#[test]
fn test_daemon_toml_rejects_negative_pad_threshold() {
    assert_config_error("pad_threshold = -1.0", "pad_threshold");
}

#[test]
fn test_daemon_toml_rejects_nan_pad_threshold() {
    assert_config_error("pad_threshold = nan", "pad_threshold");
}

#[test]
fn test_daemon_toml_rejects_infinite_pad_threshold() {
    assert_config_error("pad_threshold = inf", "pad_threshold");
}

#[test]
fn test_daemon_toml_rejects_pad_threshold_above_one() {
    assert_config_error("pad_threshold = 1.5", "pad_threshold");
}

#[test]
fn test_daemon_toml_rejects_pad_threshold_below_floor() {
    assert_config_error("pad_threshold = 0.49", "pad_threshold");
}

#[test]
fn test_daemon_toml_rejects_match_threshold_zero() {
    assert_config_error("match_threshold = 0.0", "match_threshold");
}

#[test]
fn test_daemon_toml_rejects_match_threshold_below_floor() {
    assert_config_error(
        "match_threshold = 0.39\npad_threshold = 0.90",
        "match_threshold",
    );
}

#[test]
fn test_daemon_toml_rejects_nan_match_threshold() {
    assert_config_error("match_threshold = nan", "match_threshold");
}

#[test]
fn test_daemon_toml_accepts_floor_boundaries_and_propagates_to_vision() {
    let cfg = DaemonConfig::from_toml_str(&thresholds_toml(
        "match_threshold = 0.40\npad_threshold = 0.50",
    ))
    .expect("Thresholds exactly at the security floor must be accepted");
    assert!((cfg.pipeline.thresholds.match_threshold() - 0.40).abs() < f32::EPSILON);
    assert!((cfg.pipeline.thresholds.pad_threshold() - 0.50).abs() < f32::EPSILON);
    // The PAD detector and the vision pipeline read `vision.*`; both copies must agree.
    assert!((cfg.pipeline.vision.match_threshold - 0.40).abs() < f32::EPSILON);
    assert!((cfg.pipeline.vision.pad_threshold - 0.50).abs() < f32::EPSILON);
}

#[test]
fn test_daemon_toml_partial_thresholds_keep_validated_defaults() {
    let cfg = DaemonConfig::from_toml_str(&thresholds_toml("pad_threshold = 0.92"))
        .expect("A valid pad_threshold alone must be accepted");
    assert!(
        (cfg.pipeline.thresholds.match_threshold() - ThresholdConfig::DEFAULT_MATCH_THRESHOLD)
            .abs()
            < f32::EPSILON
    );
    assert!((cfg.pipeline.thresholds.pad_threshold() - 0.92).abs() < f32::EPSILON);
    assert!((cfg.pipeline.vision.pad_threshold - 0.92).abs() < f32::EPSILON);
}

#[test]
fn test_daemon_toml_rejected_thresholds_fail_load_from_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.toml");
    std::fs::write(&path, thresholds_toml("pad_threshold = 0")).unwrap();
    assert!(
        matches!(
            DaemonConfig::load_from_path(&path),
            Err(DaemonError::Config(_))
        ),
        "A daemon.toml that disables PAD must prevent daemon start-up"
    );
}

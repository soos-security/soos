//! Contractual tests for the operator-facing threshold security floor (GitHub #170, PAD-04).
//!
//! `ThresholdConfigBuilder::build` only checks the mathematical domain `[0.0, 1.0]`, so a value
//! such as `pad_threshold = 0.0` is "valid" yet accepts every frame as live. Configuration that
//! comes from an operator-editable file must go through `build_with_security_floor`, which also
//! refuses thresholds below `MIN_MATCH_THRESHOLD` / `MIN_PAD_THRESHOLD`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use expect and unwrap for test assertions"
)]

use soos_policy::{PolicyError, ThresholdConfig};

fn assert_rejected(match_threshold: f32, pad_threshold: f32, expected_name: &str) {
    let res = ThresholdConfig::builder()
        .match_threshold(match_threshold)
        .pad_threshold(pad_threshold)
        .build_with_security_floor();
    match res {
        Err(PolicyError::InvalidThreshold { name, .. }) => assert_eq!(
            name, expected_name,
            "match={match_threshold} pad={pad_threshold} must be rejected on {expected_name}"
        ),
        other => panic!(
            "match={match_threshold} pad={pad_threshold} must be rejected with InvalidThreshold, got {other:?}"
        ),
    }
}

#[test]
fn test_security_floor_constants() {
    assert!((ThresholdConfig::MIN_MATCH_THRESHOLD - 0.40).abs() < f32::EPSILON);
    assert!((ThresholdConfig::MIN_PAD_THRESHOLD - 0.50).abs() < f32::EPSILON);
}

#[test]
fn test_defaults_satisfy_security_floor() {
    // The shipped defaults must never be refused by the floor.
    const {
        assert!(ThresholdConfig::DEFAULT_MATCH_THRESHOLD >= ThresholdConfig::MIN_MATCH_THRESHOLD);
        assert!(ThresholdConfig::DEFAULT_PAD_THRESHOLD >= ThresholdConfig::MIN_PAD_THRESHOLD);
    }
    let config = ThresholdConfig::builder()
        .build_with_security_floor()
        .expect("Default thresholds must satisfy the security floor");
    assert_eq!(config, ThresholdConfig::default());
}

#[test]
fn test_security_floor_rejects_pad_threshold_that_disables_anti_spoofing() {
    let match_ok = ThresholdConfig::DEFAULT_MATCH_THRESHOLD;
    for pad in [0.0_f32, -0.0, -1.0, 0.1, 0.49, 0.499_99] {
        assert_rejected(match_ok, pad, "pad_threshold");
    }
}

#[test]
fn test_security_floor_rejects_match_threshold_below_floor() {
    let pad_ok = ThresholdConfig::DEFAULT_PAD_THRESHOLD;
    for m in [0.0_f32, -1.0, 0.2, 0.39, 0.399_99] {
        assert_rejected(m, pad_ok, "match_threshold");
    }
}

#[test]
fn test_security_floor_still_rejects_non_finite_and_out_of_range() {
    let match_ok = ThresholdConfig::DEFAULT_MATCH_THRESHOLD;
    let pad_ok = ThresholdConfig::DEFAULT_PAD_THRESHOLD;
    for bad in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1.000_01,
        1.5,
        42.0,
    ] {
        assert_rejected(match_ok, bad, "pad_threshold");
        assert_rejected(bad, pad_ok, "match_threshold");
    }
}

#[test]
fn test_security_floor_accepts_boundaries() {
    let floor = ThresholdConfig::builder()
        .match_threshold(ThresholdConfig::MIN_MATCH_THRESHOLD)
        .pad_threshold(ThresholdConfig::MIN_PAD_THRESHOLD)
        .build_with_security_floor()
        .expect("Thresholds exactly at the floor must be accepted");
    assert!((floor.match_threshold() - 0.40).abs() < f32::EPSILON);
    assert!((floor.pad_threshold() - 0.50).abs() < f32::EPSILON);

    let max = ThresholdConfig::builder()
        .match_threshold(1.0)
        .pad_threshold(1.0)
        .build_with_security_floor()
        .expect("1.0 is the upper bound and must be accepted");
    assert!((max.match_threshold() - 1.0).abs() < f32::EPSILON);
    assert!((max.pad_threshold() - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_security_floor_error_message_names_setting_and_floor() {
    let err = ThresholdConfig::builder()
        .pad_threshold(0.0)
        .build_with_security_floor()
        .expect_err("pad_threshold = 0.0 must be refused");
    let msg = err.to_string();
    assert!(msg.contains("pad_threshold"), "message: {msg}");
    assert!(msg.contains("0.5"), "message must state the floor: {msg}");
}

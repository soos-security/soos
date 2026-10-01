//! Contractual integration tests for ThresholdConfig and ThresholdConfigBuilder.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use expect and unwrap for test assertions"
)]

use soos_policy::{PolicyError, ThresholdConfig};

#[test]
fn test_threshold_defaults_match_literature() {
    let config = ThresholdConfig::default();
    // SFace on LFW (walkthrough 160): FAR 3.9e-6 / TAR 0.957 at 0.50, target FAR <= 0.1%
    assert!((config.match_threshold() - 0.50).abs() < f32::EPSILON);
    // NIST SP 800-63B / IR 8491 guidance for PAD
    assert!((config.pad_threshold() - 0.85).abs() < f32::EPSILON);
}

#[test]
fn test_threshold_builder_custom_valid() {
    let config = ThresholdConfig::builder()
        .match_threshold(0.75)
        .pad_threshold(0.92)
        .build()
        .expect("Valid thresholds within [0.0, 1.0] must be accepted");

    assert!((config.match_threshold() - 0.75).abs() < f32::EPSILON);
    assert!((config.pad_threshold() - 0.92).abs() < f32::EPSILON);
}

#[test]
fn test_threshold_builder_boundary_extremes() {
    let min_config = ThresholdConfig::builder()
        .match_threshold(0.0)
        .pad_threshold(0.0)
        .build()
        .expect("0.0 is a valid boundary threshold");
    assert!((min_config.match_threshold() - 0.0).abs() < f32::EPSILON);

    let max_config = ThresholdConfig::builder()
        .match_threshold(1.0)
        .pad_threshold(1.0)
        .build()
        .expect("1.0 is a valid boundary threshold");
    assert!((max_config.match_threshold() - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_threshold_builder_rejects_negative() {
    let res = ThresholdConfig::builder().match_threshold(-0.01).build();
    assert!(
        matches!(
            res,
            Err(PolicyError::InvalidThreshold {
                name: "match_threshold",
                ..
            })
        ),
        "Negative match threshold must be rejected"
    );

    let res_pad = ThresholdConfig::builder().pad_threshold(-0.5).build();
    assert!(
        matches!(
            res_pad,
            Err(PolicyError::InvalidThreshold {
                name: "pad_threshold",
                ..
            })
        ),
        "Negative pad threshold must be rejected"
    );
}

#[test]
fn test_threshold_builder_rejects_greater_than_one() {
    let res = ThresholdConfig::builder().match_threshold(1.01).build();
    assert!(
        matches!(
            res,
            Err(PolicyError::InvalidThreshold {
                name: "match_threshold",
                ..
            })
        ),
        "Match threshold > 1.0 must be rejected"
    );

    let res_pad = ThresholdConfig::builder().pad_threshold(2.5).build();
    assert!(
        matches!(
            res_pad,
            Err(PolicyError::InvalidThreshold {
                name: "pad_threshold",
                ..
            })
        ),
        "PAD threshold > 1.0 must be rejected"
    );
}

#[test]
fn test_threshold_builder_rejects_nan() {
    let res = ThresholdConfig::builder().match_threshold(f32::NAN).build();
    assert!(
        matches!(
            res,
            Err(PolicyError::InvalidThreshold {
                name: "match_threshold",
                ..
            })
        ),
        "NaN match threshold must be rejected"
    );

    let res_pad = ThresholdConfig::builder().pad_threshold(f32::NAN).build();
    assert!(
        matches!(
            res_pad,
            Err(PolicyError::InvalidThreshold {
                name: "pad_threshold",
                ..
            })
        ),
        "NaN PAD threshold must be rejected"
    );
}

#[test]
fn test_threshold_builder_rejects_infinity() {
    let res = ThresholdConfig::builder()
        .match_threshold(f32::INFINITY)
        .build();
    assert!(
        matches!(
            res,
            Err(PolicyError::InvalidThreshold {
                name: "match_threshold",
                ..
            })
        ),
        "+Inf match threshold must be rejected"
    );

    let res_pad = ThresholdConfig::builder()
        .pad_threshold(f32::NEG_INFINITY)
        .build();
    assert!(
        matches!(
            res_pad,
            Err(PolicyError::InvalidThreshold {
                name: "pad_threshold",
                ..
            })
        ),
        "-Inf PAD threshold must be rejected"
    );
}

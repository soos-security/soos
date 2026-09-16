//! Contractual test suite for Presentation Attack Detection (PAD) domain types and detectors.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions and unwraps"
)]

use soos_inference_ort::error::InferenceError;
use soos_inference_ort::mock::MockPadDetector;
use soos_inference_ort::pad::{AttackType, OrtPadDetector, PadDetector, PadResult};

#[test]
fn test_mock_pad_detector_nominal_live() {
    let detector = MockPadDetector::new_live();
    let rgb = vec![128u8; 112 * 112 * 3];

    let result = detector
        .evaluate_liveness(&rgb, 112, 112)
        .expect("evaluate_liveness should succeed");

    assert!(result.is_live, "Default mock should be live");
    assert!(result.score >= 0.90, "Liveness score should be >= 0.90");
    assert_eq!(result.attack_type, None);
}

#[test]
fn test_mock_pad_detector_spoof_print_photo() {
    let detector = MockPadDetector::new_spoof(AttackType::PrintPhoto, 0.05);
    let rgb = vec![128u8; 112 * 112 * 3];

    let result = detector
        .evaluate_liveness(&rgb, 112, 112)
        .expect("evaluate_liveness should succeed");

    assert!(!result.is_live, "Spoof mock should not be live");
    assert_eq!(result.score, 0.05);
    assert_eq!(result.attack_type, Some(AttackType::PrintPhoto));
}

#[test]
fn test_mock_pad_detector_spoof_screen_replay() {
    let detector = MockPadDetector::new_spoof(AttackType::ScreenReplay, 0.12);
    let rgb = vec![128u8; 112 * 112 * 3];

    let result = detector
        .evaluate_liveness(&rgb, 112, 112)
        .expect("evaluate_liveness should succeed");

    assert!(!result.is_live);
    assert_eq!(result.score, 0.12);
    assert_eq!(result.attack_type, Some(AttackType::ScreenReplay));
}

#[test]
fn test_mock_pad_detector_buffer_size_validation() {
    let detector = MockPadDetector::new_live();
    let wrong_buffer = vec![0u8; 100];

    let err = detector
        .evaluate_liveness(&wrong_buffer, 112, 112)
        .expect_err("Buffer size mismatch must error");

    match err {
        InferenceError::InvalidBufferSize { expected, actual } => {
            assert_eq!(expected, 112 * 112 * 3);
            assert_eq!(actual, 100);
        }
        other => panic!("Unexpected error variant: {:?}", other),
    }
}

#[test]
fn test_mock_pad_detector_fault_injection() {
    let detector = MockPadDetector::new_live();
    detector.set_fail_next(true);

    let rgb = vec![128u8; 112 * 112 * 3];
    let err = detector
        .evaluate_liveness(&rgb, 112, 112)
        .expect_err("Injected fault must error");

    match err {
        InferenceError::PadFailed(msg) => {
            assert!(msg.contains("Simulated PAD inference failure"));
        }
        other => panic!("Unexpected error variant: {:?}", other),
    }

    // Subsequent call succeeds
    let ok_result = detector
        .evaluate_liveness(&rgb, 112, 112)
        .expect("Subsequent call should succeed");
    assert!(ok_result.is_live);
}

#[test]
fn test_mock_pad_detector_dynamic_update() {
    let detector = MockPadDetector::new_live();
    let rgb = vec![128u8; 112 * 112 * 3];

    let r1 = detector.evaluate_liveness(&rgb, 112, 112).unwrap();
    assert!(r1.is_live);

    detector.set_result(PadResult::spoof(0.25, AttackType::UnknownSpoof));
    let r2 = detector.evaluate_liveness(&rgb, 112, 112).unwrap();
    assert!(!r2.is_live);
    assert_eq!(r2.score, 0.25);
    assert_eq!(r2.attack_type, Some(AttackType::UnknownSpoof));
}

#[test]
fn test_softmax_numerical_stability() {
    // Empty
    assert!(OrtPadDetector::softmax(&[]).is_empty());

    // Identical values should produce uniform distribution
    let uniform = OrtPadDetector::softmax(&[1.0, 1.0]);
    assert_eq!(uniform.len(), 2);
    assert!((uniform[0] - 0.5).abs() < 1e-5);
    assert!((uniform[1] - 0.5).abs() < 1e-5);

    // Sum must always equal 1.0
    let probs = OrtPadDetector::softmax(&[-2.5, 3.8, 0.1]);
    assert_eq!(probs.len(), 3);
    let sum: f32 = probs.iter().sum();
    assert!((sum - 1.0).abs() < 1e-5);
    assert!(probs[1] > probs[0] && probs[1] > probs[2]);

    // Very large values must not cause NaN due to exp overflow
    let extreme = OrtPadDetector::softmax(&[1000.0, 1005.0, 990.0]);
    assert_eq!(extreme.len(), 3);
    for p in &extreme {
        assert!(!p.is_nan(), "Probability must not be NaN");
        assert!(*p >= 0.0 && *p <= 1.0, "Probability must be in [0, 1]");
    }
    let extreme_sum: f32 = extreme.iter().sum();
    assert!((extreme_sum - 1.0).abs() < 1e-4);
}

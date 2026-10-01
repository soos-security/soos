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

#[test]
fn test_pad_prepare_input_80x80_bgr() {
    // 80x80 RGB image with distinct channel values per pixel:
    // Pixel (x, y): R = 255, G = 128, B = 64
    let mut rgb = Vec::with_capacity(80 * 80 * 3);
    for _ in 0..(80 * 80) {
        rgb.push(255u8); // R
        rgb.push(128u8); // G
        rgb.push(64u8); // B
    }

    let input_tensor =
        OrtPadDetector::prepare_input(&rgb, 80, 80).expect("prepare_input for 80x80 must succeed");

    // MiniFASNetV2 tensor shape: [1, 3, 80, 80] -> length 19,200
    assert_eq!(
        input_tensor.len(),
        3 * 80 * 80,
        "Tensor length must be 3 * 80 * 80 = 19200"
    );

    // Raw upstream range (ADR 2026-10-01 "PAD Input Range Matches Upstream (0-255)"):
    // Channel 0 = Blue: 64.0, Channel 1 = Green: 128.0, Channel 2 = Red: 255.0
    let b_val = input_tensor[0];
    let g_val = input_tensor[80 * 80];
    let r_val = input_tensor[2 * 80 * 80];

    assert!(
        (b_val - 64.0).abs() < 1e-4,
        "Channel 0 must be Blue in the raw [0, 255] range, got {b_val}"
    );
    assert!(
        (g_val - 128.0).abs() < 1e-4,
        "Channel 1 must be Green in the raw [0, 255] range, got {g_val}"
    );
    assert!(
        (r_val - 255.0).abs() < 1e-4,
        "Channel 2 must be Red in the raw [0, 255] range, got {r_val}"
    );
}

#[test]
fn test_pad_normalization_0_1_range() {
    // 80x80 image with extreme pixel values (0 and 255)
    let mut rgb = vec![0u8; 80 * 80 * 3];
    // Fill first pixel with 255
    rgb[0] = 255;
    rgb[1] = 255;
    rgb[2] = 255;
    // Fill second pixel with 128
    rgb[3] = 128;
    rgb[4] = 128;
    rgb[5] = 128;

    let tensor = OrtPadDetector::prepare_input(&rgb, 80, 80).expect("prepare_input must succeed");

    for (i, &val) in tensor.iter().enumerate() {
        assert!(
            (0.0..=255.0).contains(&val),
            "Pixel at index {i} must stay in the raw range [0.0, 255.0], got {val}"
        );
    }

    // Min value should be 0.0 (pixel 2 is 0)
    assert_eq!(tensor[2], 0.0, "Black pixel must normalize to 0.0");
    // Max value should be 255.0 (pixel 0 is 255)
    assert!(
        (tensor[2 * 80 * 80] - 255.0).abs() < 1e-4,
        "White pixel must map to 255.0"
    );
}

#[test]
fn test_pad_invalid_dimensions_message_80x80() {
    let err =
        OrtPadDetector::prepare_input(&[], 0, 0).expect_err("Zero dimension must fail validation");

    match err {
        InferenceError::InvalidDimensions { expected, actual } => {
            assert_eq!(expected, (80, 80), "Expected dimensions must be (80, 80)");
            assert_eq!(actual, (0, 0));
        }
        other => panic!("Unexpected error variant: {:?}", other),
    }
}

#[test]
fn test_pad_class_ordering_live_index_0() {
    let threshold = 0.80f32;

    // MiniFASNetV2 class ordering: [Class 0 = Live, Class 1 = Print, Class 2 = Replay]
    // 1. Nominal Live presentation: Class 0 has highest prob >= threshold
    let live_probs = [0.95, 0.03, 0.02];
    let res = OrtPadDetector::interpret_probabilities(&live_probs, threshold, 0)
        .expect("interpret_probabilities should succeed");
    assert!(res.is_live, "Should be classified as live");
    assert_eq!(res.score, 0.95);
    assert_eq!(res.attack_type, None);

    // 2. PrintPhoto spoof: Class 0 < threshold, Class 1 (Print) > Class 2 (Replay)
    let print_probs = [0.10, 0.70, 0.20];
    let res = OrtPadDetector::interpret_probabilities(&print_probs, threshold, 0)
        .expect("interpret_probabilities should succeed");
    assert!(!res.is_live, "Should be spoof");
    assert_eq!(res.score, 0.10);
    assert_eq!(res.attack_type, Some(AttackType::PrintPhoto));

    // 3. ScreenReplay spoof: Class 0 < threshold, Class 2 (Replay) > Class 1 (Print)
    let replay_probs = [0.10, 0.20, 0.70];
    let res = OrtPadDetector::interpret_probabilities(&replay_probs, threshold, 0)
        .expect("interpret_probabilities should succeed");
    assert!(!res.is_live, "Should be spoof");
    assert_eq!(res.score, 0.10);
    assert_eq!(res.attack_type, Some(AttackType::ScreenReplay));
}

#[test]
fn test_pad_class_ordering_configurable() {
    let threshold = 0.80f32;

    // Legacy MiniFASNet class ordering: [Class 0 = Print, Class 1 = Live, Class 2 = Replay]
    // When live_class_index = 1:
    // 1. Live presentation at index 1
    let probs_legacy_live = [0.05, 0.92, 0.03];
    let res_idx1 = OrtPadDetector::interpret_probabilities(&probs_legacy_live, threshold, 1)
        .expect("interpret_probabilities should succeed");
    assert!(res_idx1.is_live);
    assert_eq!(res_idx1.score, 0.92);
    assert_eq!(res_idx1.attack_type, None);

    // With index 0, same probs would be spoof (p_live = 0.05)
    let res_idx0 = OrtPadDetector::interpret_probabilities(&probs_legacy_live, threshold, 0)
        .expect("interpret_probabilities should succeed");
    assert!(!res_idx0.is_live);
    assert_eq!(res_idx0.score, 0.05);

    // 2. Print photo spoof with live_class_index = 1:
    // Non-live are Class 0 (0.75) and Class 2 (0.15)
    let probs_legacy_print = [0.75, 0.10, 0.15];
    let res_print = OrtPadDetector::interpret_probabilities(&probs_legacy_print, threshold, 1)
        .expect("interpret_probabilities should succeed");
    assert!(!res_print.is_live);
    assert_eq!(res_print.attack_type, Some(AttackType::PrintPhoto));

    // 3. Screen replay spoof with live_class_index = 1:
    let probs_legacy_replay = [0.15, 0.10, 0.75];
    let res_replay = OrtPadDetector::interpret_probabilities(&probs_legacy_replay, threshold, 1)
        .expect("interpret_probabilities should succeed");
    assert!(!res_replay.is_live);
    assert_eq!(res_replay.attack_type, Some(AttackType::ScreenReplay));
}

#[test]
fn test_pad_default_live_class_index_is_one() {
    assert_eq!(
        soos_inference_ort::pad::DEFAULT_MINIFASNET_LIVE_CLASS_INDEX, 1,
        "MiniFASNetV2 default live class index must be 1 (Class 0: PrintPhoto, Class 1: Genuine Live, Class 2: ScreenReplay)"
    );
}

#[test]
fn test_screen_replay_detected_as_spoof_with_default_index() {
    let threshold = 0.80f32;
    let live_index = soos_inference_ort::pad::DEFAULT_MINIFASNET_LIVE_CLASS_INDEX;

    // 1. Genuine Live presentation (Class 1 dominant):
    let probs_live = [0.03, 0.94, 0.03];
    let res_live = OrtPadDetector::interpret_probabilities(&probs_live, threshold, live_index)
        .expect("interpret_probabilities should succeed");
    assert!(
        res_live.is_live,
        "Real face at class 1 must be classified as live"
    );
    assert_eq!(res_live.score, 0.94);
    assert_eq!(res_live.attack_type, None);

    // 2. Phone Screen Replay attack (Class 2 dominant):
    let probs_replay = [0.05, 0.05, 0.90];
    let res_replay = OrtPadDetector::interpret_probabilities(&probs_replay, threshold, live_index)
        .expect("interpret_probabilities should succeed");
    assert!(
        !res_replay.is_live,
        "Phone screen replay must be classified as spoof"
    );
    assert_eq!(res_replay.score, 0.05);
    assert_eq!(res_replay.attack_type, Some(AttackType::ScreenReplay));

    // 3. Print Photo attack (Class 0 dominant):
    let probs_print = [0.92, 0.04, 0.04];
    let res_print = OrtPadDetector::interpret_probabilities(&probs_print, threshold, live_index)
        .expect("interpret_probabilities should succeed");
    assert!(
        !res_print.is_live,
        "Printed photo must be classified as spoof"
    );
    assert_eq!(res_print.score, 0.04);
    assert_eq!(res_print.attack_type, Some(AttackType::PrintPhoto));
}

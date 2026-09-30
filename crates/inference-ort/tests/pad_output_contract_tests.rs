//! Contract tests for the PAD output contract (GitHub #214, review finding PAD-09).
//!
//! A PAD model whose output length disagrees with the MiniFASNetV2 contract, or a live class
//! index outside the output vector, must be an error and never a silent verdict: an out-of-range
//! index used to read as `p_live = 0.0` (permanent spoof) and a single-logit head used to score
//! every frame live (softmax over one logit is 1.0).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions and unwraps"
)]

use std::sync::{Arc, Mutex};

use soos_inference_ort::error::InferenceError;
use soos_inference_ort::pad::{
    pad_class_count_from_manifest, validate_pad_output_contract, OrtPadDetector, PadDetector,
    DEFAULT_MINIFASNET_LIVE_CLASS_INDEX, MINIFASNET_CLASS_COUNT,
};
use soos_inference_ort::SharedSession;

#[path = "../../../tests/fixtures/pad_onnx.rs"]
mod pad_onnx;

fn session_from(model: &[u8]) -> SharedSession {
    let session = ort::session::Session::builder()
        .expect("ORT session builder")
        .commit_from_memory(model)
        .expect("PAD fixture ONNX model must load");
    Arc::new(Mutex::new(session))
}

fn assert_pad_failed<T: std::fmt::Debug>(result: Result<T, InferenceError>, context: &str) {
    match result {
        Err(InferenceError::PadFailed(_)) => {}
        other => panic!("{context}: expected InferenceError::PadFailed, got {other:?}"),
    }
}

#[test]
fn test_minifasnet_class_count_is_three() {
    assert_eq!(MINIFASNET_CLASS_COUNT, 3);
    const { assert!(DEFAULT_MINIFASNET_LIVE_CLASS_INDEX < MINIFASNET_CLASS_COUNT) };
}

#[test]
fn test_interpret_probabilities_rejects_out_of_range_index_three_class() {
    assert_pad_failed(
        OrtPadDetector::interpret_probabilities(&[0.05, 0.05, 0.90], 0.85, 3),
        "index 3 on a 3-class vector",
    );
    assert_pad_failed(
        OrtPadDetector::interpret_probabilities(&[0.05, 0.05, 0.90], 0.85, usize::MAX),
        "index usize::MAX on a 3-class vector",
    );
}

#[test]
fn test_interpret_probabilities_rejects_out_of_range_index_two_class() {
    assert_pad_failed(
        OrtPadDetector::interpret_probabilities(&[0.10, 0.90], 0.85, 2),
        "index 2 on a 2-class vector",
    );
}

#[test]
fn test_interpret_probabilities_rejects_out_of_range_index_single_output() {
    assert_pad_failed(
        OrtPadDetector::interpret_probabilities(&[0.99], 0.85, 1),
        "index 1 on a single-output vector",
    );
}

#[test]
fn test_interpret_probabilities_in_range_index_still_classifies() {
    let res = OrtPadDetector::interpret_probabilities(&[0.03, 0.94, 0.03], 0.85, 1)
        .expect("in-range index must classify");
    assert!(res.is_live);
    let res = OrtPadDetector::interpret_probabilities(&[0.9], 0.85, 0)
        .expect("index 0 on a sigmoid output must classify");
    assert!(res.is_live);
}

#[test]
fn test_validate_pad_output_contract() {
    validate_pad_output_contract(3, 1, 3).expect("nominal MiniFASNetV2 contract must pass");
    assert_pad_failed(validate_pad_output_contract(1, 1, 3), "single logit");
    assert_pad_failed(validate_pad_output_contract(2, 1, 3), "two-class head");
    assert_pad_failed(
        validate_pad_output_contract(19_200, 1, 3),
        "flattened input",
    );
    assert_pad_failed(validate_pad_output_contract(3, 3, 3), "index == length");
    assert_pad_failed(validate_pad_output_contract(0, 0, 3), "empty output");
}

#[test]
fn test_pad_class_count_from_manifest() {
    assert_eq!(
        pad_class_count_from_manifest(&[vec![1, 3]]).expect("[[1, 3]] is the shipped contract"),
        3
    );
    assert_eq!(
        pad_class_count_from_manifest(&[]).expect("omitted output_shapes falls back"),
        MINIFASNET_CLASS_COUNT
    );
    assert_pad_failed(
        pad_class_count_from_manifest(&[vec![1, 2]]),
        "2-class manifest",
    );
    assert_pad_failed(
        pad_class_count_from_manifest(&[vec![1, 1]]),
        "1-class manifest",
    );
    assert_pad_failed(pad_class_count_from_manifest(&[vec![]]), "rank-0 output");
    assert_pad_failed(
        pad_class_count_from_manifest(&[vec![1, 3], vec![1, 3]]),
        "two declared outputs",
    );
}

#[test]
fn test_evaluate_liveness_rejects_single_logit_head() {
    let detector = OrtPadDetector::new(session_from(&pad_onnx::single_logit_pad_model()), 0.85);
    let rgb = vec![128u8; 80 * 80 * 3];
    assert_pad_failed(
        detector.evaluate_liveness(&rgb, 80, 80),
        "single-logit head must never score a frame live",
    );
}

#[test]
fn test_evaluate_liveness_rejects_wrong_output_length() {
    let detector = OrtPadDetector::new(session_from(&pad_onnx::flatten_pad_model()), 0.85);
    let rgb = vec![128u8; 80 * 80 * 3];
    assert_pad_failed(
        detector.evaluate_liveness(&rgb, 80, 80),
        "19200-element output must not be read as a class vector",
    );
}

#[test]
fn test_evaluate_liveness_three_class_head_classifies() {
    let detector = OrtPadDetector::new(session_from(&pad_onnx::three_class_pad_model()), 0.85);
    let rgb = vec![128u8; 80 * 80 * 3];
    let result = detector
        .evaluate_liveness(&rgb, 80, 80)
        .expect("well-shaped 3-class head must classify");
    // Uniform input -> equal logits -> p_live = 1/3 < 0.85.
    assert!(!result.is_live);
}

#[test]
fn test_self_test_passes_for_three_class_head() {
    let detector = OrtPadDetector::new(session_from(&pad_onnx::three_class_pad_model()), 0.85);
    let report = detector
        .self_test(MINIFASNET_CLASS_COUNT)
        .expect("self-test must pass on a 3-class head");
    assert_eq!(report.class_count, 3);
    assert_eq!(report.live_class_index, DEFAULT_MINIFASNET_LIVE_CLASS_INDEX);
    assert_eq!(report.liveness_threshold, 0.85);
}

#[test]
fn test_self_test_fails_closed_on_single_logit_head() {
    let detector = OrtPadDetector::new(session_from(&pad_onnx::single_logit_pad_model()), 0.85);
    assert_pad_failed(detector.self_test(MINIFASNET_CLASS_COUNT), "single logit");
}

#[test]
fn test_self_test_fails_closed_on_wrong_output_length() {
    let detector = OrtPadDetector::new(session_from(&pad_onnx::flatten_pad_model()), 0.85);
    assert_pad_failed(detector.self_test(MINIFASNET_CLASS_COUNT), "flattened head");
}

#[test]
fn test_self_test_fails_closed_on_out_of_range_index() {
    let detector = OrtPadDetector::new_with_class_index(
        session_from(&pad_onnx::three_class_pad_model()),
        0.85,
        3,
    );
    assert_pad_failed(detector.self_test(MINIFASNET_CLASS_COUNT), "index 3");
}

#[test]
fn test_self_test_fails_closed_on_manifest_class_count_mismatch() {
    let detector = OrtPadDetector::new(session_from(&pad_onnx::three_class_pad_model()), 0.85);
    assert_pad_failed(detector.self_test(2), "manifest declares 2 classes");
}

/// Real-model evidence: the attested MiniFASNetV2 passes the startup self-test with the
/// manifest-derived class count. Skipped (printed) when the model is not installed, unless
/// `SOOS_REQUIRE_REAL_MODELS=1`.
#[test]
#[allow(
    clippy::print_stdout,
    reason = "Prints a SKIPPED marker when the model is absent"
)]
fn test_self_test_passes_on_real_minifasnet_model() {
    use soos_inference_ort::registry::{ModelRegistry, RegistryConfig};

    let dir = std::env::var_os("SOOS_MODELS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/var/lib/soos/models"));
    if !dir.join("minifasnet_v2_80x80.onnx").is_file() {
        assert!(
            std::env::var("SOOS_REQUIRE_REAL_MODELS").map_or(true, |v| v != "1"),
            "SOOS_REQUIRE_REAL_MODELS=1 but the PAD model is missing from {}",
            dir.display()
        );
        println!("SKIPPED test_self_test_passes_on_real_minifasnet_model: PAD model not installed");
        return;
    }
    let manifest =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    let mut registry = ModelRegistry::new(RegistryConfig::with_manifest(&dir, manifest))
        .expect("committed manifest must parse");
    let expected = pad_class_count_from_manifest(
        &registry
            .manifest()
            .get_model("minifasnet_v2_pad")
            .expect("PAD manifest entry")
            .output_shapes,
    )
    .expect("shipped manifest declares the 3-class contract");
    let session = registry
        .get_or_load_session("minifasnet_v2_pad")
        .expect("installed PAD model must match the committed manifest");
    let report = OrtPadDetector::new(session, 0.85)
        .self_test(expected)
        .expect("attested MiniFASNetV2 must pass the startup self-test");
    assert_eq!(report.class_count, MINIFASNET_CLASS_COUNT);
    assert_eq!(report.live_class_index, DEFAULT_MINIFASNET_LIVE_CLASS_INDEX);
}

//! Contract tests for the daemon's PAD startup validation (GitHub #214, review finding PAD-09).
//!
//! `initialize_pipeline` must run a PAD self-test against the manifest-declared output shape and
//! fail closed on a wrong head or an out-of-range live class index, instead of starting a daemon
//! that silently denies (or accepts) every presentation.
//!
//! Kept in its own test binary (ORT sessions next to timing-sensitive tests made them flaky).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions and unwraps"
)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use soos_daemon::error::DaemonError;
use soos_daemon::pipeline::{build_pad_detector, validate_pad_detector, PAD_MODEL_ID};
use soos_inference_ort::pad::DEFAULT_MINIFASNET_LIVE_CLASS_INDEX;
use soos_inference_ort::{InferenceError, ModelManifest, OrtPadDetector, SharedSession};

#[path = "../../../tests/fixtures/pad_onnx.rs"]
mod pad_onnx;

fn session_from(model: &[u8]) -> SharedSession {
    let session = ort::session::Session::builder()
        .expect("ORT session builder")
        .commit_from_memory(model)
        .expect("PAD fixture ONNX model must load");
    Arc::new(Mutex::new(session))
}

fn shipped_manifest_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    std::fs::read_to_string(path).expect("repository models/manifest.toml must be readable")
}

fn shipped_manifest() -> ModelManifest {
    ModelManifest::from_toml_str(&shipped_manifest_source()).expect("shipped manifest parses")
}

fn assert_pad_failed<T: std::fmt::Debug>(result: Result<T, DaemonError>, context: &str) {
    match result {
        Err(DaemonError::Inference(InferenceError::PadFailed(_))) => {}
        other => panic!("{context}: expected DaemonError::Inference(PadFailed), got {other:?}"),
    }
}

#[test]
fn test_pad_model_id_matches_shipped_manifest() {
    assert_eq!(PAD_MODEL_ID, "minifasnet_v2_pad");
    assert!(shipped_manifest().get_model(PAD_MODEL_ID).is_some());
}

#[test]
fn test_validate_pad_detector_accepts_shipped_contract() {
    let pad = build_pad_detector(session_from(&pad_onnx::three_class_pad_model()), 0.85);
    let report = validate_pad_detector(&pad, &shipped_manifest())
        .expect("3-class head with the shipped manifest must validate");
    assert_eq!(report.class_count, 3);
    assert_eq!(report.live_class_index, DEFAULT_MINIFASNET_LIVE_CLASS_INDEX);
    assert_eq!(report.liveness_threshold, 0.85);
}

#[test]
fn test_validate_pad_detector_fails_closed_on_single_logit_head() {
    let pad = build_pad_detector(session_from(&pad_onnx::single_logit_pad_model()), 0.85);
    assert_pad_failed(
        validate_pad_detector(&pad, &shipped_manifest()),
        "single-logit head",
    );
}

#[test]
fn test_validate_pad_detector_fails_closed_on_wrong_output_length() {
    let pad = build_pad_detector(session_from(&pad_onnx::flatten_pad_model()), 0.85);
    assert_pad_failed(
        validate_pad_detector(&pad, &shipped_manifest()),
        "flattened head",
    );
}

#[test]
fn test_validate_pad_detector_fails_closed_on_out_of_range_index() {
    let pad = OrtPadDetector::new_with_class_index(
        session_from(&pad_onnx::three_class_pad_model()),
        0.85,
        3,
    );
    assert_pad_failed(
        validate_pad_detector(&pad, &shipped_manifest()),
        "live class index 3",
    );
}

#[test]
fn test_validate_pad_detector_fails_closed_on_manifest_class_count_mismatch() {
    let source = shipped_manifest_source();
    let pad_section = source
        .find("[models.minifasnet_v2_pad]")
        .expect("PAD section present");
    let (head, tail) = source.split_at(pad_section);
    let tail = tail.replacen("output_shapes = [[1, 3]]", "output_shapes = [[1, 2]]", 1);
    assert_ne!(
        tail,
        source[pad_section..],
        "PAD output_shapes line must be rewritten"
    );
    let manifest = ModelManifest::from_toml_str(&format!("{head}{tail}")).expect("parses");

    let pad = build_pad_detector(session_from(&pad_onnx::three_class_pad_model()), 0.85);
    assert_pad_failed(
        validate_pad_detector(&pad, &manifest),
        "manifest declares a 2-class PAD head",
    );
}

#[test]
fn test_validate_pad_detector_fails_closed_without_pad_manifest_entry() {
    let mut manifest = shipped_manifest();
    manifest.models.remove(PAD_MODEL_ID);
    let pad = build_pad_detector(session_from(&pad_onnx::three_class_pad_model()), 0.85);
    match validate_pad_detector(&pad, &manifest) {
        Err(DaemonError::Inference(InferenceError::ModelNotFound { id, .. })) => {
            assert_eq!(id, PAD_MODEL_ID);
        }
        other => panic!("expected ModelNotFound for the PAD entry, got {other:?}"),
    }
}

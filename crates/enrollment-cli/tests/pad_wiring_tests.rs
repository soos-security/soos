//! Contractual tests binding the `soos-enroll` PAD detector construction to the
//! crate default live class index (GitHub #146: PAD-01 / STO-01).

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

use std::sync::{Arc, Mutex};

use soos_enrollment_cli::service::build_pad_detector;
use soos_inference_ort::pad::DEFAULT_MINIFASNET_LIVE_CLASS_INDEX;
use soos_inference_ort::SharedSession;

// User-approved 2026-09-30 (GitHub #241): the shared fixtures are a dev-dependency crate.
use soos_test_fixtures as fixtures;

fn in_memory_session() -> SharedSession {
    let model = fixtures::onnx::minimal_identity_model();
    let session = ort::session::Session::builder()
        .expect("ORT session builder")
        .commit_from_memory(&model)
        .expect("Minimal identity ONNX model must load");
    Arc::new(Mutex::new(session))
}

/// The enrollment and diagnostic paths (`soos-enroll enroll|verify`) must score liveness
/// from the same class as the daemon and the GUI: the crate default, never a literal.
#[test]
fn test_enrollment_pad_detector_uses_default_live_class_index() {
    let threshold = 0.80f32;
    let pad = build_pad_detector(in_memory_session(), threshold);

    assert_eq!(
        pad.live_class_index(),
        DEFAULT_MINIFASNET_LIVE_CLASS_INDEX,
        "soos-enroll PAD detector must use DEFAULT_MINIFASNET_LIVE_CLASS_INDEX"
    );
    assert_eq!(
        pad.live_class_index(),
        1,
        "MiniFASNetV2 contract: class 1 = genuine live (class 2 = screen replay)"
    );
    assert_eq!(pad.liveness_threshold(), threshold);
}

/// With the default index a screen-replay-dominant distribution must be a spoof and a
/// class-1-dominant distribution must be live, through the detector built by the CLI.
#[test]
fn test_enrollment_pad_detector_classifies_replay_as_spoof() {
    let pad = build_pad_detector(in_memory_session(), 0.80);

    let replay = pad
        .classify_probabilities(&[0.05, 0.05, 0.90])
        .expect("classification must succeed");
    assert!(
        !replay.is_live,
        "Screen replay column (class 2) must never be read as liveness by soos-enroll"
    );

    let genuine = pad
        .classify_probabilities(&[0.03, 0.94, 0.03])
        .expect("classification must succeed");
    assert!(
        genuine.is_live,
        "Genuine live column (class 1) must pass PAD"
    );
}

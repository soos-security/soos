//! Embedding extractor I/O contract (GitHub #268, review finding VIS-14).
//!
//! - The extractor must reject a model output whose length is not the attested dimension of the
//!   shipped model (128 for SFace since GitHub #278) instead of emitting a vector that only fails
//!   later in the matcher.
//! - The session layout must be the shipped model's layout (NCHW); an NHWC session fails closed.
//! - The input layout is inferred from the session input; when it cannot be inferred (no input,
//!   rank other than 4, no channel axis of size 3) extraction fails closed instead of silently
//!   assuming NCHW.
//!
//! The sessions are real ONNX Runtime sessions built from hand-encoded graphs
//! (`tests/fixtures/embedding_onnx.rs`), so no model file is needed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwraps"
)]

use std::sync::{Arc, Mutex};

use soos_inference_ort::embedding::{
    EmbeddingModelSpec, EMBEDDING_DIMENSION, SFACE_2021DEC, SHIPPED_EMBEDDING_MODEL,
};
use soos_inference_ort::{EmbeddingExtractor, InferenceError, OrtEmbeddingExtractor, TensorLayout};

#[path = "../../../tests/fixtures/embedding_onnx.rs"]
mod embedding_onnx;

fn extractor_from(model: &[u8]) -> OrtEmbeddingExtractor {
    let session = ort::session::Session::builder()
        .expect("ORT session builder")
        .commit_from_memory(model)
        .expect("embedding fixture ONNX model must load");
    OrtEmbeddingExtractor::new(Arc::new(Mutex::new(session)))
}

/// A non-uniform 112x112 RGB crop (non-zero after normalization, so L2 normalization succeeds).
fn crop() -> Vec<u8> {
    (0..112usize * 112 * 3)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect()
}

#[test]
fn test_embedding_dimension_constant_is_128() {
    assert_eq!(EMBEDDING_DIMENSION, 128);
}

/// SFC2: the shipped embedding model is SFace 2021dec (id, 128-D, NCHW), the single source of
/// the id and dimension used by every binary (GitHub #278).
#[test]
fn test_shipped_embedding_spec_is_sface() {
    assert_eq!(SHIPPED_EMBEDDING_MODEL, SFACE_2021DEC);
    assert_eq!(
        SFACE_2021DEC,
        EmbeddingModelSpec {
            model_id: "sface_2021dec",
            dimension: 128,
            input_layout: TensorLayout::Nchw,
        }
    );
    assert_eq!(EMBEDDING_DIMENSION, SHIPPED_EMBEDDING_MODEL.dimension);
    let extractor = extractor_from(&embedding_onnx::nchw_embedding_model(128));
    assert_eq!(extractor.spec(), SHIPPED_EMBEDDING_MODEL);
    assert_eq!(extractor.output_dimension(), Some(128));
}

#[test]
fn test_extract_embedding_rejects_nhwc_session() {
    let extractor = extractor_from(&embedding_onnx::nhwc_embedding_model(128));
    assert!(!extractor.is_nhwc());
    assert_eq!(extractor.input_layout(), None);
    match extractor.extract_embedding(&crop(), 112, 112) {
        Err(InferenceError::TensorError(_)) => {}
        other => panic!("an NHWC session must fail closed for the NCHW SFace spec, got {other:?}"),
    }
}

#[test]
fn test_extract_embedding_accepts_128d_nchw_output() {
    let extractor = extractor_from(&embedding_onnx::nchw_embedding_model(128));
    assert!(!extractor.is_nhwc());
    assert_eq!(extractor.input_layout(), Some(TensorLayout::Nchw));
    let embedding = extractor
        .extract_embedding(&crop(), 112, 112)
        .expect("a 128D NCHW output must be accepted");
    assert_eq!(embedding.len(), 128);
    assert!(embedding.is_normalized(1e-4));
}

#[test]
fn test_extract_embedding_rejects_wrong_dimension() {
    for dim in [3u64, 127, 129, 512] {
        let extractor = extractor_from(&embedding_onnx::nchw_embedding_model(dim));
        match extractor.extract_embedding(&crop(), 112, 112) {
            Err(InferenceError::DimensionMismatch { expected, actual }) => {
                assert_eq!(expected, 128, "dim {dim}");
                assert_eq!(actual, usize::try_from(dim).unwrap(), "dim {dim}");
            }
            other => panic!("a {dim}D output must be rejected, got {other:?}"),
        }
    }
}

#[test]
fn test_nhwc_detection_fails_closed_without_inputs() {
    let extractor = extractor_from(&embedding_onnx::inputless_embedding_model());
    assert_eq!(extractor.input_layout(), None);
    assert!(!extractor.is_nhwc());
    match extractor.extract_embedding(&crop(), 112, 112) {
        Err(InferenceError::TensorError(_)) => {}
        other => panic!("an input-less session must fail closed, got {other:?}"),
    }
}

#[test]
fn test_embedding_layout_detection_fails_closed_on_ambiguous_input() {
    let extractor = extractor_from(&embedding_onnx::ambiguous_layout_embedding_model());
    assert_eq!(extractor.input_layout(), None);
    match extractor.extract_embedding(&crop(), 112, 112) {
        Err(InferenceError::TensorError(_)) => {}
        other => panic!("a rank-2 input must fail closed, got {other:?}"),
    }
}

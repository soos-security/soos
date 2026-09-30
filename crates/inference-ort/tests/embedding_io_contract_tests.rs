//! Embedding extractor I/O contract (GitHub #268, review finding VIS-14).
//!
//! - The extractor must reject a model output whose length is not the attested 512 dimensions
//!   instead of emitting a vector that only fails later in the matcher.
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

use soos_inference_ort::embedding::EMBEDDING_DIMENSION;
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
fn test_embedding_dimension_constant_is_512() {
    assert_eq!(EMBEDDING_DIMENSION, 512);
}

#[test]
fn test_extract_embedding_accepts_512d_nhwc_output() {
    let extractor = extractor_from(&embedding_onnx::nhwc_embedding_model(512));
    assert!(extractor.is_nhwc());
    assert_eq!(extractor.input_layout(), Some(TensorLayout::Nhwc));
    let embedding = extractor
        .extract_embedding(&crop(), 112, 112)
        .expect("a 512D output must be accepted");
    assert_eq!(embedding.len(), 512);
    assert!(embedding.is_normalized(1e-4));
}

#[test]
fn test_extract_embedding_accepts_512d_nchw_output() {
    let extractor = extractor_from(&embedding_onnx::nchw_embedding_model(512));
    assert!(!extractor.is_nhwc());
    assert_eq!(extractor.input_layout(), Some(TensorLayout::Nchw));
    let embedding = extractor
        .extract_embedding(&crop(), 112, 112)
        .expect("a 512D NCHW output must be accepted");
    assert_eq!(embedding.len(), 512);
}

#[test]
fn test_extract_embedding_rejects_wrong_dimension() {
    for dim in [3u64, 128, 511, 513] {
        let extractor = extractor_from(&embedding_onnx::nhwc_embedding_model(dim));
        match extractor.extract_embedding(&crop(), 112, 112) {
            Err(InferenceError::DimensionMismatch { expected, actual }) => {
                assert_eq!(expected, 512, "dim {dim}");
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

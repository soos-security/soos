//! GitHub #298 (row SGF3): `OrtEmbeddingExtractor::with_spec` rejects, at construction, a spec
//! whose input layout is not NCHW, because `extract_embedding` always feeds an NCHW
//! `[1, 3, 112, 112]` tensor. Before this change such a spec was accepted and only failed
//! closed at run time.
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

use soos_inference_ort::embedding::{EmbeddingModelSpec, SFACE_2021DEC, SHIPPED_EMBEDDING_MODEL};
use soos_inference_ort::{InferenceError, OrtEmbeddingExtractor, TensorLayout};

#[path = "../../../tests/fixtures/embedding_onnx.rs"]
mod embedding_onnx;

fn session_from(model: &[u8]) -> Arc<Mutex<ort::session::Session>> {
    let session = ort::session::Session::builder()
        .expect("ORT session builder")
        .commit_from_memory(model)
        .expect("embedding fixture ONNX model must load");
    Arc::new(Mutex::new(session))
}

const NHWC_SPEC: EmbeddingModelSpec = EmbeddingModelSpec {
    input_layout: TensorLayout::Nhwc,
    ..SFACE_2021DEC
};

fn assert_rejected(result: Result<OrtEmbeddingExtractor, InferenceError>) {
    match result {
        Err(InferenceError::InvalidInput(message)) => {
            assert!(message.contains("NCHW"), "{message}");
            assert!(message.contains("sface_2021dec"), "{message}");
        }
        Err(other) => panic!("expected InvalidInput for a non-NCHW spec, got {other:?}"),
        Ok(extractor) => panic!(
            "a non-NCHW spec must be rejected at construction, got layout {:?}",
            extractor.input_layout()
        ),
    }
}

/// SGF3: an NHWC spec is rejected even when the session itself is NCHW.
#[test]
fn test_with_spec_rejects_nhwc_spec_for_nchw_session() {
    let session = session_from(&embedding_onnx::nchw_embedding_model(128));
    assert_rejected(OrtEmbeddingExtractor::with_spec(session, NHWC_SPEC));
}

/// SGF3: an NHWC spec is rejected for a matching NHWC session too (the extractor never feeds
/// channels-last tensors, so the pair would only fail at run time).
#[test]
fn test_with_spec_rejects_nhwc_spec_for_nhwc_session() {
    let session = session_from(&embedding_onnx::nhwc_embedding_model(128));
    assert_rejected(OrtEmbeddingExtractor::with_spec(session, NHWC_SPEC));
}

/// SGF3: the NCHW shipped spec is still accepted and keeps the inferred NCHW layout.
#[test]
fn test_with_spec_accepts_nchw_shipped_spec() {
    let session = session_from(&embedding_onnx::nchw_embedding_model(128));
    let extractor = OrtEmbeddingExtractor::with_spec(session, SHIPPED_EMBEDDING_MODEL)
        .expect("the NCHW shipped spec is accepted");
    assert_eq!(extractor.spec(), SHIPPED_EMBEDDING_MODEL);
    assert_eq!(extractor.input_layout(), Some(TensorLayout::Nchw));
}

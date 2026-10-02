//! GitHub #313 (VIS-NEW-2 to VIS-NEW-6): inference findings of the 2026-10-02 review
//! (matrix rows GCV8 to GCV12).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses direct assertions"
)]

use std::path::PathBuf;

use soos_inference_ort::{
    BiometricEmbedding, InferenceError, ModelManifest, OrtEmbeddingExtractor, TensorLayout,
};

/// GCV8 (VIS-NEW-2): a zero-sized crop is refused with `InvalidDimensions` (it used to
/// underflow `height - 1` while resizing).
#[test]
fn test_gcv_embedding_prepare_input_rejects_zero_dimensions() {
    for (w, h) in [(0u32, 112u32), (112, 0), (0, 0)] {
        let result = std::panic::catch_unwind(|| OrtEmbeddingExtractor::prepare_input(&[], w, h));
        let result = result.expect("prepare_input must not panic on a zero dimension");
        assert!(
            matches!(
                result,
                Err(InferenceError::InvalidDimensions {
                    expected: (112, 112),
                    actual,
                }) if actual == (w, h)
            ),
            "{w}x{h}: {result:?}"
        );
    }
}

/// GCV9 (VIS-NEW-3): a non-finite model output is never normalized into an embedding.
#[test]
fn test_gcv_embedding_normalize_rejects_non_finite_vectors() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut embedding = BiometricEmbedding::new(vec![0.5, bad, 0.25, 0.1]);
        assert!(
            embedding.normalize().is_err(),
            "{bad} must be rejected, got {:?}",
            embedding.as_slice().len()
        );
    }
    // Finite components whose squares overflow to an infinite norm are rejected too.
    let mut overflow = BiometricEmbedding::new(vec![f32::MAX, f32::MAX]);
    assert!(overflow.normalize().is_err());

    let mut fine = BiometricEmbedding::new(vec![3.0, 4.0]);
    fine.normalize().expect("finite vector normalizes");
    assert!((fine.as_slice()[0] - 0.6).abs() < 1e-6);
}

/// GCV10 (VIS-NEW-4): every entry of the shipped manifest declares its input layout, so the
/// registry asserts the layout of every model and never logs the legacy-manifest warning.
#[test]
fn test_gcv_shipped_manifest_declares_every_input_layout() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    let manifest = ModelManifest::from_file(path).expect("committed manifest parses");
    assert!(!manifest.models.is_empty());
    for (id, meta) in &manifest.models {
        assert!(meta.input_layout_declared, "{id} must declare input_layout");
        assert_eq!(
            meta.input_layout,
            TensorLayout::Nchw,
            "{id} is an NCHW graph"
        );
    }
}

/// GCV12 (VIS-NEW-6): `Debug` of an embedding prints its dimension, never its values.
#[test]
fn test_gcv_biometric_embedding_debug_is_redacted() {
    let embedding = BiometricEmbedding::new(vec![0.123_456_7, -0.765_432_1, 0.5]);
    let text = format!("{embedding:?}");
    assert!(!text.contains("0.123"), "{text}");
    assert!(!text.contains("0.765"), "{text}");
    assert!(text.contains("BiometricEmbedding"), "{text}");
    assert!(text.contains('3'), "the dimension is shown: {text}");
}

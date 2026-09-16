//! Tests for memory zeroization of biometric embeddings.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Test suite assertions"
)]

use soos_inference_ort::BiometricEmbedding;
use zeroize::Zeroize;

#[test]
fn test_biometric_embedding_zeroize_trait() {
    let raw = vec![0.5_f32; 128];
    let mut embedding = BiometricEmbedding::new(raw);
    assert_eq!(embedding.as_slice()[0], 0.5);

    embedding.zeroize();
    for (i, &val) in embedding.as_slice().iter().enumerate() {
        assert_eq!(
            val.to_bits(),
            0,
            "Embedding element at index {i} was not zeroed: {val}"
        );
    }
}

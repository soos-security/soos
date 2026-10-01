//! `BiometricEmbedding::to_vec` returns a wipe-on-drop copy (GitHub #287, matrix row CVF6).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Contractual test suite uses assertions and unwrap"
)]

use soos_inference_ort::BiometricEmbedding;
use zeroize::Zeroizing;

#[test]
fn test_cvf_embedding_to_vec_copy_is_zeroizing() {
    let embedding = BiometricEmbedding::new(vec![0.6, 0.8, 0.0]);
    // The type annotation is the contract: the copy is a `Zeroizing` container, so the
    // duplicated biometric floats are wiped when the caller drops it.
    let copy: Zeroizing<Vec<f32>> = embedding.to_vec();
    assert_eq!(copy.as_slice(), embedding.as_slice());
    assert_eq!(copy.len(), 3);

    // The copy is independent of the embedding it was taken from.
    drop(embedding);
    assert_eq!(*copy, vec![0.6, 0.8, 0.0]);
}

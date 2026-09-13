//! Tests for cosine similarity biometric matching in `soos-vision`.
//! Acceptance criteria: V3 — Cosine similarity correctness verified with precomputed values.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_inference_ort::BiometricEmbedding;
use soos_vision::{cosine_similarity, match_embeddings, VisionError};

#[test]
fn test_cosine_similarity_identical_vectors() {
    let a = [0.6_f32, 0.8_f32];
    let score = cosine_similarity(&a, &a).expect("Identical vector similarity");
    assert!((score - 1.0).abs() < 1e-5, "Expected 1.0, got {}", score);
}

#[test]
fn test_cosine_similarity_orthogonal_vectors() {
    let a = [1.0_f32, 0.0_f32];
    let b = [0.0_f32, 1.0_f32];
    let score = cosine_similarity(&a, &b).expect("Orthogonal vector similarity");
    assert!(score.abs() < 1e-5, "Expected 0.0, got {}", score);
}

#[test]
fn test_cosine_similarity_opposite_vectors() {
    let a = [0.6_f32, 0.8_f32];
    let b = [-0.6_f32, -0.8_f32];
    let score = cosine_similarity(&a, &b).expect("Opposite vector similarity");
    assert!(
        (score - (-1.0)).abs() < 1e-5,
        "Expected -1.0, got {}",
        score
    );
}

#[test]
fn test_cosine_similarity_known_precomputed_vectors() {
    // a = [3, 4], b = [4, 3] -> dot = 12 + 12 = 24, norm(a)=5, norm(b)=5 -> 24/25 = 0.96
    let a = [3.0_f32, 4.0_f32];
    let b = [4.0_f32, 3.0_f32];
    let score = cosine_similarity(&a, &b).expect("Precomputed vector similarity");
    assert!((score - 0.96).abs() < 1e-5, "Expected 0.96, got {}", score);
}

#[test]
fn test_cosine_similarity_dimension_mismatch_fails() {
    let a = [1.0_f32, 2.0_f32];
    let b = [1.0_f32, 2.0_f32, 3.0_f32];
    let result = cosine_similarity(&a, &b);
    assert!(matches!(
        result,
        Err(VisionError::DimensionMismatch {
            expected: 2,
            actual: 3
        })
    ));
}

#[test]
fn test_cosine_similarity_zero_norm_fails() {
    let a = [0.0_f32, 0.0_f32];
    let b = [1.0_f32, 1.0_f32];
    let result = cosine_similarity(&a, &b);
    assert!(matches!(result, Err(VisionError::DegenerateEmbedding(_))));
}

#[test]
fn test_match_embeddings_decision_threshold() {
    // Two 128D unit vectors with high similarity
    let vec_a = vec![0.08838834_f32; 128];
    let mut vec_b = vec_a.clone();
    vec_b[0] += 0.02; // Minor variation

    let mut emb_a = BiometricEmbedding::new(vec_a);
    emb_a.normalize().expect("Normalize A");
    let mut emb_b = BiometricEmbedding::new(vec_b);
    emb_b.normalize().expect("Normalize B");

    // Threshold 0.45: should match
    let match_res = match_embeddings(&emb_a, &emb_b, 0.45).expect("Match comparison");
    assert!(
        match_res.matched,
        "Score {} should meet threshold 0.45",
        match_res.score
    );
    assert!(match_res.score > 0.95);

    // Strict threshold 0.9999: should not match
    let strict_res = match_embeddings(&emb_a, &emb_b, 0.9999).expect("Strict match comparison");
    assert!(
        !strict_res.matched,
        "Score {} should not meet strict threshold 0.9999",
        strict_res.score
    );
}

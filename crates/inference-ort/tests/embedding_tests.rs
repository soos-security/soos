//! Contractual test suite for BiometricEmbedding and EmbeddingExtractor.
//!
//! Enforces Verification Matrix Criterion V2: L2-normalized embeddings (norm ≈ 1.0).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_inference_ort::embedding::{BiometricEmbedding, EmbeddingExtractor};
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::mock::MockEmbeddingExtractor;

#[test]
fn test_l2_norm_and_normalization_criterion_v2() {
    // 3-4-0 vector has norm 5.0
    let mut raw = BiometricEmbedding::new(vec![3.0, 4.0, 0.0]);
    assert!((raw.l2_norm() - 5.0).abs() < 1e-6);
    assert!(!raw.is_normalized(1e-4));

    // Normalize
    raw.normalize().expect("normalization failed");

    // Verification Matrix V2: norm ≈ 1.0
    let norm = raw.l2_norm();
    assert!(
        (norm - 1.0).abs() < 1e-6,
        "Criterion V2 invariant failed: norm is {}",
        norm
    );
    assert!(raw.is_normalized(1e-5));
    assert_eq!(raw.as_slice(), &[0.6, 0.8, 0.0]);
}

#[test]
fn test_normalization_zero_vector_fails_closed() {
    let mut zero_vec = BiometricEmbedding::new(vec![0.0, 0.0, 0.0]);
    let err = zero_vec
        .normalize()
        .expect_err("zero vector must fail normalization");

    match err {
        InferenceError::EmbeddingFailed(msg) => {
            assert!(msg.contains("zero"));
        }
        other => panic!("Expected EmbeddingFailed, got: {:?}", other),
    }
}

#[test]
fn test_cosine_similarity_properties() {
    // Identity: cosine(v, v) == 1.0
    let v1 = BiometricEmbedding::new(vec![0.6, 0.8, 0.0]);
    let sim_self = v1.cosine_similarity(&v1).expect("self similarity failed");
    assert!((sim_self - 1.0).abs() < 1e-6);

    // Opposite: cosine(v, -v) == -1.0
    let v_opp = BiometricEmbedding::new(vec![-0.6, -0.8, 0.0]);
    let sim_opp = v1
        .cosine_similarity(&v_opp)
        .expect("opposite similarity failed");
    assert!((sim_opp - (-1.0)).abs() < 1e-6);

    // Orthogonal: cosine(v1, v_ortho) == 0.0
    let v_ortho = BiometricEmbedding::new(vec![0.0, 0.0, 1.0]);
    let sim_ortho = v1
        .cosine_similarity(&v_ortho)
        .expect("orthogonal similarity failed");
    assert!(sim_ortho.abs() < 1e-6);

    // Dimension mismatch
    let v_mismatch = BiometricEmbedding::new(vec![0.6, 0.8]);
    let err = v1
        .cosine_similarity(&v_mismatch)
        .expect_err("dimension mismatch must fail");

    match err {
        InferenceError::DimensionMismatch { expected, actual } => {
            assert_eq!(expected, 3);
            assert_eq!(actual, 2);
        }
        other => panic!("Expected DimensionMismatch, got: {:?}", other),
    }
}

#[test]
fn test_mock_embedding_extractor_criterion_v2() {
    // 128D embedding extractor (MobileFaceNet standard)
    let extractor_128 = MockEmbeddingExtractor::new(128);
    let dummy_112x112 = vec![120u8; 112 * 112 * 3];

    let emb1 = extractor_128
        .extract_embedding(&dummy_112x112, 112, 112)
        .expect("extraction failed");

    assert_eq!(emb1.len(), 128);

    // Assert Criterion V2: L2 norm ≈ 1.0
    let norm1 = emb1.l2_norm();
    assert!(
        (norm1 - 1.0).abs() < 1e-5,
        "Criterion V2 invariant failed: norm is {}",
        norm1
    );

    // 512D embedding extractor
    let extractor_512 = MockEmbeddingExtractor::new(512);
    let emb2 = extractor_512
        .extract_embedding(&dummy_112x112, 112, 112)
        .expect("extraction failed");

    assert_eq!(emb2.len(), 512);
    let norm2 = emb2.l2_norm();
    assert!(
        (norm2 - 1.0).abs() < 1e-5,
        "Criterion V2 invariant failed: norm is {}",
        norm2
    );

    // Rejection of invalid crop dimensions
    let invalid_crop = vec![120u8; 100];
    let err = extractor_128
        .extract_embedding(&invalid_crop, 112, 112)
        .expect_err("truncated crop must fail");

    match err {
        InferenceError::InvalidBufferSize { expected, actual } => {
            assert_eq!(expected, 112 * 112 * 3);
            assert_eq!(actual, 100);
        }
        other => panic!("Expected InvalidBufferSize, got: {:?}", other),
    }
}

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

use soos_inference_ort::embedding::{
    BiometricEmbedding, EmbeddingExtractor, OrtEmbeddingExtractor, EMBEDDING_DIMENSION,
};
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

#[test]
fn test_embedding_input_is_raw_0_255() {
    // Test input with pixel values 0, 127, 128, and 255
    let width = 112u32;
    let height = 112u32;
    let mut pixels = vec![0u8; (width * height * 3) as usize];
    // First pixel: R=0, G=127, B=255
    pixels[0] = 0;
    pixels[1] = 127;
    pixels[2] = 255;

    // Last pixel: R=255, G=128, B=0
    let last_idx = ((width * height * 3) - 3) as usize;
    pixels[last_idx] = 255;
    pixels[last_idx + 1] = 128;
    pixels[last_idx + 2] = 0;

    let input_tensor = OrtEmbeddingExtractor::prepare_input(&pixels, width, height)
        .expect("prepare_input must succeed");

    // SFace RGB NCHW layout, raw values (the graph applies (x - 127.5) / 128 itself).
    let plane = (width * height) as usize;
    let last = plane - 1;
    // Pixel 0: R=0 on plane 0, G=127 on plane 1, B=255 on plane 2.
    assert!(
        input_tensor[0].abs() < 1e-6,
        "R=0 must stay 0.0, got {}",
        input_tensor[0]
    );
    assert!(
        (input_tensor[plane] - 127.0).abs() < 1e-6,
        "G=127 must stay 127.0, got {}",
        input_tensor[plane]
    );
    assert!(
        (input_tensor[2 * plane] - 255.0).abs() < 1e-6,
        "B=255 must stay 255.0, got {}",
        input_tensor[2 * plane]
    );
    // Last pixel: R=255, G=128, B=0.
    assert!((input_tensor[last] - 255.0).abs() < 1e-6);
    assert!((input_tensor[plane + last] - 128.0).abs() < 1e-6);
    assert!(input_tensor[2 * plane + last].abs() < 1e-6);
}

#[test]
fn test_mock_embedding_default_matches_embedding_dimension() {
    let mock_default = MockEmbeddingExtractor::default();
    assert_eq!(mock_default.dim(), EMBEDDING_DIMENSION);
    assert_eq!(EMBEDDING_DIMENSION, 128);

    let dummy_112x112 = vec![120u8; 112 * 112 * 3];
    let emb = mock_default
        .extract_embedding(&dummy_112x112, 112, 112)
        .expect("extraction failed");

    assert_eq!(
        emb.len(),
        128,
        "Mock embedding extractor must produce 128D vectors by default like the shipped SFace model"
    );

    let norm = emb.l2_norm();
    assert!(
        (norm - 1.0).abs() < 1e-5,
        "Criterion V2 invariant failed: norm is {}",
        norm
    );

    let mock_new_default = MockEmbeddingExtractor::new_default();
    assert_eq!(mock_new_default.dim(), 128);
    let emb_new_default = mock_new_default
        .extract_embedding(&dummy_112x112, 112, 112)
        .expect("extraction failed");
    assert_eq!(emb_new_default.len(), 128);
}

#[test]
fn test_prepare_input_writes_rgb_nchw_planes() {
    let width = 112u32;
    let height = 112u32;
    let mut pixels = vec![0u8; (width * height * 3) as usize];
    // First pixel: R=0, G=127, B=255
    pixels[0] = 0;
    pixels[1] = 127;
    pixels[2] = 255;
    // Last pixel: R=10, G=20, B=30
    let last_idx = ((width * height * 3) - 3) as usize;
    pixels[last_idx] = 10;
    pixels[last_idx + 1] = 20;
    pixels[last_idx + 2] = 30;

    // SFace: NCHW planes in R, G, B order (R=0, G=1, B=2), raw values.
    let nchw = OrtEmbeddingExtractor::prepare_input(&pixels, width, height)
        .expect("prepare_input must succeed");
    assert_eq!(nchw.len(), 3 * 112 * 112);
    assert!(nchw[0].abs() < 1e-6, "Plane 0 must be R (value 0.0)");
    assert!(
        (nchw[112 * 112] - 127.0).abs() < 1e-6,
        "Plane 1 must be G (value 127.0)"
    );
    assert!(
        (nchw[2 * 112 * 112] - 255.0).abs() < 1e-6,
        "Plane 2 must be B (value 255.0)"
    );
    let last = 112 * 112 - 1;
    assert!((nchw[last] - 10.0).abs() < 1e-6, "last pixel R");
    assert!((nchw[112 * 112 + last] - 20.0).abs() < 1e-6, "last pixel G");
    assert!(
        (nchw[2 * 112 * 112 + last] - 30.0).abs() < 1e-6,
        "last pixel B"
    );
}

#[test]
fn test_sface_input_rgb_ordering() {
    let width = 112u32;
    let height = 112u32;
    // Pure Red image (R=255, G=0, B=0)
    let red_pixels = [255, 0, 0].repeat(112 * 112);

    let tensor = OrtEmbeddingExtractor::prepare_input(&red_pixels, width, height)
        .expect("prepare_input must succeed");

    // In RGB NCHW raw layout:
    // Plane 0 (R) should be 255.0, planes 1 (G) and 2 (B) should be 0.0
    assert!(
        (tensor[0] - 255.0).abs() < 1e-5,
        "RGB plane 0 (R) must carry the raw red value 255.0"
    );
    assert!(
        tensor[112 * 112].abs() < 1e-5,
        "RGB plane 1 (G) must be 0.0"
    );
    assert!(
        tensor[2 * 112 * 112].abs() < 1e-5,
        "RGB plane 2 (B) must be 0.0"
    );
}

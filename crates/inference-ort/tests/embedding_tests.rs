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
    BiometricEmbedding, EmbeddingExtractor, OrtEmbeddingExtractor,
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
fn test_embedding_normalization_symmetric_range() {
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

    // Under BGR NCHW layout:
    // Pixel 0 has R=0, G=127, B=255.
    // Channel 0 is B (value 255): (255.0 - 127.5) / 127.5 == +1.0
    let norm_255 = input_tensor[0];
    assert!(
        (norm_255 - 1.0).abs() < 1e-6,
        "Pixel value 255 must normalize to +1.0, got: {norm_255}"
    );

    // Channel 2 is R (value 0): (0.0 - 127.5) / 127.5 == -1.0
    let norm_0 = input_tensor[2 * (width * height) as usize];
    assert!(
        (norm_0 - (-1.0)).abs() < 1e-6,
        "Pixel value 0 must normalize to -1.0, got: {norm_0}"
    );

    // Pixel value 127: (127.0 - 127.5) / 127.5 = -0.5 / 127.5
    // Pixel value 128: (128.0 - 127.5) / 127.5 = +0.5 / 127.5
    let norm_127 = input_tensor[(width * height) as usize]; // G channel for pixel 0
    let norm_128 = input_tensor[(width * height) as usize + (width * height - 1) as usize]; // G channel for last pixel
    assert!(
        (norm_127 + norm_128).abs() < 1e-6,
        "Values around 127.5 must be anti-symmetric, got norm_127={norm_127}, norm_128={norm_128}"
    );
}

#[test]
fn test_mock_embedding_default_512d() {
    let mock_default = MockEmbeddingExtractor::default();
    assert_eq!(mock_default.dim(), 512);

    let dummy_112x112 = vec![120u8; 112 * 112 * 3];
    let emb = mock_default
        .extract_embedding(&dummy_112x112, 112, 112)
        .expect("extraction failed");

    assert_eq!(
        emb.len(),
        512,
        "Mock embedding extractor must produce 512D vectors by default for ArcFace w600k"
    );

    let norm = emb.l2_norm();
    assert!(
        (norm - 1.0).abs() < 1e-5,
        "Criterion V2 invariant failed: norm is {}",
        norm
    );

    let mock_new_default = MockEmbeddingExtractor::new_default();
    assert_eq!(mock_new_default.dim(), 512);
    let emb_new_default = mock_new_default
        .extract_embedding(&dummy_112x112, 112, 112)
        .expect("extraction failed");
    assert_eq!(emb_new_default.len(), 512);
}

#[test]
fn test_prepare_input_layout_nhwc_and_nchw() {
    let width = 112u32;
    let height = 112u32;
    let mut pixels = vec![0u8; (width * height * 3) as usize];
    // First pixel: R=0, G=127, B=255
    pixels[0] = 0;
    pixels[1] = 127;
    pixels[2] = 255;

    // Test NCHW layout: ArcFace w600k expects BGR channel ordering (B=0, G=1, R=2)
    let nchw = OrtEmbeddingExtractor::prepare_input_layout(&pixels, width, height, false)
        .expect("prepare_input_layout NCHW must succeed");
    assert_eq!(nchw.len(), 3 * 112 * 112);
    assert!(
        (nchw[0] - 1.0).abs() < 1e-6,
        "Channel 0 must be B (value 1.0)"
    );
    assert!(
        (nchw[112 * 112] - ((127.0 - 127.5) / 127.5)).abs() < 1e-6,
        "Channel 1 must be G"
    );
    assert!(
        (nchw[2 * 112 * 112] - (-1.0)).abs() < 1e-6,
        "Channel 2 must be R (value -1.0)"
    );

    // Test NHWC layout: Pixel layout must be [B, G, R]
    let nhwc = OrtEmbeddingExtractor::prepare_input_layout(&pixels, width, height, true)
        .expect("prepare_input_layout NHWC must succeed");
    assert_eq!(nhwc.len(), 3 * 112 * 112);
    assert!(
        (nhwc[0] - 1.0).abs() < 1e-6,
        "Index 0 must be B (value 1.0)"
    );
    assert!(
        (nhwc[1] - ((127.0 - 127.5) / 127.5)).abs() < 1e-6,
        "Index 1 must be G"
    );
    assert!(
        (nhwc[2] - (-1.0)).abs() < 1e-6,
        "Index 2 must be R (value -1.0)"
    );
}

#[test]
fn test_arcface_input_bgr_ordering() {
    let width = 112u32;
    let height = 112u32;
    // Pure Red image (R=255, G=0, B=0)
    let red_pixels = [255, 0, 0].repeat(112 * 112);

    let tensor = OrtEmbeddingExtractor::prepare_input_layout(&red_pixels, width, height, false)
        .expect("prepare_input_layout must succeed");

    // In BGR NCHW layout:
    // Channel 0 (B) should be -1.0 ((0 - 127.5)/127.5)
    // Channel 1 (G) should be -1.0 ((0 - 127.5)/127.5)
    // Channel 2 (R) should be +1.0 ((255 - 127.5)/127.5)
    assert!(
        (tensor[0] - (-1.0)).abs() < 1e-5,
        "BGR Channel 0 (B) must be normalized from 0 to -1.0"
    );
    assert!(
        (tensor[112 * 112] - (-1.0)).abs() < 1e-5,
        "BGR Channel 1 (G) must be normalized from 0 to -1.0"
    );
    assert!(
        (tensor[2 * 112 * 112] - 1.0).abs() < 1e-5,
        "BGR Channel 2 (R) must be normalized from 255 to +1.0"
    );
}

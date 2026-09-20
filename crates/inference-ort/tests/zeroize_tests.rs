//! Tests for memory zeroization of biometric embeddings and inference input buffers.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Test suite assertions"
)]

use soos_inference_ort::{
    BiometricEmbedding, OrtEmbeddingExtractor, OrtFaceDetector, OrtPadDetector, OrtScrfdDetector,
};
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

#[test]
fn test_inference_input_buffers_zeroized() {
    // 1. OrtFaceDetector input buffer
    let dummy_rgb = vec![200u8; 320 * 240 * 3];
    let mut detector_buf =
        OrtFaceDetector::prepare_input(&dummy_rgb, 320, 240).expect("detector input");
    assert_eq!(detector_buf.len(), 3 * 240 * 320);
    assert!(
        detector_buf.iter().any(|&v| v != 0.0),
        "Detector buffer must contain non-zero normalized pixels"
    );

    detector_buf.zeroize();
    for (i, &val) in detector_buf.iter().enumerate() {
        assert_eq!(
            val.to_bits(),
            0,
            "Detector buffer element at index {i} was not zeroed: {val}"
        );
    }

    // 2. OrtEmbeddingExtractor input buffer
    let dummy_crop = vec![180u8; 112 * 112 * 3];
    let mut extractor_buf =
        OrtEmbeddingExtractor::prepare_input(&dummy_crop, 112, 112).expect("extractor input");
    assert_eq!(extractor_buf.len(), 3 * 112 * 112);
    assert!(
        extractor_buf.iter().any(|&v| v != 0.0),
        "Extractor buffer must contain non-zero normalized pixels"
    );

    extractor_buf.zeroize();
    for (i, &val) in extractor_buf.iter().enumerate() {
        assert_eq!(
            val.to_bits(),
            0,
            "Extractor buffer element at index {i} was not zeroed: {val}"
        );
    }

    // 3. OrtScrfdDetector input buffer
    let mut scrfd_buf = OrtScrfdDetector::prepare_input(&dummy_rgb, 320, 240).expect("scrfd input");
    assert_eq!(scrfd_buf.len(), 3 * 640 * 640);
    assert!(
        scrfd_buf.iter().any(|&v| v != 0.0),
        "SCRFD buffer must contain non-zero normalized pixels"
    );

    scrfd_buf.zeroize();
    for (i, &val) in scrfd_buf.iter().enumerate() {
        assert_eq!(
            val.to_bits(),
            0,
            "SCRFD buffer element at index {i} was not zeroed: {val}"
        );
    }

    // 4. OrtPadDetector input buffer (MiniFASNetV2 80x80)
    let mut pad_buf = OrtPadDetector::prepare_input(&dummy_crop, 112, 112).expect("pad input");
    assert_eq!(pad_buf.len(), 3 * 80 * 80);
    assert!(
        pad_buf.iter().any(|&v| v != 0.0),
        "PAD buffer must contain non-zero normalized pixels"
    );

    pad_buf.zeroize();
    for (i, &val) in pad_buf.iter().enumerate() {
        assert_eq!(
            val.to_bits(),
            0,
            "PAD buffer element at index {i} was not zeroed: {val}"
        );
    }
}

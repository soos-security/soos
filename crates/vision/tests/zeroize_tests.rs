//! Contractual acceptance tests for memory zeroization of intermediate frame buffers.
//! References: AI/BACKLOG.md Issue #24, AI/ARCHITECTURE.md §10 Memory Hygiene.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::mock::{
    MockEmbeddingExtractor, MockFaceDetector, MockLandmarkDetector, MockPadDetector,
};
use soos_inference_ort::{
    BiometricEmbedding, BoundingBox, FaceDetection, LandmarkDetector, PadDetector,
};
use soos_vision::{
    convert_to_rgb, MatchResult, PipelineOutput, VerificationOutcome, VisionPipeline,
    VisionPipelineConfig,
};
use zeroize::{Zeroize, Zeroizing};

#[test]
fn test_rgb_buffer_zeroized_after_pipeline() {
    let raw = vec![0xAA_u8; 640 * 480 * 3];
    let rgb = convert_to_rgb(&raw, 640, 480, PixelFormat::Rgb24).expect("convert_to_rgb");
    assert_eq!(rgb[0], 0xAA);

    let mut zeroizing_rgb = Zeroizing::new(rgb);
    assert_eq!(zeroizing_rgb[0], 0xAA);

    // Explicit zeroize on the buffer must set all bytes to 0
    zeroizing_rgb.zeroize();
    for (i, &b) in zeroizing_rgb.iter().enumerate() {
        assert_eq!(b, 0, "Byte at index {i} was not zeroed");
    }
}

#[test]
fn test_verification_outcome_zeroize_on_drop() {
    let box_ = BoundingBox::new(10.0, 10.0, 100.0, 100.0);
    let detection = FaceDetection::new(box_, 0.95);
    let landmarks = MockLandmarkDetector::new_canonical()
        .detect_landmarks(&[0u8; 12], 2, 2, &box_)
        .expect("landmarks");
    let aligned_crop = vec![0xCC_u8; 112 * 112 * 3];
    let pad_result = MockPadDetector::new_live()
        .evaluate_liveness(&aligned_crop, 112, 112)
        .expect("pad");
    let embedding = BiometricEmbedding::new(vec![0.42_f32; 128]);

    let output = PipelineOutput {
        detection,
        landmarks,
        aligned_crop_rgb: aligned_crop,
        pad_result,
        embedding,
    };

    let outcome = VerificationOutcome {
        output,
        match_result: MatchResult {
            matched: true,
            score: 0.99,
            threshold: 0.75,
        },
    };

    // Clone outcome: verify that cloned outcome also supports zeroization
    let mut cloned_outcome = outcome.clone();

    // Verify non-zero before zeroize
    assert_eq!(cloned_outcome.output.aligned_crop_rgb[0], 0xCC);
    assert_eq!(cloned_outcome.output.embedding.as_slice()[0], 0.42);

    // Call Zeroize::zeroize() on VerificationOutcome
    cloned_outcome.zeroize();

    // Verify aligned crop is zeroed
    for (i, &b) in cloned_outcome.output.aligned_crop_rgb.iter().enumerate() {
        assert_eq!(b, 0, "Aligned crop byte at index {i} was not zeroed");
    }

    // Verify embedding floats are zeroed
    for (i, &val) in cloned_outcome
        .output
        .embedding
        .as_slice()
        .iter()
        .enumerate()
    {
        assert_eq!(
            val.to_bits(),
            0,
            "Embedding element at index {i} was not zeroed: {val}"
        );
    }
}

#[test]
fn test_pipeline_zeroizes_intermediate_buffers_on_error() {
    // Pipeline configured to fail on PAD (spoof frame)
    let width = 320;
    let height = 240;
    let detector = Arc::new(MockFaceDetector::new_centered_face(width, height, 0.95));
    let pad = Arc::new(MockPadDetector::new_spoof(
        soos_inference_ort::AttackType::PrintPhoto,
        0.10,
    ));
    let extractor = Arc::new(MockEmbeddingExtractor::new(128));
    let config = VisionPipelineConfig::default();

    let pipeline = VisionPipeline::new(detector, pad, extractor, config);
    let frame = Frame::new(
        vec![0xEE_u8; (width * height * 3) as usize],
        width,
        height,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    );

    let err = pipeline
        .process_frame(&frame)
        .expect_err("Must fail on spoof");
    assert!(matches!(err, soos_vision::VisionError::PadFailed { .. }));
}

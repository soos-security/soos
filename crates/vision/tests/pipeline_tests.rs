//! Tests for `VisionPipeline` orchestrator and the single-face security invariant.
//! Acceptance criteria: V4 — Rejection tests for 0 and multi-face frames.

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

use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector, MockLandmarkDetector};
use soos_inference_ort::{BiometricEmbedding, BoundingBox, FaceDetection};
use soos_vision::{VisionError, VisionPipeline, VisionPipelineConfig};

fn setup_pipeline(detections: Vec<FaceDetection>) -> (VisionPipeline, Frame) {
    let width = 320;
    let height = 240;
    let detector = Arc::new(MockFaceDetector::new_with_detections(detections));
    let landmarks = Arc::new(MockLandmarkDetector::new_canonical());
    let extractor = Arc::new(MockEmbeddingExtractor::new(128));
    let config = VisionPipelineConfig::default();

    let pipeline = VisionPipeline::new(detector, landmarks, extractor, config);
    let frame = Frame::new(
        vec![128u8; (width * height * 3) as usize],
        width,
        height,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    );
    (pipeline, frame)
}

#[test]
fn test_pipeline_rejects_zero_faces() {
    let (pipeline, frame) = setup_pipeline(vec![]);
    let result = pipeline.process_frame(&frame);
    assert!(
        matches!(result, Err(VisionError::NoFaceDetected)),
        "Expected NoFaceDetected error, got {:?}",
        result
    );
}

#[test]
fn test_pipeline_rejects_two_faces() {
    let box1 = BoundingBox::new(10.0, 10.0, 100.0, 100.0);
    let box2 = BoundingBox::new(150.0, 10.0, 240.0, 100.0);
    let detections = vec![
        FaceDetection::new(box1, 0.95),
        FaceDetection::new(box2, 0.92),
    ];

    let (pipeline, frame) = setup_pipeline(detections);
    let result = pipeline.process_frame(&frame);
    assert!(
        matches!(result, Err(VisionError::MultipleFacesDetected { count: 2 })),
        "Expected MultipleFacesDetected {{ count: 2 }}, got {:?}",
        result
    );
}

#[test]
fn test_pipeline_rejects_three_faces() {
    let box1 = BoundingBox::new(10.0, 10.0, 80.0, 80.0);
    let box2 = BoundingBox::new(100.0, 10.0, 180.0, 80.0);
    let box3 = BoundingBox::new(200.0, 10.0, 280.0, 80.0);
    let detections = vec![
        FaceDetection::new(box1, 0.95),
        FaceDetection::new(box2, 0.92),
        FaceDetection::new(box3, 0.90),
    ];

    let (pipeline, frame) = setup_pipeline(detections);
    let result = pipeline.process_frame(&frame);
    assert!(
        matches!(result, Err(VisionError::MultipleFacesDetected { count: 3 })),
        "Expected MultipleFacesDetected {{ count: 3 }}, got {:?}",
        result
    );
}

#[test]
fn test_pipeline_rejects_low_confidence_face() {
    let box1 = BoundingBox::new(50.0, 50.0, 200.0, 200.0);
    // Config requires default min_confidence 0.70; detection has 0.40
    let detections = vec![FaceDetection::new(box1, 0.40)];

    let (pipeline, frame) = setup_pipeline(detections);
    let result = pipeline.process_frame(&frame);
    assert!(
        matches!(result, Err(VisionError::FaceBelowConfidence { .. })),
        "Expected FaceBelowConfidence error, got {:?}",
        result
    );
}

#[test]
fn test_pipeline_nominal_single_face_verification() {
    let box1 = BoundingBox::new(50.0, 50.0, 200.0, 200.0);
    let detections = vec![FaceDetection::new(box1, 0.98)];

    let (pipeline, frame) = setup_pipeline(detections);
    let output = pipeline
        .process_frame(&frame)
        .expect("Nominal single face should succeed");

    assert_eq!(output.aligned_crop_rgb.len(), 112 * 112 * 3);
    assert_eq!(output.embedding.len(), 128);
    assert!(output.embedding.is_normalized(1e-4));

    // Matching against the extracted embedding itself should verify successfully
    let outcome = pipeline
        .verify(&frame, &output.embedding)
        .expect("Verification nominal");
    assert!(
        outcome.match_result.matched,
        "Self-match should yield matched=true"
    );
    assert!((outcome.match_result.score - 1.0).abs() < 1e-4);

    // Matching against orthogonal template should fail
    let mut orthogonal = vec![0.0_f32; 128];
    orthogonal[127] = 1.0;
    // Ensure it's not parallel to mock vector
    let ortho_emb = BiometricEmbedding::new(orthogonal);
    let outcome_fail = pipeline
        .verify(&frame, &ortho_emb)
        .expect("Verification check");
    assert!(
        !outcome_fail.match_result.matched,
        "Orthogonal vector should yield matched=false"
    );
}

//! Tests for `VisionPipeline` orchestrator and the single-face security invariant.
//! Acceptance criteria: V4 — Rejection tests for 0 and multi-face frames.
//! Issue #41 / NGM11, NGM12, NGM13: 3-model architecture tests.

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

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_inference_ort::pad::{PadDetector, PadResult};
use soos_inference_ort::{
    BiometricEmbedding, BoundingBox, EmbeddingExtractor, FaceDetection, FaceLandmarks, Point2f,
};
use soos_vision::{expand_bbox_for_pad, VisionError, VisionPipeline, VisionPipelineConfig};

/// Canonical 5-point facial landmarks helper.
fn canonical_landmarks() -> FaceLandmarks {
    FaceLandmarks::new(
        Point2f::new(38.29, 51.69),
        Point2f::new(73.53, 51.50),
        Point2f::new(56.02, 71.73),
        Point2f::new(41.54, 92.36),
        Point2f::new(70.72, 92.20),
    )
}

/// Spy PAD detector that records the dimensions and buffer size of the crop it receives.
struct SpyPadDetector {
    received_width: AtomicU32,
    received_height: AtomicU32,
    received_len: AtomicUsize,
    inner: MockPadDetector,
}

impl SpyPadDetector {
    fn new_live() -> Self {
        Self {
            received_width: AtomicU32::new(0),
            received_height: AtomicU32::new(0),
            received_len: AtomicUsize::new(0),
            inner: MockPadDetector::new_live(),
        }
    }
}

impl PadDetector for SpyPadDetector {
    fn evaluate_liveness(
        &self,
        crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError> {
        self.received_width.store(width, Ordering::SeqCst);
        self.received_height.store(height, Ordering::SeqCst);
        self.received_len.store(crop_rgb.len(), Ordering::SeqCst);
        self.inner.evaluate_liveness(crop_rgb, width, height)
    }
}

/// Spy embedding extractor that records the dimensions and buffer size of the crop it receives.
struct SpyExtractor {
    received_width: AtomicU32,
    received_height: AtomicU32,
    received_len: AtomicUsize,
    inner: MockEmbeddingExtractor,
}

impl SpyExtractor {
    fn new(dim: usize) -> Self {
        Self {
            received_width: AtomicU32::new(0),
            received_height: AtomicU32::new(0),
            received_len: AtomicUsize::new(0),
            inner: MockEmbeddingExtractor::new(dim),
        }
    }
}

impl EmbeddingExtractor for SpyExtractor {
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError> {
        self.received_width.store(width, Ordering::SeqCst);
        self.received_height.store(height, Ordering::SeqCst);
        self.received_len
            .store(aligned_crop_rgb.len(), Ordering::SeqCst);
        self.inner
            .extract_embedding(aligned_crop_rgb, width, height)
    }
}

fn setup_pipeline(detections: Vec<FaceDetection>) -> (VisionPipeline, Frame) {
    let width = 320;
    let height = 240;
    let detector = Arc::new(MockFaceDetector::new_with_detections(detections));
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let config = VisionPipelineConfig::default();

    let pipeline = VisionPipeline::new(detector, pad, extractor, config);
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
    let detections = vec![FaceDetection::with_landmarks(
        box1,
        0.98,
        canonical_landmarks(),
    )];

    let (pipeline, frame) = setup_pipeline(detections);
    let output = pipeline
        .process_frame(&frame)
        .expect("Nominal single face should succeed");

    assert_eq!(output.aligned_crop_rgb.len(), 112 * 112 * 3);
    assert_eq!(output.embedding.len(), 512);
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
    let mut orthogonal = vec![0.0_f32; 512];
    orthogonal[511] = 1.0;
    let ortho_emb = BiometricEmbedding::new(orthogonal);
    let outcome_fail = pipeline
        .verify(&frame, &ortho_emb)
        .expect("Verification check");
    assert!(
        !outcome_fail.match_result.matched,
        "Orthogonal vector should yield matched=false"
    );
}

#[test]
fn test_pipeline_constructs_with_three_backends() {
    let detector = Arc::new(MockFaceDetector::new_empty());
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let config = VisionPipelineConfig::default();

    let pipeline = VisionPipeline::new(detector, pad, extractor, config.clone());
    assert_eq!(pipeline.config(), &config);
}

#[test]
fn test_pipeline_extracts_landmarks_from_detection() {
    let landmarks = FaceLandmarks::new(
        Point2f::new(38.0, 52.0),
        Point2f::new(74.0, 52.0),
        Point2f::new(56.0, 70.0),
        Point2f::new(42.0, 88.0),
        Point2f::new(70.0, 88.0),
    );
    let detection =
        FaceDetection::with_landmarks(BoundingBox::new(20.0, 20.0, 92.0, 92.0), 0.99, landmarks);

    let width = 320;
    let height = 240;
    let detector = Arc::new(MockFaceDetector::new_with_detections(vec![detection]));
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let config = VisionPipelineConfig::default();

    let pipeline = VisionPipeline::new(detector, pad, extractor, config);
    let frame = Frame::new(
        vec![128u8; (width * height * 3) as usize],
        width,
        height,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    );

    let output = pipeline
        .process_frame(&frame)
        .expect("Pipeline should process frame");
    assert_eq!(
        output.landmarks, landmarks,
        "Landmarks in output must match detection landmarks"
    );
}

#[test]
fn test_pipeline_fails_when_detection_lacks_landmarks() {
    let detection = FaceDetection::new(BoundingBox::new(20.0, 20.0, 92.0, 92.0), 0.99);
    assert!(detection.landmarks.is_none());

    let width = 320;
    let height = 240;
    let detector = Arc::new(MockFaceDetector::new_with_detections(vec![detection]));
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let config = VisionPipelineConfig::default();

    let pipeline = VisionPipeline::new(detector, pad, extractor, config);
    let frame = Frame::new(
        vec![128u8; (width * height * 3) as usize],
        width,
        height,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    );

    let res = pipeline.process_frame(&frame);
    assert!(
        matches!(res, Err(VisionError::MissingLandmarks)),
        "Expected VisionError::MissingLandmarks, got {:?}",
        res
    );
}

#[test]
fn test_expand_bbox_centered() {
    let bbox = BoundingBox::new(100.0, 100.0, 200.0, 200.0);
    let expanded = expand_bbox_for_pad(&bbox, 2.7, 640, 480);

    // Original center: (150, 150)
    let orig_cx = (bbox.x1 + bbox.x2) / 2.0;
    let orig_cy = (bbox.y1 + bbox.y2) / 2.0;
    let exp_cx = (expanded.x1 + expanded.x2) / 2.0;
    let exp_cy = (expanded.y1 + expanded.y2) / 2.0;

    assert!(
        (orig_cx - exp_cx).abs() < 1e-4,
        "Expanded bbox center X must match original center"
    );
    assert!(
        (orig_cy - exp_cy).abs() < 1e-4,
        "Expanded bbox center Y must match original center"
    );

    // Original width: 100.0 -> expanded: 270.0
    assert!(((expanded.x2 - expanded.x1) - 270.0).abs() < 1e-4);
    assert!(((expanded.y2 - expanded.y1) - 270.0).abs() < 1e-4);
    assert!((expanded.x1 - 15.0).abs() < 1e-4);
    assert!((expanded.y1 - 15.0).abs() < 1e-4);
    assert!((expanded.x2 - 285.0).abs() < 1e-4);
    assert!((expanded.y2 - 285.0).abs() < 1e-4);
}

#[test]
fn test_expand_bbox_clamped_to_image() {
    // Bbox near top-left: expansion would go negative
    let top_left = BoundingBox::new(10.0, 10.0, 110.0, 110.0);
    let exp_tl = expand_bbox_for_pad(&top_left, 2.7, 640, 480);
    assert_eq!(exp_tl.x1, 0.0, "Clamped to left edge 0.0");
    assert_eq!(exp_tl.y1, 0.0, "Clamped to top edge 0.0");
    assert!(exp_tl.x2 > 0.0 && exp_tl.x2 <= 640.0);
    assert!(exp_tl.y2 > 0.0 && exp_tl.y2 <= 480.0);

    // Bbox near bottom-right: expansion exceeds frame dimensions
    let bottom_right = BoundingBox::new(550.0, 400.0, 630.0, 470.0);
    let exp_br = expand_bbox_for_pad(&bottom_right, 2.7, 640, 480);
    assert!(exp_br.x1 >= 0.0);
    assert!(exp_br.y1 >= 0.0);
    assert_eq!(exp_br.x2, 640.0, "Clamped to right edge 640.0");
    assert_eq!(exp_br.y2, 480.0, "Clamped to bottom edge 480.0");
}

#[test]
fn test_pipeline_pad_receives_expanded_crop() {
    let landmarks = canonical_landmarks();
    let detection =
        FaceDetection::with_landmarks(BoundingBox::new(50.0, 50.0, 150.0, 150.0), 0.95, landmarks);

    let width = 320;
    let height = 240;
    let detector = Arc::new(MockFaceDetector::new_with_detections(vec![detection]));
    let pad = Arc::new(SpyPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let config = VisionPipelineConfig::default();

    let pipeline = VisionPipeline::new(detector, pad.clone(), extractor, config);
    let frame = Frame::new(
        vec![128u8; (width * height * 3) as usize],
        width,
        height,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    );

    let _ = pipeline
        .process_frame(&frame)
        .expect("process_frame nominal");

    assert_eq!(
        pad.received_width.load(Ordering::SeqCst),
        80,
        "PAD width must be 80"
    );
    assert_eq!(
        pad.received_height.load(Ordering::SeqCst),
        80,
        "PAD height must be 80"
    );
    assert_eq!(
        pad.received_len.load(Ordering::SeqCst),
        80 * 80 * 3,
        "PAD buffer size must be 80*80*3"
    );
}

#[test]
fn test_pipeline_embedding_receives_aligned_crop() {
    let landmarks = canonical_landmarks();
    let detection =
        FaceDetection::with_landmarks(BoundingBox::new(50.0, 50.0, 150.0, 150.0), 0.95, landmarks);

    let width = 320;
    let height = 240;
    let detector = Arc::new(MockFaceDetector::new_with_detections(vec![detection]));
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(SpyExtractor::new(512));
    let config = VisionPipelineConfig::default();

    let pipeline = VisionPipeline::new(detector, pad, extractor.clone(), config);
    let frame = Frame::new(
        vec![128u8; (width * height * 3) as usize],
        width,
        height,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    );

    let _ = pipeline
        .process_frame(&frame)
        .expect("process_frame nominal");

    assert_eq!(
        extractor.received_width.load(Ordering::SeqCst),
        112,
        "Embedding width must be 112"
    );
    assert_eq!(
        extractor.received_height.load(Ordering::SeqCst),
        112,
        "Embedding height must be 112"
    );
    assert_eq!(
        extractor.received_len.load(Ordering::SeqCst),
        112 * 112 * 3,
        "Embedding crop size must be 112*112*3"
    );
}

#[test]
fn test_pipeline_config_defaults_3_model() {
    let config = VisionPipelineConfig::default();
    assert_eq!(config.pad_target_width, 80);
    assert_eq!(config.pad_target_height, 80);
    assert!((config.pad_bbox_scale - 2.7).abs() < 1e-4);
    assert_eq!(config.target_width, 112);
    assert_eq!(config.target_height, 112);
    assert!(
        (config.match_threshold - 0.50).abs() < 1e-6,
        "VisionPipelineConfig default match_threshold must be 0.50 (SFace, LFW FAR 3.9e-6)"
    );
    assert!(
        (config.pad_threshold - 0.85).abs() < 1e-6,
        "VisionPipelineConfig default pad_threshold must be 0.85 matching policy"
    );
}

#[test]
fn test_vision_pipeline_default_thresholds_calibrated() {
    let vision_cfg = VisionPipelineConfig::default();

    assert!(
        (vision_cfg.match_threshold - 0.50).abs() < 1e-6,
        "Vision match_threshold ({}) must be 0.50 (SFace, LFW FAR 3.9e-6)",
        vision_cfg.match_threshold
    );
    assert!(
        (vision_cfg.pad_threshold - 0.85).abs() < 1e-6,
        "Vision pad_threshold ({}) must be 0.85 matching NIST SP 800-63B standards",
        vision_cfg.pad_threshold
    );
}

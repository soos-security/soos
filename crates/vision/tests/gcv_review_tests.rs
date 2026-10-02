//! GitHub #304 (VIS-NEW-1) and #313 (VIS-NEW-5, VIS-NEW-6): review findings of 2026-10-02
//! (matrix rows GCV1, GCV11, GCV12).
//!
//! - `analyze_frame` exposes the face count and never scores PAD, aligns or embeds a face
//!   when the frame holds more than one face (the GUI guided enrollment samples from it);
//! - pipeline outputs never print biometric data through `Debug`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual test suite uses direct assertions and synthetic frame fixtures"
)]

use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::{
    BoundingBox, FaceDetection, MockEmbeddingExtractor, MockFaceDetector, MockPadDetector,
};
use soos_vision::{VisionAnalysis, VisionPipeline, VisionPipelineConfig};
use zeroize::Zeroizing;

const W: u32 = 640;
const H: u32 = 480;

fn flat_frame() -> Frame {
    Frame::new(
        vec![128u8; (W * H * 3) as usize],
        W,
        H,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    )
}

fn face_at(cx: f32, cy: f32, side: f32, score: f32) -> FaceDetection {
    let box_ = BoundingBox::new(
        cx - side / 2.0,
        cy - side / 2.0,
        cx + side / 2.0,
        cy + side / 2.0,
    );
    let landmarks = MockFaceDetector::canonical_landmarks_for_box(&box_);
    FaceDetection::with_landmarks(box_, score, landmarks)
}

fn pipeline(detections: Vec<FaceDetection>) -> (VisionPipeline, Arc<MockPadDetector>) {
    let detector = Arc::new(MockFaceDetector::new_with_detections(detections));
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(128));
    (
        VisionPipeline::new(
            detector,
            Arc::clone(&pad) as _,
            extractor,
            VisionPipelineConfig::default(),
        ),
        pad,
    )
}

/// GCV1: two confident faces are counted, and neither is scored, aligned or embedded.
#[test]
fn test_gcv_analyze_frame_two_faces_skips_pad_and_embedding() {
    let (pipeline, pad) = pipeline(vec![
        face_at(200.0, 240.0, 140.0, 0.97),
        face_at(440.0, 240.0, 140.0, 0.95),
    ]);
    let analysis = pipeline.analyze_frame(&flat_frame()).expect("analysis");
    assert_eq!(analysis.face_count(), 2);
    assert_eq!(analysis.detections.len(), 2, "detections stay visible");
    assert!(
        analysis.pad_result.is_none(),
        "no PAD on a multi-face frame"
    );
    assert!(
        analysis.embedding.is_none(),
        "no embedding on a multi-face frame"
    );
    assert!(analysis.aligned_crop.is_none(), "no aligned crop either");
    assert_eq!(pad.call_count(), 0, "the PAD model is never consulted");
}

/// GCV1: a single face keeps the full analysis.
#[test]
fn test_gcv_analyze_frame_single_face_is_analyzed() {
    let (pipeline, _pad) = pipeline(vec![face_at(320.0, 240.0, 140.0, 0.97)]);
    let analysis = pipeline.analyze_frame(&flat_frame()).expect("analysis");
    assert_eq!(analysis.face_count(), 1);
    assert!(analysis.pad_result.is_some());
    assert!(analysis.embedding.is_some());
}

/// GCV1: a second face below the detector confidence still counts (same rule as
/// `process_frame`, which rejects any frame with more than one detection).
#[test]
fn test_gcv_analyze_frame_counts_every_detection_like_process_frame() {
    let (pipeline, _pad) = pipeline(vec![
        face_at(200.0, 240.0, 140.0, 0.97),
        face_at(440.0, 240.0, 140.0, 0.10),
    ]);
    let analysis = pipeline.analyze_frame(&flat_frame()).expect("analysis");
    assert_eq!(analysis.face_count(), 2);
    assert!(analysis.embedding.is_none());
    assert!(pipeline.process_frame(&flat_frame()).is_err());
}

/// GCV1: no face at all.
#[test]
fn test_gcv_analyze_frame_without_face_counts_zero() {
    let (pipeline, _pad) = pipeline(Vec::new());
    let analysis = pipeline.analyze_frame(&flat_frame()).expect("analysis");
    assert_eq!(analysis.face_count(), 0);
    assert!(analysis.embedding.is_none());
}

/// GCV12: `Debug` of the pipeline outputs never prints pixels or embedding values.
#[test]
fn test_gcv_pipeline_outputs_debug_is_redacted() {
    let (pipeline, _pad) = pipeline(vec![face_at(320.0, 240.0, 140.0, 0.97)]);
    let output = pipeline.process_frame(&flat_frame()).expect("output");
    let text = format!("{output:?}");
    let first = output.embedding.as_slice()[0];
    assert!(!text.contains(&format!("{first}")), "{text}");
    assert!(!text.contains("128, 128, 128"), "no pixel bytes: {text}");
    assert!(text.contains("PipelineOutput"));

    let analysis = VisionAnalysis {
        rgb: Zeroizing::new(vec![77u8; 12]),
        detections: Vec::new(),
        pad_result: None,
        pose: None,
        aligned_crop: Some(Zeroizing::new(vec![91u8; 12])),
        embedding: Some(Zeroizing::new(vec![0.123_456_7_f32; 4])),
        quality_rejection: None,
    };
    let text = format!("{analysis:?}");
    assert!(!text.contains("77"), "no frame bytes: {text}");
    assert!(!text.contains("91"), "no crop bytes: {text}");
    assert!(!text.contains("0.123"), "no embedding values: {text}");
    assert!(text.contains("VisionAnalysis"));
}

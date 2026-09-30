//! Contractual tests for the pre-PAD face quality gate (GitHub #218 / PAD-13).
//!
//! A face whose bounding box is smaller than `min_face_width_px` (smaller box side) or
//! whose 80x80 PAD crop is below `min_pad_crop_sharpness` (variance of the Laplacian)
//! must be rejected before the PAD model and the embedding extractor are consulted.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "Contractual test suite uses direct assertions and synthetic frame fixtures"
)]

use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::{
    BoundingBox, FaceDetection, MockEmbeddingExtractor, MockFaceDetector, MockPadDetector,
};
use soos_vision::quality::{
    laplacian_variance, FaceQualityRejection, DEFAULT_MIN_FACE_WIDTH_PX,
    DEFAULT_MIN_PAD_CROP_SHARPNESS,
};
use soos_vision::{VisionError, VisionPipeline, VisionPipelineConfig};

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

/// RGB24 checkerboard with `cell`-pixel squares (strong edges survive the PAD resize).
fn checkerboard_frame(cell: u32) -> Frame {
    let mut data = vec![0u8; (W * H * 3) as usize];
    for y in 0..H {
        for x in 0..W {
            let v = if ((x / cell) + (y / cell)).is_multiple_of(2) {
                30
            } else {
                220
            };
            let i = ((y * W + x) * 3) as usize;
            data[i] = v;
            data[i + 1] = v;
            data[i + 2] = v;
        }
    }
    Frame::new(data, W, H, 1_000_000, PixelFormat::Rgb24, 1)
}

/// Face box of `w` x `h` pixels centred in the frame, with canonical landmarks.
fn centred_detection(w: f32, h: f32) -> FaceDetection {
    let cx = W as f32 / 2.0;
    let cy = H as f32 / 2.0;
    let box_ = BoundingBox::new(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0);
    let landmarks = MockFaceDetector::canonical_landmarks_for_box(&box_);
    FaceDetection::with_landmarks(box_, 0.95, landmarks)
}

fn pipeline_with(
    detection: FaceDetection,
    config: VisionPipelineConfig,
) -> (VisionPipeline, Arc<MockPadDetector>) {
    let detector = Arc::new(MockFaceDetector::new_with_detections(vec![detection]));
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    (
        VisionPipeline::new(detector, pad.clone(), extractor, config),
        pad,
    )
}

#[test]
fn test_quality_gate_default_constants() {
    assert_eq!(DEFAULT_MIN_FACE_WIDTH_PX, 48.0);
    // Sharpness gate ships disabled until calibrated on real hardware (ADR PQG).
    assert_eq!(DEFAULT_MIN_PAD_CROP_SHARPNESS, 0.0);
    let config = VisionPipelineConfig::default();
    assert_eq!(config.min_face_width_px, DEFAULT_MIN_FACE_WIDTH_PX);
    assert_eq!(
        config.min_pad_crop_sharpness,
        DEFAULT_MIN_PAD_CROP_SHARPNESS
    );
    // The floor must at least avoid upsampling the 2.7x context crop into the PAD input.
    let no_upsample_floor = config.pad_target_width as f32 / config.pad_bbox_scale;
    assert!(DEFAULT_MIN_FACE_WIDTH_PX > no_upsample_floor);
}

#[test]
fn test_process_frame_rejects_30px_face_as_too_small_before_pad() {
    let (pipeline, pad) = pipeline_with(
        centred_detection(30.0, 30.0),
        VisionPipelineConfig::default(),
    );
    let err = pipeline.process_frame(&flat_frame()).unwrap_err();
    match err {
        VisionError::FaceTooSmall {
            width_px,
            min_width_px,
        } => {
            assert!((width_px - 30.0).abs() < 1e-3, "width {width_px}");
            assert_eq!(min_width_px, DEFAULT_MIN_FACE_WIDTH_PX);
        }
        other => panic!("Expected FaceTooSmall, got {other:?}"),
    }
    assert_eq!(pad.call_count(), 0, "PAD must not score a too-small face");
}

#[test]
fn test_process_frame_accepts_120px_face() {
    let (pipeline, pad) = pipeline_with(
        centred_detection(120.0, 120.0),
        VisionPipelineConfig::default(),
    );
    let output = pipeline.process_frame(&flat_frame()).expect("120 px face");
    assert!(output.pad_result.is_live);
    assert_eq!(pad.call_count(), 1);
}

#[test]
fn test_face_size_gate_uses_smaller_box_side() {
    let (pipeline, _pad) = pipeline_with(
        centred_detection(200.0, 30.0),
        VisionPipelineConfig::default(),
    );
    let err = pipeline.process_frame(&flat_frame()).unwrap_err();
    assert!(
        matches!(err, VisionError::FaceTooSmall { width_px, .. } if (width_px - 30.0).abs() < 1e-3),
        "got {err:?}"
    );
}

#[test]
fn test_face_size_gate_rejects_non_finite_box() {
    let box_ = BoundingBox::new(f32::NAN, 100.0, 300.0, 300.0);
    let landmarks = MockFaceDetector::canonical_landmarks_for_box(&BoundingBox::new(
        100.0, 100.0, 300.0, 300.0,
    ));
    let detection = FaceDetection::with_landmarks(box_, 0.95, landmarks);
    let (pipeline, pad) = pipeline_with(detection, VisionPipelineConfig::default());
    let err = pipeline.process_frame(&flat_frame()).unwrap_err();
    assert!(
        matches!(err, VisionError::FaceTooSmall { .. }),
        "got {err:?}"
    );
    assert_eq!(pad.call_count(), 0);
}

#[test]
fn test_face_size_gate_fails_closed_on_nan_threshold() {
    let config = VisionPipelineConfig {
        min_face_width_px: f32::NAN,
        ..Default::default()
    };
    let (pipeline, _pad) = pipeline_with(centred_detection(200.0, 200.0), config);
    let err = pipeline.process_frame(&flat_frame()).unwrap_err();
    assert!(
        matches!(err, VisionError::FaceTooSmall { .. }),
        "got {err:?}"
    );
}

#[test]
fn test_laplacian_variance_flat_is_zero_and_edges_are_high() {
    let flat = vec![128u8; 80 * 80 * 3];
    assert_eq!(laplacian_variance(&flat, 80, 80), Some(0.0));

    let mut board = vec![0u8; 80 * 80 * 3];
    for y in 0..80usize {
        for x in 0..80usize {
            let v = if ((x / 2) + (y / 2)).is_multiple_of(2) {
                0
            } else {
                255
            };
            let i = (y * 80 + x) * 3;
            board[i] = v;
            board[i + 1] = v;
            board[i + 2] = v;
        }
    }
    let sharp = laplacian_variance(&board, 80, 80).expect("valid buffer");
    assert!(sharp > 1000.0, "checkerboard sharpness {sharp}");
}

#[test]
fn test_laplacian_variance_rejects_invalid_buffers() {
    assert_eq!(laplacian_variance(&[0u8; 10], 80, 80), None);
    assert_eq!(laplacian_variance(&[0u8; 2 * 2 * 3], 2, 2), None);
    assert_eq!(laplacian_variance(&[], 0, 0), None);
}

#[test]
fn test_blurred_pad_crop_rejected_when_sharpness_gate_enabled() {
    let config = VisionPipelineConfig {
        min_pad_crop_sharpness: 5.0,
        ..Default::default()
    };
    let (pipeline, pad) = pipeline_with(centred_detection(120.0, 120.0), config);
    let err = pipeline.process_frame(&flat_frame()).unwrap_err();
    match err {
        VisionError::FaceBlurred {
            sharpness,
            min_sharpness,
        } => {
            assert_eq!(sharpness, 0.0);
            assert_eq!(min_sharpness, 5.0);
        }
        other => panic!("Expected FaceBlurred, got {other:?}"),
    }
    assert_eq!(pad.call_count(), 0, "PAD must not score a blurred crop");
}

#[test]
fn test_sharp_pad_crop_passes_sharpness_gate() {
    let config = VisionPipelineConfig {
        min_pad_crop_sharpness: 5.0,
        ..Default::default()
    };
    let (pipeline, pad) = pipeline_with(centred_detection(120.0, 120.0), config);
    let output = pipeline
        .process_frame(&checkerboard_frame(8))
        .expect("sharp crop");
    assert!(output.pad_result.is_live);
    assert_eq!(pad.call_count(), 1);
}

#[test]
fn test_analyze_frame_reports_quality_rejection_and_skips_pad_and_embedding() {
    let (pipeline, pad) = pipeline_with(
        centred_detection(30.0, 30.0),
        VisionPipelineConfig::default(),
    );
    let analysis = pipeline.analyze_frame(&flat_frame()).expect("analysis");
    assert_eq!(
        analysis.quality_rejection,
        Some(FaceQualityRejection::TooSmall)
    );
    assert!(analysis.pad_result.is_none());
    assert!(analysis.embedding.is_none());
    assert_eq!(pad.call_count(), 0);
}

#[test]
fn test_analyze_frame_reports_blurred_crop() {
    let config = VisionPipelineConfig {
        min_pad_crop_sharpness: 5.0,
        ..Default::default()
    };
    let (pipeline, _pad) = pipeline_with(centred_detection(120.0, 120.0), config);
    let analysis = pipeline.analyze_frame(&flat_frame()).expect("analysis");
    assert_eq!(
        analysis.quality_rejection,
        Some(FaceQualityRejection::Blurred)
    );
    assert!(analysis.pad_result.is_none());
    assert!(analysis.embedding.is_none());
}

#[test]
fn test_analyze_frame_accepts_good_face_without_quality_rejection() {
    let (pipeline, _pad) = pipeline_with(
        centred_detection(120.0, 120.0),
        VisionPipelineConfig::default(),
    );
    let analysis = pipeline.analyze_frame(&flat_frame()).expect("analysis");
    assert_eq!(analysis.quality_rejection, None);
    assert!(analysis.pad_result.is_some());
    assert!(analysis.embedding.is_some());
}

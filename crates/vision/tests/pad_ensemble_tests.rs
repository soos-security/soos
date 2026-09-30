//! Multi-scale PAD fusion (review finding PAD-07, GitHub #212) and the upstream-parity PAD
//! crop used by the pipeline (review finding PAD-08, GitHub #213).
//!
//! Upstream Silent-Face-Anti-Spoofing averages the softmax of the 2.7-scale MiniFASNetV2 and
//! the 4.0-scale MiniFASNetV1SE, each run on its own context crop. The pipeline accepts extra
//! `(scale, detector)` members; the decision is the mean live probability of every member,
//! compared with the (modality-aware) PAD threshold. Without extra members the behaviour is
//! the single-model behaviour, unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use std::sync::{Arc, Mutex};

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_inference_ort::pad::{AttackType, PadDetector, PadResult};
use soos_inference_ort::{BoundingBox, FaceDetection};
use soos_vision::crop::crop_pad_context;
use soos_vision::pad_fusion::fuse_pad_results;
use soos_vision::pipeline::MAX_PAD_ENSEMBLE_MODELS;
use soos_vision::{VisionError, VisionPipeline, VisionPipelineConfig};

const W: u32 = 320;
const H: u32 = 240;

/// Records every crop it receives and returns a fixed result.
struct RecordingPad {
    crops: Mutex<Vec<Vec<u8>>>,
    result: PadResult,
}

impl RecordingPad {
    fn new(result: PadResult) -> Arc<Self> {
        Arc::new(Self {
            crops: Mutex::new(Vec::new()),
            result,
        })
    }

    fn crops(&self) -> Vec<Vec<u8>> {
        self.crops.lock().unwrap().clone()
    }
}

impl PadDetector for RecordingPad {
    fn evaluate_liveness(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError> {
        if (width, height) != (80, 80) {
            return Err(InferenceError::InvalidDimensions {
                expected: (80, 80),
                actual: (width, height),
            });
        }
        self.crops.lock().unwrap().push(rgb.to_vec());
        Ok(self.result.clone())
    }
}

fn face_box() -> BoundingBox {
    BoundingBox::new(120.0, 70.0, 180.0, 150.0)
}

fn textured_frame(format: PixelFormat) -> Frame {
    let (bpp, fmt) = match format {
        PixelFormat::Grey => (1u32, PixelFormat::Grey),
        _ => (3u32, PixelFormat::Rgb24),
    };
    let mut data = Vec::with_capacity((W * H * bpp) as usize);
    for y in 0..H {
        for x in 0..W {
            for c in 0..bpp {
                data.push(((x * 13 + y * 7 + c * 29) % 200 + 20) as u8);
            }
        }
    }
    Frame::new(data, W, H, 1_000_000, fmt, 1)
}

fn frame_rgb(frame: &Frame) -> Vec<u8> {
    soos_vision::convert_to_rgb(&frame.data, frame.width, frame.height, frame.format).unwrap()
}

fn pipeline(primary: Arc<dyn PadDetector>) -> VisionPipeline {
    let detection = FaceDetection::with_landmarks(
        face_box(),
        0.95,
        MockFaceDetector::canonical_landmarks_for_box(&face_box()),
    );
    VisionPipeline::new(
        Arc::new(MockFaceDetector::new_with_detections(vec![detection])),
        primary,
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig::default(),
    )
}

// ---------------------------------------------------------------------------
// Fusion rule
// ---------------------------------------------------------------------------

#[test]
fn test_fuse_pad_results_is_mean_of_live_probabilities() {
    let fused = fuse_pad_results(&[PadResult::live(0.95), PadResult::live(0.85)], 0.85)
        .expect("two members fuse");
    assert!((fused.score - 0.90).abs() < 1e-6);
    assert!(fused.is_live);

    // One confident spoof drags the mean below the threshold even if the other is live.
    let fused = fuse_pad_results(
        &[
            PadResult::live(0.99),
            PadResult::spoof(0.40, AttackType::ScreenReplay),
        ],
        0.85,
    )
    .expect("two members fuse");
    assert!((fused.score - 0.695).abs() < 1e-6);
    assert!(!fused.is_live);
    assert_eq!(fused.attack_type, Some(AttackType::ScreenReplay));

    // Averaging can accept a member below the threshold (upstream semantics).
    let fused = fuse_pad_results(
        &[
            PadResult::spoof(0.80, AttackType::PrintPhoto),
            PadResult::live(0.96),
        ],
        0.85,
    )
    .expect("two members fuse");
    assert!((fused.score - 0.88).abs() < 1e-6);
    assert!(fused.is_live);
    assert_eq!(fused.attack_type, None);
}

#[test]
fn test_fuse_pad_results_single_member_is_passthrough_and_empty_is_none() {
    let only = PadResult::spoof(0.97, AttackType::PrintPhoto);
    assert_eq!(
        fuse_pad_results(std::slice::from_ref(&only), 0.85),
        Some(only)
    );
    assert_eq!(fuse_pad_results(&[], 0.85), None);
}

#[test]
fn test_fuse_pad_results_non_finite_fails_closed() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let fused =
            fuse_pad_results(&[PadResult::live(0.99), PadResult::live(bad)], 0.85).expect("fuses");
        assert!(!fused.is_live, "non-finite member score {bad} must reject");
    }
    let fused =
        fuse_pad_results(&[PadResult::live(0.99), PadResult::live(0.99)], f32::NAN).expect("fuses");
    assert!(!fused.is_live, "NaN threshold must reject");
}

// ---------------------------------------------------------------------------
// Ensemble construction
// ---------------------------------------------------------------------------

#[test]
fn test_additional_pad_model_rejects_invalid_scale_and_bounds_members() {
    for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
        let res = pipeline(Arc::new(MockPadDetector::new_live()))
            .with_additional_pad_model(bad, Arc::new(MockPadDetector::new_live()));
        assert!(
            matches!(res, Err(VisionError::InvalidPadEnsemble { .. })),
            "scale {bad} must be rejected"
        );
    }

    let mut p = pipeline(Arc::new(MockPadDetector::new_live()));
    assert_eq!(p.pad_scales(), vec![2.7]);
    for i in 1..MAX_PAD_ENSEMBLE_MODELS {
        p = p
            .with_additional_pad_model(1.0 + i as f32, Arc::new(MockPadDetector::new_live()))
            .expect("within bound");
    }
    assert_eq!(p.pad_scales().len(), MAX_PAD_ENSEMBLE_MODELS);
    assert!(matches!(
        p.with_additional_pad_model(9.0, Arc::new(MockPadDetector::new_live())),
        Err(VisionError::InvalidPadEnsemble { .. })
    ));
}

// ---------------------------------------------------------------------------
// Pipeline geometry and decision
// ---------------------------------------------------------------------------

#[test]
fn test_pipeline_primary_pad_crop_uses_upstream_geometry() {
    let pad = RecordingPad::new(PadResult::live(0.99));
    let frame = textured_frame(PixelFormat::Rgb24);
    pipeline(pad.clone())
        .process_frame(&frame)
        .expect("live frame passes");
    let expected = crop_pad_context(&frame_rgb(&frame), W, H, &face_box(), 2.7, 80, 80).unwrap();
    assert_eq!(pad.crops(), vec![expected]);
}

#[test]
fn test_pipeline_runs_each_member_on_its_own_scale_crop() {
    let primary = RecordingPad::new(PadResult::live(0.99));
    let secondary = RecordingPad::new(PadResult::live(0.97));
    let frame = textured_frame(PixelFormat::Rgb24);
    let p = pipeline(primary.clone())
        .with_additional_pad_model(4.0, secondary.clone())
        .expect("valid member");
    assert_eq!(p.pad_scales(), vec![2.7, 4.0]);

    let out = p.process_frame(&frame).expect("both live");
    let rgb = frame_rgb(&frame);
    let crop_27 = crop_pad_context(&rgb, W, H, &face_box(), 2.7, 80, 80).unwrap();
    let crop_40 = crop_pad_context(&rgb, W, H, &face_box(), 4.0, 80, 80).unwrap();
    assert_ne!(crop_27, crop_40, "the two scales see different context");
    assert_eq!(primary.crops(), vec![crop_27]);
    assert_eq!(secondary.crops(), vec![crop_40]);
    assert!(
        (out.pad_result.score - 0.98).abs() < 1e-6,
        "mean of 0.99 and 0.97"
    );
}

#[test]
fn test_pipeline_fused_score_below_threshold_is_pad_failed() {
    let p = pipeline(RecordingPad::new(PadResult::live(0.95)))
        .with_additional_pad_model(4.0, RecordingPad::new(PadResult::live(0.70)))
        .expect("valid member");
    let res = p.process_frame(&textured_frame(PixelFormat::Rgb24));
    match res {
        Err(VisionError::PadFailed { score, threshold }) => {
            assert!((score - 0.825).abs() < 1e-6, "fused score {score}");
            assert!((threshold - 0.85).abs() < 1e-6);
        }
        other => panic!("expected PadFailed, got {other:?}"),
    }
}

#[test]
fn test_pipeline_fused_score_above_threshold_passes_even_if_one_member_is_lower() {
    let p = pipeline(RecordingPad::new(PadResult::spoof(
        0.80,
        AttackType::PrintPhoto,
    )))
    .with_additional_pad_model(4.0, RecordingPad::new(PadResult::live(0.96)))
    .expect("valid member");
    let out = p
        .process_frame(&textured_frame(PixelFormat::Rgb24))
        .expect("fused 0.88 >= 0.85");
    assert!(out.pad_result.is_live);
    assert!((out.pad_result.score - 0.88).abs() < 1e-6);
}

#[test]
fn test_pipeline_member_inference_error_fails_closed() {
    let failing = Arc::new(MockPadDetector::new_live());
    failing.set_fail_next(true);
    let p = pipeline(RecordingPad::new(PadResult::live(0.99)))
        .with_additional_pad_model(4.0, failing)
        .expect("valid member");
    assert!(matches!(
        p.process_frame(&textured_frame(PixelFormat::Rgb24)),
        Err(VisionError::Inference(_))
    ));
}

#[test]
fn test_pipeline_monochrome_ensemble_uses_ir_threshold() {
    // Mean 0.90: above the colour threshold 0.85, below the IR threshold 0.95.
    let p = pipeline(RecordingPad::new(PadResult::live(0.92)))
        .with_additional_pad_model(4.0, RecordingPad::new(PadResult::live(0.88)))
        .expect("valid member");
    match p.process_frame(&textured_frame(PixelFormat::Grey)) {
        Err(VisionError::PadFailed { threshold, .. }) => {
            assert!((threshold - 0.95).abs() < 1e-6);
        }
        other => panic!("expected PadFailed at the IR threshold, got {other:?}"),
    }
}

#[test]
fn test_analyze_frame_reports_fused_result() {
    let p = pipeline(RecordingPad::new(PadResult::live(0.95)))
        .with_additional_pad_model(4.0, RecordingPad::new(PadResult::live(0.70)))
        .expect("valid member");
    let analysis = p
        .analyze_frame(&textured_frame(PixelFormat::Rgb24))
        .expect("analysis");
    let pad = analysis.pad_result.clone().expect("pad result");
    assert!((pad.score - 0.825).abs() < 1e-6);
    assert!(
        !pad.is_live,
        "GUI must never show a fused sub-threshold result as live"
    );
}

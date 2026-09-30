//! Contract tests for the format-aware PAD policy (review finding PAD-03 / GitHub #169).
//!
//! MiniFASNetV2 is trained on colour (RGB/BGR) captures. A `PixelFormat::Grey` frame (the
//! default IR sensor on dual-sensor laptops) is replicated into three identical channels by
//! `convert_to_rgb`, which is out-of-distribution for the model. These tests enforce that:
//! - the source pixel format is mapped to a PAD input modality (`Grey` => monochrome);
//! - a monochrome frame never silently takes the RGB PAD path: it must pass a fail-closed
//!   IR exposure/contrast/texture gate AND a stricter, never-looser liveness threshold;
//! - colour frames keep the existing RGB behavior unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual test suite uses assertions, unwrap and synthetic pixel math"
)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_inference_ort::pad::{PadDetector, PadResult};
use soos_inference_ort::{BiometricEmbedding, EmbeddingExtractor};
use soos_vision::{
    evaluate_ir_gate, ir_crop_statistics, IrGateRejection, PadInputModality, VisionError,
    VisionPipeline, VisionPipelineConfig, DEFAULT_IR_PAD_THRESHOLD, IR_MAX_MEAN_LUMA,
    IR_MIN_LUMA_STDDEV, IR_MIN_MEAN_LUMA, IR_MIN_TEXTURE_ENERGY,
};

const W: u32 = 640;
const H: u32 = 480;

/// Spy extractor recording whether an embedding was computed.
struct SpyExtractor {
    called: AtomicBool,
    inner: MockEmbeddingExtractor,
}

impl SpyExtractor {
    fn extractor_called(&self) -> bool {
        self.called.load(Ordering::SeqCst)
    }
}

impl EmbeddingExtractor for SpyExtractor {
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError> {
        self.called.store(true, Ordering::SeqCst);
        self.inner
            .extract_embedding(aligned_crop_rgb, width, height)
    }
}

/// Spy PAD detector counting how many times the RGB-trained model is invoked.
struct CountingPad {
    calls: AtomicUsize,
    inner: MockPadDetector,
}

impl PadDetector for CountingPad {
    fn evaluate_liveness(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.evaluate_liveness(rgb, width, height)
    }
}

struct Harness {
    pipeline: VisionPipeline,
    pad: Arc<CountingPad>,
    extractor: Arc<SpyExtractor>,
}

fn harness(pad_result: PadResult, config: VisionPipelineConfig) -> Harness {
    let detector = Arc::new(MockFaceDetector::new_centered_face(W, H, 0.95));
    let pad = Arc::new(CountingPad {
        calls: AtomicUsize::new(0),
        inner: MockPadDetector::new_with_result(pad_result),
    });
    let extractor = Arc::new(SpyExtractor {
        called: AtomicBool::new(false),
        inner: MockEmbeddingExtractor::new(512),
    });
    let pipeline = VisionPipeline::new(detector, pad.clone(), extractor.clone(), config);
    Harness {
        pipeline,
        pad,
        extractor,
    }
}

/// Grey frame of 16x16 blocks alternating between `lo` and `hi` (textured, high contrast).
fn grey_checker(lo: u8, hi: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity((W * H) as usize);
    for y in 0..H {
        for x in 0..W {
            let on = ((x / 16) + (y / 16)) % 2 == 0;
            data.push(if on { hi } else { lo });
        }
    }
    data
}

/// Grey frame with a slow horizontal ramp: enough contrast, almost no local texture.
fn grey_smooth_ramp() -> Vec<u8> {
    let mut data = Vec::with_capacity((W * H) as usize);
    for _y in 0..H {
        for x in 0..W {
            data.push(96u8.saturating_add((x * 64 / W) as u8));
        }
    }
    data
}

fn grey_frame(data: Vec<u8>) -> Frame {
    Frame::new(data, W, H, 1_000_000, PixelFormat::Grey, 1)
}

fn rgb_from_grey(grey: &[u8]) -> Vec<u8> {
    grey.iter().flat_map(|&g| [g, g, g]).collect()
}

// ---------------------------------------------------------------------------
// Modality mapping
// ---------------------------------------------------------------------------

#[test]
fn test_grey_pixel_format_maps_to_monochrome_modality() {
    assert_eq!(
        PadInputModality::from_pixel_format(PixelFormat::Grey),
        PadInputModality::Monochrome
    );
    assert!(PadInputModality::from_pixel_format(PixelFormat::Grey).is_monochrome());
}

#[test]
fn test_colour_pixel_formats_map_to_colour_modality() {
    for format in [
        PixelFormat::Rgb24,
        PixelFormat::Yuyv,
        PixelFormat::Nv12,
        PixelFormat::Mjpeg,
    ] {
        assert_eq!(
            PadInputModality::from_pixel_format(format),
            PadInputModality::Color,
            "{format:?} must keep the RGB PAD path"
        );
        assert!(!PadInputModality::from_pixel_format(format).is_monochrome());
    }
}

// ---------------------------------------------------------------------------
// Constants and threshold selection
// ---------------------------------------------------------------------------

#[test]
fn test_ir_gate_constants_are_ordered_and_bounded() {
    const { assert!(IR_MIN_MEAN_LUMA > 0.0 && IR_MIN_MEAN_LUMA < IR_MAX_MEAN_LUMA) };
    const { assert!(IR_MAX_MEAN_LUMA < 255.0) };
    const { assert!(IR_MIN_LUMA_STDDEV > 0.0) };
    const { assert!(IR_MIN_TEXTURE_ENERGY > 0.0) };
    const { assert!(DEFAULT_IR_PAD_THRESHOLD > 0.0 && DEFAULT_IR_PAD_THRESHOLD <= 1.0) };
}

#[test]
fn test_default_ir_pad_threshold_is_stricter_than_rgb_threshold() {
    let config = VisionPipelineConfig::default();
    assert_eq!(config.ir_pad_threshold, DEFAULT_IR_PAD_THRESHOLD);
    assert!(
        config.ir_pad_threshold > config.pad_threshold,
        "IR liveness threshold must be stricter than the RGB threshold by default"
    );
}

#[test]
fn test_effective_pad_threshold_is_never_looser_for_monochrome() {
    let config = VisionPipelineConfig {
        pad_threshold: 0.85,
        ir_pad_threshold: 0.10,
        ..Default::default()
    };
    assert_eq!(
        config.effective_pad_threshold(PadInputModality::Color),
        0.85
    );
    assert_eq!(
        config.effective_pad_threshold(PadInputModality::Monochrome),
        0.85,
        "a misconfigured low IR threshold must never loosen the RGB threshold"
    );

    let strict = VisionPipelineConfig::default();
    assert_eq!(
        strict.effective_pad_threshold(PadInputModality::Monochrome),
        DEFAULT_IR_PAD_THRESHOLD
    );
    assert_eq!(
        strict.effective_pad_threshold(PadInputModality::Color),
        strict.pad_threshold
    );
}

// ---------------------------------------------------------------------------
// Gate unit behavior
// ---------------------------------------------------------------------------

#[test]
fn test_ir_crop_statistics_rejects_malformed_buffers() {
    assert!(matches!(
        ir_crop_statistics(&[0u8; 10], 80, 80),
        Err(VisionError::InvalidBufferSize { .. })
    ));
    assert!(matches!(
        ir_crop_statistics(&[], 0, 80),
        Err(VisionError::InvalidDimensions { .. })
    ));
    assert!(matches!(
        ir_crop_statistics(&[], 80, 0),
        Err(VisionError::InvalidDimensions { .. })
    ));
}

#[test]
fn test_ir_crop_statistics_on_flat_crop() {
    let crop = vec![128u8; 80 * 80 * 3];
    let stats = ir_crop_statistics(&crop, 80, 80).unwrap();
    assert!((stats.mean_luma - 128.0).abs() < 1e-3);
    assert!(stats.luma_stddev.abs() < 1e-3);
    assert!(stats.texture_energy.abs() < 1e-3);
}

#[test]
fn test_ir_gate_rejection_reasons() {
    let dark = vec![5u8; 80 * 80 * 3];
    assert_eq!(
        evaluate_ir_gate(&dark, 80, 80),
        Err(IrGateRejection::Underexposed)
    );

    let saturated = vec![250u8; 80 * 80 * 3];
    assert_eq!(
        evaluate_ir_gate(&saturated, 80, 80),
        Err(IrGateRejection::Overexposed)
    );

    let flat = vec![128u8; 80 * 80 * 3];
    assert_eq!(
        evaluate_ir_gate(&flat, 80, 80),
        Err(IrGateRejection::LowContrast)
    );

    // Two flat halves: strong global contrast but a single edge, i.e. no skin texture.
    let mut halves = Vec::with_capacity(80 * 80 * 3);
    for _y in 0..80 {
        for x in 0..80 {
            let v = if x < 40 { 90u8 } else { 170u8 };
            halves.extend_from_slice(&[v, v, v]);
        }
    }
    assert_eq!(
        evaluate_ir_gate(&halves, 80, 80),
        Err(IrGateRejection::LowTexture)
    );

    let mut textured = Vec::with_capacity(80 * 80 * 3);
    for y in 0..80u32 {
        for x in 0..80u32 {
            let v = if (x / 2 + y / 2) % 2 == 0 {
                90u8
            } else {
                170u8
            };
            textured.extend_from_slice(&[v, v, v]);
        }
    }
    assert_eq!(evaluate_ir_gate(&textured, 80, 80), Ok(()));
}

#[test]
fn test_ir_gate_malformed_crop_fails_closed() {
    assert_eq!(
        evaluate_ir_gate(&[1u8, 2, 3], 80, 80),
        Err(IrGateRejection::InvalidCrop)
    );
    assert_eq!(
        evaluate_ir_gate(&[], 0, 0),
        Err(IrGateRejection::InvalidCrop)
    );
}

// ---------------------------------------------------------------------------
// Pipeline: a Grey frame never silently takes the RGB path
// ---------------------------------------------------------------------------

#[test]
fn test_grey_flat_frame_is_rejected_by_ir_gate_even_if_model_says_live() {
    let h = harness(PadResult::live(0.99), VisionPipelineConfig::default());
    let err = h
        .pipeline
        .process_frame(&grey_frame(vec![128u8; (W * H) as usize]))
        .expect_err("a flat IR crop must never be accepted");
    assert!(
        matches!(
            err,
            VisionError::IrLivenessGateFailed {
                reason: IrGateRejection::LowContrast
            }
        ),
        "unexpected error: {err:?}"
    );
    assert!(!h.extractor.extractor_called());
    assert_eq!(
        h.pad.calls.load(Ordering::SeqCst),
        0,
        "the RGB-trained model must not be consulted when the IR gate rejects"
    );
}

#[test]
fn test_grey_dark_frame_is_rejected_as_underexposed() {
    let h = harness(PadResult::live(0.99), VisionPipelineConfig::default());
    let err = h
        .pipeline
        .process_frame(&grey_frame(grey_checker(2, 14)))
        .expect_err("an unlit IR crop (emitter off / screen replay) must be rejected");
    assert!(matches!(
        err,
        VisionError::IrLivenessGateFailed {
            reason: IrGateRejection::Underexposed
        }
    ));
    assert!(!h.extractor.extractor_called());
}

#[test]
fn test_grey_saturated_frame_is_rejected_as_overexposed() {
    let h = harness(PadResult::live(0.99), VisionPipelineConfig::default());
    let err = h
        .pipeline
        .process_frame(&grey_frame(grey_checker(240, 254)))
        .expect_err("a saturated IR crop must be rejected");
    assert!(matches!(
        err,
        VisionError::IrLivenessGateFailed {
            reason: IrGateRejection::Overexposed
        }
    ));
}

#[test]
fn test_grey_smooth_frame_is_rejected_as_low_texture() {
    let h = harness(PadResult::live(0.99), VisionPipelineConfig::default());
    let err = h
        .pipeline
        .process_frame(&grey_frame(grey_smooth_ramp()))
        .expect_err("a texture-less IR crop must be rejected");
    assert!(
        matches!(
            err,
            VisionError::IrLivenessGateFailed {
                reason: IrGateRejection::LowTexture
            }
        ),
        "unexpected error: {err:?}"
    );
}

#[test]
fn test_grey_frame_uses_stricter_ir_threshold() {
    // 0.90 passes the RGB threshold (0.85) but not the IR threshold.
    let h = harness(PadResult::live(0.90), VisionPipelineConfig::default());
    let err = h
        .pipeline
        .process_frame(&grey_frame(grey_checker(90, 170)))
        .expect_err("a Grey frame must be scored against the IR threshold");
    match err {
        VisionError::PadFailed { score, threshold } => {
            assert!((score - 0.90).abs() < 1e-6);
            assert_eq!(threshold, DEFAULT_IR_PAD_THRESHOLD);
        }
        other => panic!("Expected PadFailed with the IR threshold, got {other:?}"),
    }
    assert!(!h.extractor.extractor_called());
}

#[test]
fn test_grey_frame_threshold_never_looser_than_rgb_threshold() {
    let config = VisionPipelineConfig {
        pad_threshold: 0.99,
        ir_pad_threshold: 0.50,
        ..Default::default()
    };
    let h = harness(PadResult::live(0.97), config);
    let err = h
        .pipeline
        .process_frame(&grey_frame(grey_checker(90, 170)))
        .expect_err("IR threshold must never be looser than pad_threshold");
    assert!(matches!(
        err,
        VisionError::PadFailed { threshold, .. } if (threshold - 0.99).abs() < 1e-6
    ));
}

#[test]
fn test_grey_frame_non_finite_thresholds_fail_closed() {
    let config = VisionPipelineConfig {
        pad_threshold: f32::NAN,
        ir_pad_threshold: f32::NAN,
        ..Default::default()
    };
    let h = harness(PadResult::live(0.999), config);
    let res = h.pipeline.process_frame(&grey_frame(grey_checker(90, 170)));
    assert!(
        matches!(res, Err(VisionError::PadFailed { .. })),
        "non-finite thresholds must reject, never accept"
    );
}

#[test]
fn test_grey_frame_non_finite_score_fails_closed() {
    let h = harness(PadResult::live(f32::NAN), VisionPipelineConfig::default());
    let res = h.pipeline.process_frame(&grey_frame(grey_checker(90, 170)));
    assert!(matches!(res, Err(VisionError::PadFailed { .. })));
}

#[test]
fn test_grey_frame_model_spoof_is_rejected() {
    let h = harness(
        PadResult::spoof(0.99, soos_inference_ort::pad::AttackType::ScreenReplay),
        VisionPipelineConfig::default(),
    );
    let res = h.pipeline.process_frame(&grey_frame(grey_checker(90, 170)));
    assert!(matches!(res, Err(VisionError::PadFailed { .. })));
}

#[test]
fn test_grey_textured_frame_with_high_liveness_is_accepted() {
    let h = harness(PadResult::live(0.99), VisionPipelineConfig::default());
    let output = h
        .pipeline
        .process_frame(&grey_frame(grey_checker(90, 170)))
        .expect("a well-exposed, textured IR crop scored above the IR threshold passes");
    assert!(output.pad_result.is_live);
    assert_eq!(h.pad.calls.load(Ordering::SeqCst), 1);
    assert!(h.extractor.extractor_called());
}

#[test]
fn test_rgb_frame_keeps_rgb_threshold_and_skips_ir_gate() {
    // The same flat content that the IR gate rejects is untouched on the colour path.
    let h = harness(PadResult::live(0.90), VisionPipelineConfig::default());
    let flat_rgb = Frame::new(
        rgb_from_grey(&vec![128u8; (W * H) as usize]),
        W,
        H,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    );
    let output = h
        .pipeline
        .process_frame(&flat_rgb)
        .expect("colour frames keep the existing RGB PAD behavior");
    assert!(output.pad_result.is_live);
}

// ---------------------------------------------------------------------------
// GUI analysis path
// ---------------------------------------------------------------------------

#[test]
fn test_analyze_frame_marks_grey_gate_failure_as_not_live() {
    let h = harness(PadResult::live(0.99), VisionPipelineConfig::default());
    let analysis = h
        .pipeline
        .analyze_frame(&grey_frame(vec![128u8; (W * H) as usize]))
        .expect("analysis never short-circuits");
    let pad = analysis.pad_result.clone().expect("PAD result is reported");
    assert!(!pad.is_live, "IR gate failure must be reported as not live");
}

#[test]
fn test_analyze_frame_marks_grey_score_below_ir_threshold_as_not_live() {
    let h = harness(PadResult::live(0.90), VisionPipelineConfig::default());
    let analysis = h
        .pipeline
        .analyze_frame(&grey_frame(grey_checker(90, 170)))
        .expect("analysis never short-circuits");
    let pad = analysis.pad_result.clone().expect("PAD result is reported");
    assert!(!pad.is_live);
}

#[test]
fn test_analyze_frame_keeps_rgb_result_unchanged() {
    let h = harness(PadResult::live(0.90), VisionPipelineConfig::default());
    let flat_rgb = Frame::new(
        rgb_from_grey(&vec![128u8; (W * H) as usize]),
        W,
        H,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    );
    let analysis = h.pipeline.analyze_frame(&flat_rgb).unwrap();
    assert_eq!(analysis.pad_result, Some(PadResult::live(0.90)));
}

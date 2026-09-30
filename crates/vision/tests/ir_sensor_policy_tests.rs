//! Contract tests: the IR PAD policy keys on the sensor type, not only on the pixel format
//! (candid review finding 2 on GitHub #169).
//!
//! An IR node picked by the resolver (card name classified `Infrared`) that advertises a colour
//! format used to be negotiated as YUYV/MJPEG. `PadInputModality::from_pixel_format(Yuyv)` is
//! `Color`, so the IR gate and the stricter IR threshold were skipped and the RGB-trained
//! MiniFASNet scored near-infrared data at the RGB threshold. Every frame stamped
//! `SensorType::Infrared` must now take the IR policy whatever its pixel format.

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

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat, SensorType};
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_inference_ort::pad::{PadDetector, PadResult};
use soos_vision::{
    IrGateRejection, PadInputModality, VisionError, VisionPipeline, VisionPipelineConfig,
    DEFAULT_IR_PAD_THRESHOLD,
};

const W: u32 = 640;
const H: u32 = 480;

const ALL_FORMATS: [PixelFormat; 5] = [
    PixelFormat::Rgb24,
    PixelFormat::Yuyv,
    PixelFormat::Nv12,
    PixelFormat::Mjpeg,
    PixelFormat::Grey,
];

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

fn pipeline(pad_result: PadResult) -> (VisionPipeline, Arc<CountingPad>) {
    let detector = Arc::new(MockFaceDetector::new_centered_face(W, H, 0.95));
    let pad = Arc::new(CountingPad {
        calls: AtomicUsize::new(0),
        inner: MockPadDetector::new_with_result(pad_result),
    });
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let pipeline = VisionPipeline::new(
        detector,
        pad.clone(),
        extractor,
        VisionPipelineConfig::default(),
    );
    (pipeline, pad)
}

/// Encodes a luma plane as YUYV 4:2:2 with neutral chroma (U = V = 128).
fn yuyv_from_luma(luma: &[u8]) -> Vec<u8> {
    luma.chunks_exact(2)
        .flat_map(|pair| [pair[0], 128, pair[1], 128])
        .collect()
}

/// 16x16 checkerboard luma plane (well exposed, contrasted and textured).
fn checker_luma(lo: u8, hi: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity((W * H) as usize);
    for y in 0..H {
        for x in 0..W {
            let on = ((x / 16) + (y / 16)) % 2 == 0;
            data.push(if on { hi } else { lo });
        }
    }
    data
}

fn ir_yuyv_frame(luma: &[u8]) -> Frame {
    Frame::new(yuyv_from_luma(luma), W, H, 1_000_000, PixelFormat::Yuyv, 1)
        .with_sensor_type(SensorType::Infrared)
}

#[test]
fn test_frame_new_defaults_to_unknown_sensor_and_tag_is_applied() {
    let frame = Frame::new(vec![0u8; 4], 2, 1, 0, PixelFormat::Yuyv, 0);
    assert_eq!(frame.sensor_type, SensorType::Unknown);
    let tagged = frame.with_sensor_type(SensorType::Infrared);
    assert_eq!(tagged.sensor_type, SensorType::Infrared);
    assert_eq!(
        tagged.format,
        PixelFormat::Yuyv,
        "tagging never alters the format"
    );
}

#[test]
fn test_infrared_sensor_is_monochrome_modality_for_every_pixel_format() {
    for format in ALL_FORMATS {
        assert_eq!(
            PadInputModality::for_source(format, SensorType::Infrared),
            PadInputModality::Monochrome,
            "an IR sensor streaming {format:?} must take the IR PAD policy"
        );
    }
}

#[test]
fn test_rgb_and_unknown_sensors_keep_format_based_modality() {
    for sensor in [SensorType::Rgb, SensorType::Unknown] {
        for format in ALL_FORMATS {
            assert_eq!(
                PadInputModality::for_source(format, sensor),
                PadInputModality::from_pixel_format(format),
                "{sensor:?} sensor streaming {format:?}"
            );
        }
    }
}

#[test]
fn test_for_frame_uses_the_frame_sensor_tag() {
    let frame = ir_yuyv_frame(&checker_luma(90, 170));
    assert_eq!(
        PadInputModality::for_frame(&frame),
        PadInputModality::Monochrome
    );
    let untagged = Frame::new(vec![0u8; 4], 2, 1, 0, PixelFormat::Yuyv, 0);
    assert_eq!(
        PadInputModality::for_frame(&untagged),
        PadInputModality::Color
    );
}

#[test]
fn test_ir_sensor_streaming_yuyv_flat_frame_is_rejected_by_ir_gate() {
    let (pipeline, pad) = pipeline(PadResult::live(0.99));
    let err = pipeline
        .process_frame(&ir_yuyv_frame(&vec![128u8; (W * H) as usize]))
        .expect_err("a flat crop from an IR sensor must never be accepted, even in YUYV");
    assert!(
        matches!(
            err,
            VisionError::IrLivenessGateFailed {
                reason: IrGateRejection::LowContrast
            }
        ),
        "unexpected error: {err:?}"
    );
    assert_eq!(
        pad.calls.load(Ordering::SeqCst),
        0,
        "the RGB-trained model must not be consulted when the IR gate rejects"
    );
}

#[test]
fn test_ir_sensor_streaming_yuyv_uses_stricter_ir_threshold() {
    // 0.90 passes the RGB threshold (0.85) but not the IR threshold (0.95).
    let (pipeline, _pad) = pipeline(PadResult::live(0.90));
    let err = pipeline
        .process_frame(&ir_yuyv_frame(&checker_luma(90, 170)))
        .expect_err("an IR sensor streaming YUYV must be scored against the IR threshold");
    match err {
        VisionError::PadFailed { score, threshold } => {
            assert!((score - 0.90).abs() < 1e-6);
            assert_eq!(threshold, DEFAULT_IR_PAD_THRESHOLD);
        }
        other => panic!("Expected PadFailed with the IR threshold, got {other:?}"),
    }
}

#[test]
fn test_ir_sensor_streaming_yuyv_textured_high_liveness_is_accepted() {
    let (pipeline, pad) = pipeline(PadResult::live(0.99));
    let output = pipeline
        .process_frame(&ir_yuyv_frame(&checker_luma(90, 170)))
        .expect("a textured IR capture above the IR threshold passes");
    assert!(output.pad_result.is_live);
    assert_eq!(pad.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn test_analyze_frame_ir_sensor_yuyv_below_ir_threshold_is_not_live() {
    let (pipeline, _pad) = pipeline(PadResult::live(0.90));
    let analysis = pipeline
        .analyze_frame(&ir_yuyv_frame(&checker_luma(90, 170)))
        .expect("analysis never short-circuits");
    let pad = analysis.pad_result.clone().expect("PAD result is reported");
    assert!(
        !pad.is_live,
        "the GUI must never show an IR capture as live when the daemon would reject it"
    );
}

//! Real-model validation of the PAD crop geometry and multi-scale fusion
//! (review findings PAD-07 / PAD-08, GitHub #212 / #213).
//!
//! Loads the attested `minifasnet_v2_80x80.onnx` (committed `models/manifest.toml` SHA-256)
//! and checks that the pipeline PAD crop matches the real graph input, and that the fused
//! multi-scale decision is the mean of the real per-crop scores.
//!
//! Gating: `SOOS_MODELS_DIR`, falling back to `/var/lib/soos/models`. Without the model the
//! tests print `SKIPPED` and return, unless `SOOS_REQUIRE_REAL_MODELS=1`.
//! Only synthetic, non-biometric frames are used.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::print_stdout,
    reason = "Evidence test suite uses assertions, unwrap and prints measurements"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector};
use soos_inference_ort::pad::{OrtPadDetector, PadDetector};
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig, SharedSession};
use soos_inference_ort::{BoundingBox, FaceDetection};
use soos_vision::crop::crop_pad_context;
use soos_vision::{VisionError, VisionPipeline, VisionPipelineConfig};

const PAD_MODEL_ID: &str = "minifasnet_v2_pad";
const PAD_MODEL_FILE: &str = "minifasnet_v2_80x80.onnx";
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
const FRAME_W: u32 = 640;
const FRAME_H: u32 = 480;

fn load_real_pad_session(test_name: &str) -> Option<SharedSession> {
    let dir = std::env::var_os("SOOS_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR));
    if !dir.join(PAD_MODEL_FILE).is_file() {
        assert!(
            std::env::var("SOOS_REQUIRE_REAL_MODELS").map_or(true, |v| v != "1"),
            "SOOS_REQUIRE_REAL_MODELS=1 but {PAD_MODEL_FILE} is missing from {}",
            dir.display()
        );
        println!(
            "SKIPPED {test_name}: {PAD_MODEL_FILE} not found in {}",
            dir.display()
        );
        return None;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    let mut registry = ModelRegistry::new(RegistryConfig::with_manifest(&dir, manifest))
        .expect("committed manifest parses");
    Some(
        registry
            .get_or_load_session(PAD_MODEL_ID)
            .expect("installed PAD model matches the committed manifest and loads"),
    )
}

fn synthetic_frame() -> Frame {
    let mut data = Vec::with_capacity((FRAME_W * FRAME_H * 3) as usize);
    for y in 0..FRAME_H {
        for x in 0..FRAME_W {
            let on = ((x / 16) + (y / 16)) % 2 == 0;
            data.push(if on { 200 } else { (x % 256) as u8 });
            data.push(((y * 255) / FRAME_H) as u8);
            data.push(if on { 40 } else { 160 });
        }
    }
    Frame::new(data, FRAME_W, FRAME_H, 1_000_000, PixelFormat::Rgb24, 1)
}

fn face_box() -> BoundingBox {
    // Near the right edge so the 4.0 crop is shifted inward and scale-capped.
    BoundingBox::new(470.0, 150.0, 590.0, 300.0)
}

#[test]
fn test_real_pad_model_input_matches_pipeline_pad_crop() {
    let Some(session) = load_real_pad_session("test_real_pad_model_input_matches_pipeline") else {
        return;
    };
    let dims: Vec<i64> = {
        let guard = session.lock().expect("session mutex");
        guard.inputs()[0]
            .dtype()
            .tensor_shape()
            .expect("tensor input")
            .to_vec()
    };
    let config = VisionPipelineConfig::default();
    println!("REAL PAD MODEL input dims {dims:?}");
    assert_eq!(dims.len(), 4, "NCHW input");
    assert_eq!(dims[1], 3);
    assert_eq!(dims[2], i64::from(config.pad_target_height));
    assert_eq!(dims[3], i64::from(config.pad_target_width));

    let frame = synthetic_frame();
    for scale in [2.7f32, 4.0] {
        let crop = crop_pad_context(
            &frame.data,
            FRAME_W,
            FRAME_H,
            &face_box(),
            scale,
            config.pad_target_width,
            config.pad_target_height,
        )
        .expect("crop");
        assert_eq!(
            crop.len(),
            usize::try_from(dims[2] * dims[3] * dims[1]).expect("positive dims"),
            "crop at scale {scale} must fill the real graph input exactly"
        );
    }
}

#[test]
fn test_real_pad_multiscale_fusion_is_mean_of_real_scores() {
    let Some(session) = load_real_pad_session("test_real_pad_multiscale_fusion") else {
        return;
    };
    let config = VisionPipelineConfig::default();
    let real_pad = Arc::new(OrtPadDetector::new(session, config.pad_threshold));
    let frame = synthetic_frame();

    let mut scores = Vec::new();
    for scale in [2.7f32, 4.0] {
        let crop = crop_pad_context(&frame.data, FRAME_W, FRAME_H, &face_box(), scale, 80, 80)
            .expect("crop");
        let r = real_pad.evaluate_liveness(&crop, 80, 80).expect("real PAD");
        println!(
            "REAL PAD scale {scale}: live={} score={:.6}",
            r.is_live, r.score
        );
        assert!(r.score.is_finite() && (0.0..=1.0).contains(&r.score));
        scores.push(r.score);
    }

    // The attested set holds one PAD model; the 2.7-trained V2 stands in for the 4.0 member
    // to exercise the fusion wiring on real outputs (GitHub #212 follow-up: V1SE model).
    let detection = FaceDetection::with_landmarks(
        face_box(),
        0.95,
        MockFaceDetector::canonical_landmarks_for_box(&face_box()),
    );
    let pipeline = VisionPipeline::new(
        Arc::new(MockFaceDetector::new_with_detections(vec![detection])),
        real_pad.clone(),
        Arc::new(MockEmbeddingExtractor::new(512)),
        config,
    )
    .with_additional_pad_model(4.0, real_pad)
    .expect("valid member");

    let expected = (scores[0] + scores[1]) / 2.0;
    match pipeline.process_frame(&frame) {
        Err(VisionError::PadFailed { score, .. }) => {
            assert!(
                (score - expected).abs() < 1e-6,
                "fused score {score} must be the mean {expected} of the real scores"
            );
        }
        other => panic!("a synthetic non-face must never pass real PAD, got {other:?}"),
    }
}

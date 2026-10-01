//! Non-face frames never pass the production vision path (GitHub #212, PVA11 / PVA12;
//! owner decision 2026-10-01, walkthrough 161).
//!
//! With the upstream `[0, 255]` PAD input range, MiniFASNet alone scores some non-face patterns
//! live (the 160x160 checkerboard fixture of `pad_real_model_tests` reaches `p_live` 0.995). The
//! production path rejects them earlier: the real SCRFD detector finds no face, so
//! `VisionPipeline::process_frame` returns `NoFaceDetected` and no PAD verdict is ever produced.
//!
//! The control case bypasses detection (mock detector reporting a face box) on the same frames
//! and shows that PAD alone then accepts at least one of them: the detector is the gate, so the
//! main assertion would fail if detection were bypassed.
//!
//! Gating like the other real-model tests: `SOOS_MODELS_DIR` (default `/var/lib/soos/models`),
//! `SKIPPED` without the models unless `SOOS_REQUIRE_REAL_MODELS=1`. Synthetic frames only.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::print_stdout,
    reason = "Evidence test suite uses assertions, unwrap and prints measurements"
)]

use std::path::PathBuf;
use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat};
use soos_daemon::pipeline::{build_pad_detector, EMBEDDING_MODEL_ID, PAD_MODEL_ID};
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector};
use soos_inference_ort::{
    BoundingBox, FaceDetection, ModelRegistry, OrtEmbeddingExtractor, OrtScrfdDetector,
    RegistryConfig,
};
use soos_vision::crop::pad_crop_window;
use soos_vision::{VisionError, VisionPipeline, VisionPipelineConfig};

const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
const W: u32 = 640;
const H: u32 = 480;
const REQUIRED_FILES: [&str; 3] = [
    "scrfd_500m_kps.onnx",
    "minifasnet_v2_80x80.onnx",
    "arcface_w600k_mbf.onnx",
];

fn models_dir(test: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("SOOS_MODELS_DIR")
        .map_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR), PathBuf::from);
    if REQUIRED_FILES.iter().all(|f| dir.join(f).is_file()) {
        return Some(dir);
    }
    assert!(
        std::env::var("SOOS_REQUIRE_REAL_MODELS").map_or(true, |v| v != "1"),
        "SOOS_REQUIRE_REAL_MODELS=1 but models are missing from {}",
        dir.display()
    );
    println!("SKIPPED {test}: models not found in {}", dir.display());
    None
}

/// Face box reported by the detection-bypass control.
fn control_face_box() -> BoundingBox {
    BoundingBox::new(250.0, 150.0, 370.0, 300.0)
}

/// Synthetic RGB24 frames: the `pad_real_model_tests` patterns tiled to 640x480.
fn frames() -> Vec<(&'static str, Frame)> {
    let make = |f: &dyn Fn(usize, usize) -> [u8; 3]| {
        let mut data = Vec::with_capacity((W * H * 3) as usize);
        for y in 0..H as usize {
            for x in 0..W as usize {
                data.extend_from_slice(&f(x, y));
            }
        }
        Frame::new(data, W, H, 1_000_000, PixelFormat::Rgb24, 1)
    };
    vec![
        ("uniform_grey", make(&|_, _| [128, 128, 128])),
        (
            "gradient",
            make(&|x, y| {
                let (x, y) = (x % 80, y % 80);
                [(x * 3) as u8, (y * 3) as u8, ((x + y) * 3 / 2) as u8]
            }),
        ),
        (
            "checkerboard_10px",
            make(&|x, y| {
                let v: u8 = if ((x / 10) + (y / 10)) % 2 == 0 {
                    220
                } else {
                    30
                };
                [v, v / 2, 255 - v]
            }),
        ),
        (
            "checkerboard_20px",
            make(&|x, y| {
                let v: u8 = if ((x / 20) + (y / 20)) % 2 == 0 {
                    220
                } else {
                    30
                };
                [v, v / 2, 255 - v]
            }),
        ),
        (
            "checkerboard_25px",
            make(&|x, y| {
                let v: u8 = if ((x / 25) + (y / 25)) % 2 == 0 {
                    220
                } else {
                    30
                };
                [v, v / 2, 255 - v]
            }),
        ),
        (
            "checkerboard_30px",
            make(&|x, y| {
                let v: u8 = if ((x / 30) + (y / 30)) % 2 == 0 {
                    220
                } else {
                    30
                };
                [v, v / 2, 255 - v]
            }),
        ),
        (
            // 16x16 squares across the 2.7x PAD window of `control_face_box`: once cropped and
            // resized to 80x80 it is the `checkerboard_160` fixture of `pad_real_model_tests`
            // (5-pixel squares), which PAD alone scores live.
            "checkerboard_pad_window",
            {
                let w = pad_crop_window(&control_face_box(), 2.7, W, H).unwrap();
                make(&move |x, y| {
                    let u = (x as i64 - i64::from(w.x)) * 16 / i64::from(w.width);
                    let v = (y as i64 - i64::from(w.y)) * 16 / i64::from(w.height);
                    let c: u8 = if (u + v).rem_euclid(2) == 0 { 220 } else { 30 };
                    [c, c / 2, 255 - c]
                })
            },
        ),
        (
            "checkerboard_40px",
            make(&|x, y| {
                let v: u8 = if ((x / 40) + (y / 40)) % 2 == 0 {
                    220
                } else {
                    30
                };
                [v, v / 2, 255 - v]
            }),
        ),
    ]
}

/// PVA12: synthetic non-face frames through real SCRFD -> real PAD -> real embedding never
/// produce a `PipelineOutput` (no live verdict); with detection bypassed, PAD alone accepts at
/// least one of them (control).
#[test]
fn test_pva12_real_pipeline_never_accepts_synthetic_nonface_frames() {
    let Some(dir) = models_dir("test_pva12_real_pipeline_never_accepts_synthetic_nonface_frames")
    else {
        return;
    };
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    let mut registry = ModelRegistry::new(RegistryConfig::with_manifest(&dir, manifest)).unwrap();
    let config = VisionPipelineConfig::default();
    let detector = Arc::new(
        OrtScrfdDetector::new(
            registry.get_or_load_session("scrfd_500m_kps").unwrap(),
            config.min_face_confidence,
            config.nms_iou_threshold,
        )
        .unwrap(),
    );
    let pad = Arc::new(build_pad_detector(
        registry.get_or_load_session(PAD_MODEL_ID).unwrap(),
        config.pad_threshold,
    ));
    let extractor = Arc::new(OrtEmbeddingExtractor::new(
        registry.get_or_load_session(EMBEDDING_MODEL_ID).unwrap(),
    ));
    let production = VisionPipeline::new(detector, pad.clone(), extractor, config.clone());

    let face = control_face_box();
    let bypass = VisionPipeline::new(
        Arc::new(MockFaceDetector::new_with_detections(vec![
            FaceDetection::with_landmarks(
                face,
                0.95,
                MockFaceDetector::canonical_landmarks_for_box(&face),
            ),
        ])),
        pad,
        Arc::new(MockEmbeddingExtractor::new(512)),
        config,
    );

    let mut bypass_accepted = 0usize;
    for (name, frame) in frames() {
        let result = production.process_frame(&frame);
        println!(
            "REAL PIPELINE {name}: {:?}",
            result.as_ref().map(|_| "OUTPUT").map_err(|e| e.to_string())
        );
        assert!(
            matches!(result, Err(VisionError::NoFaceDetected)),
            "{name}: a synthetic non-face frame must stop at detection (NoFaceDetected)"
        );

        let control = bypass.process_frame(&frame);
        match &control {
            Ok(out) => {
                println!("BYPASS {name}: PAD live, score {:.4}", out.pad_result.score);
                bypass_accepted += 1;
            }
            Err(e) => println!("BYPASS {name}: {e}"),
        }
    }
    assert!(
        bypass_accepted > 0,
        "control: with detection bypassed PAD alone must accept a synthetic pattern, otherwise \
         this test does not prove that detection is the gate"
    );
}

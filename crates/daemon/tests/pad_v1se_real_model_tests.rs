//! Second PAD model (4.0x MiniFASNetV1SE) attestation and fused real-model evidence
//! (GitHub #212, review finding PAD-07; rows PVA5–PVA7).
//!
//! The model is attested in `models/optional_models.toml` and disabled by default. These tests
//! check that the optional entry, appended to the shipped manifest exactly as an operator would
//! enable it, wires the member at the upstream 4.0 scale; that the 4.0x crop window equals
//! upstream `CropImage._get_new_box`; and (ignored, opt-in) the fused latency with the real files.
//!
//! Only synthetic, non-biometric frames are used. Kept in its own test binary (ORT sessions).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::print_stdout,
    reason = "Evidence test suite uses assertions, unwrap and prints measurements"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use soos_camera_v4l::{Frame, PixelFormat};
use soos_daemon::pipeline::{
    attach_optional_pad_members, build_pad_detector, optional_pad_members, validate_pad_detector,
    PAD_MODEL_ID, SECONDARY_PAD_BBOX_SCALE, SECONDARY_PAD_MODEL_ID,
};
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector};
use soos_inference_ort::pad::PadDetector;
use soos_inference_ort::{
    BoundingBox, FaceDetection, ModelManifest, ModelRegistry, RegistryConfig, TensorLayout,
};
use soos_vision::crop::{crop_pad_context, pad_crop_window};
use soos_vision::{VisionError, VisionPipeline, VisionPipelineConfig};

const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
const FRAME_W: u32 = 640;
const FRAME_H: u32 = 480;
const WARMUP_RUNS: usize = 10;
const TIMED_RUNS: usize = 200;

fn repo_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The `[models.minifasnet_v1se_pad]` table of the optional file, without its `[manifest]`
/// header: exactly what an operator appends to the deployed manifest to enable the member.
fn optional_v1se_table() -> String {
    let optional = repo_file("models/optional_models.toml");
    let header = format!("[models.{SECONDARY_PAD_MODEL_ID}]");
    let start = optional.find(&header).expect("optional V1SE table");
    let rest = &optional[start..];
    let end = rest[1..].find("\n[").map_or(rest.len(), |i| i + 1);
    rest[..end].to_string()
}

/// Shipped manifest with the optional member enabled.
fn enabled_manifest_source() -> String {
    format!(
        "{}\n{}",
        repo_file("models/manifest.toml"),
        optional_v1se_table()
    )
}

/// PVA5: the attested optional entry is a well-formed manifest entry for the daemon's optional
/// member: same id, 80x80 NCHW input, 3-class head, and once appended to the shipped manifest
/// it is the single optional member, at the upstream 4.0 scale. Disabled while not appended.
#[test]
fn test_pva_optional_entry_enables_v1se_at_upstream_scale_only_when_appended() {
    let optional = ModelManifest::from_toml_str(&repo_file("models/optional_models.toml"))
        .expect("optional_models.toml uses the manifest schema");
    let meta = optional
        .get_model(SECONDARY_PAD_MODEL_ID)
        .expect("optional V1SE entry");
    assert_eq!(meta.input_shape, vec![1, 3, 80, 80]);
    assert!(meta.input_layout_declared);
    assert_eq!(meta.input_layout, TensorLayout::Nchw);
    assert_eq!(meta.output_shapes, vec![vec![1, 3]]);
    assert_eq!(meta.license, "Apache-2.0");

    let shipped = ModelManifest::from_toml_str(&repo_file("models/manifest.toml")).unwrap();
    assert!(
        optional_pad_members(&shipped).is_empty(),
        "disabled by default"
    );

    let enabled = ModelManifest::from_toml_str(&enabled_manifest_source())
        .expect("appending the optional table keeps a valid manifest");
    assert_eq!(enabled.get_model(SECONDARY_PAD_MODEL_ID), Some(meta));
    let members = optional_pad_members(&enabled);
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].0, SECONDARY_PAD_MODEL_ID);
    assert_eq!(members[0].1.to_bits(), SECONDARY_PAD_BBOX_SCALE.to_bits());
}

/// Upstream `CropImage._get_new_box(src_w, src_h, [x, y, w, h], 4.0)` windows, generated with
/// `src/generate_patches.py` at commit b6d5f04 (walkthrough 161): `(frame w, frame h,
/// [x1, y1, x2, y2], [x, y, width, height])`, width/height of the inclusive slice.
const UPSTREAM_4_0_WINDOWS: [(u32, u32, [f32; 4], [u32; 4]); 8] = [
    (640, 480, [250.0, 150.0, 370.0, 300.0], [118, 0, 384, 480]),
    (640, 480, [470.0, 150.0, 590.0, 300.0], [255, 0, 385, 480]),
    (640, 480, [10.0, 20.0, 110.0, 150.0], [0, 0, 369, 480]),
    (
        640,
        480,
        [300.5, 200.25, 380.75, 310.5],
        [180, 34, 322, 442],
    ),
    (640, 480, [200.0, 100.0, 440.0, 400.0], [128, 0, 384, 480]),
    (1280, 720, [600.0, 250.0, 760.0, 450.0], [392, 0, 576, 720]),
    (
        1280,
        720,
        [1100.0, 500.0, 1250.0, 700.0],
        [739, 0, 541, 720],
    ),
    (320, 240, [140.0, 90.0, 180.0, 140.0], [80, 15, 161, 201]),
];

/// PVA6: the 4.0x context window used for the second member equals upstream's for centred,
/// edge-shifted, corner, fractional, scale-capped, 720p and small-frame boxes.
#[test]
fn test_pva_v1se_crop_window_matches_upstream_get_new_box() {
    for (w, h, [x1, y1, x2, y2], expected) in UPSTREAM_4_0_WINDOWS {
        let window = pad_crop_window(
            &BoundingBox::new(x1, y1, x2, y2),
            SECONDARY_PAD_BBOX_SCALE,
            w,
            h,
        )
        .expect("non-degenerate window");
        assert_eq!(
            [window.x, window.y, window.width, window.height],
            expected,
            "frame {w}x{h}, box [{x1}, {y1}, {x2}, {y2}]"
        );
    }
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
    BoundingBox::new(250.0, 150.0, 370.0, 300.0)
}

fn median_and_p95(mut samples: Vec<Duration>) -> (f64, f64) {
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let p95 = (samples.len() * 95 / 100).min(samples.len() - 1);
    (ms(samples[samples.len() / 2]), ms(samples[p95]))
}

fn time_process_frame(pipeline: &VisionPipeline, frame: &Frame) -> Vec<Duration> {
    for _ in 0..WARMUP_RUNS {
        let _ = pipeline.process_frame(frame);
    }
    (0..TIMED_RUNS)
        .map(|_| {
            let start = Instant::now();
            let _ = pipeline.process_frame(frame);
            start.elapsed()
        })
        .collect()
}

/// PVA7 (ignored, opt-in): the real 2.7x V2 + 4.0x V1SE ensemble through the daemon's own
/// wiring (`attach_optional_pad_members`) on a registry whose manifest is the shipped manifest
/// with the optional table appended. Checks the attested files load, the fused score is the
/// mean of the real per-member scores, and prints the added per-frame latency.
///
/// Run: `SOOS_PAD_V1SE_DIR=<dir holding minifasnet_v1se_80x80.onnx> cargo test --locked
/// -p soos-daemon --test pad_v1se_real_model_tests -- --ignored --nocapture`
/// (`SOOS_MODELS_DIR` holds `minifasnet_v2_80x80.onnx`, default `/var/lib/soos/models`).
#[test]
#[ignore = "needs the real V2 and V1SE ONNX files (SOOS_PAD_V1SE_DIR)"]
fn test_pva_real_fused_v2_v1se_latency_and_mean() {
    let Some(v1se_dir) = std::env::var_os("SOOS_PAD_V1SE_DIR").map(PathBuf::from) else {
        println!("SKIPPED test_pva_real_fused_v2_v1se_latency_and_mean: SOOS_PAD_V1SE_DIR not set");
        return;
    };
    let models_dir = std::env::var_os("SOOS_MODELS_DIR")
        .map_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR), PathBuf::from);
    let shipped = ModelManifest::from_toml_str(&repo_file("models/manifest.toml")).unwrap();
    let enabled = ModelManifest::from_toml_str(&enabled_manifest_source()).unwrap();
    let v2_file = &shipped.get_model(PAD_MODEL_ID).unwrap().filename;
    let v1se_file = &enabled.get_model(SECONDARY_PAD_MODEL_ID).unwrap().filename;

    // Registry over a scratch directory holding only the two PAD files and the enabled manifest.
    let dir = tempfile::tempdir().unwrap();
    let copy = |from: &Path, name: &str| {
        std::fs::copy(from.join(name), dir.path().join(name))
            .unwrap_or_else(|e| panic!("copy {name} from {}: {e}", from.display()));
    };
    copy(&models_dir, v2_file);
    copy(&v1se_dir, v1se_file);
    let manifest = format!(
        "[manifest]\nversion = \"2.0.0\"\n\n[models.{PAD_MODEL_ID}]\n{}\n{}",
        pad_table_body(&repo_file("models/manifest.toml"), PAD_MODEL_ID),
        optional_v1se_table()
    );
    std::fs::write(dir.path().join("manifest.toml"), manifest).unwrap();
    let mut registry = ModelRegistry::new(RegistryConfig::new(dir.path())).unwrap();
    registry
        .verify_integrity()
        .expect("both PAD files match their attested SHA-256");

    let config = VisionPipelineConfig::default();
    let primary = build_pad_detector(
        registry.get_or_load_session(PAD_MODEL_ID).unwrap(),
        config.pad_threshold,
    );
    validate_pad_detector(&primary, registry.manifest()).expect("V2 self-test");
    let primary = Arc::new(primary);

    let detection = FaceDetection::with_landmarks(
        face_box(),
        0.95,
        MockFaceDetector::canonical_landmarks_for_box(&face_box()),
    );
    let single = VisionPipeline::new(
        Arc::new(MockFaceDetector::new_with_detections(vec![
            detection.clone()
        ])),
        primary.clone(),
        Arc::new(MockEmbeddingExtractor::new(512)),
        config.clone(),
    );
    let fused = attach_optional_pad_members(
        VisionPipeline::new(
            Arc::new(MockFaceDetector::new_with_detections(vec![detection])),
            primary.clone(),
            Arc::new(MockEmbeddingExtractor::new(512)),
            config.clone(),
        ),
        &mut registry,
        config.pad_threshold,
    )
    .expect("the attested V1SE member is wired");
    let scales = fused.pad_scales();
    assert_eq!(scales.len(), 2, "{scales:?}");
    assert_eq!(scales[1].to_bits(), SECONDARY_PAD_BBOX_SCALE.to_bits());

    // Fused score == mean of the real member scores on their own crops.
    let frame = synthetic_frame();
    let secondary = build_pad_detector(
        registry
            .get_or_load_session(SECONDARY_PAD_MODEL_ID)
            .unwrap(),
        config.pad_threshold,
    );
    let mut member_scores = Vec::new();
    for (scale, pad) in [
        (config.pad_bbox_scale, primary.as_ref() as &dyn PadDetector),
        (SECONDARY_PAD_BBOX_SCALE, &secondary as &dyn PadDetector),
    ] {
        let crop =
            crop_pad_context(&frame.data, FRAME_W, FRAME_H, &face_box(), scale, 80, 80).unwrap();
        let r = pad.evaluate_liveness(&crop, 80, 80).unwrap();
        println!(
            "REAL PAD member scale {scale}: live={} p_live={:.6}",
            r.is_live, r.score
        );
        member_scores.push(r.score);
    }
    let expected = (member_scores[0] + member_scores[1]) / 2.0;
    match fused.process_frame(&frame) {
        Err(VisionError::PadFailed { score, .. }) => {
            assert!(
                (score - expected).abs() < 1e-6,
                "fused {score} != mean {expected}"
            );
        }
        other => panic!("a synthetic non-face must never pass real PAD, got {other:?}"),
    }

    let (single_median, single_p95) = median_and_p95(time_process_frame(&single, &frame));
    let (fused_median, fused_p95) = median_and_p95(time_process_frame(&fused, &frame));
    println!(
        "REAL PAD latency per frame ({TIMED_RUNS} runs, {}x{} frame): single V2 median \
         {single_median:.3} ms p95 {single_p95:.3} ms; fused V2+V1SE median {fused_median:.3} ms \
         p95 {fused_p95:.3} ms; added median {:.3} ms",
        FRAME_W,
        FRAME_H,
        fused_median - single_median
    );
}

/// Body (key lines) of the `[models.<id>]` table of `toml`, without its header.
fn pad_table_body(toml: &str, id: &str) -> String {
    let header = format!("[models.{id}]");
    toml.lines()
        .skip_while(|l| l.trim() != header)
        .skip(1)
        .take_while(|l| !l.trim_start().starts_with('['))
        .collect::<Vec<_>>()
        .join("\n")
}

//! PAD input range matches upstream Silent-Face-Anti-Spoofing (GitHub #212, matrix PVA11).
//!
//! Upstream `src/data_io/functional.py::to_tensor` returns `img.float()` without dividing by
//! 255, so both MiniFASNet checkpoints were trained and are evaluated on raw BGR values in
//! `[0, 255]`. `OrtPadDetector::prepare_input` must produce exactly those values.
//!
//! The real-model test reproduces logits computed with the upstream preprocessing in Python
//! (onnxruntime 1.20.1, the attested `minifasnet_v2_80x80.onnx`, walkthrough 161) on synthetic,
//! non-biometric 80x80 inputs (identity resize, so only the value range and channel order
//! matter). Gating like `pad_real_model_tests`: `SOOS_MODELS_DIR`, default `/var/lib/soos/models`,
//! `SKIPPED` without the model unless `SOOS_REQUIRE_REAL_MODELS=1`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::print_stdout,
    reason = "Contractual test suite uses assertions, unwrap and tensor indexing"
)]

use std::path::{Path, PathBuf};

use soos_inference_ort::pad::OrtPadDetector;
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig, SharedSession};

const PLANE: usize = 80 * 80;
const PAD_MODEL_ID: &str = "minifasnet_v2_pad";
const PAD_MODEL_FILE: &str = "minifasnet_v2_80x80.onnx";
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
/// Same absolute tolerance as the golden logits of `pad_real_model_tests`.
const UPSTREAM_LOGIT_TOLERANCE: f32 = 1e-3;

/// Logits of the upstream preprocessing (`to_tensor` without `/ 255`, BGR, NCHW) on the
/// attested V2 model, computed in Python (walkthrough 161 §10).
const UPSTREAM_GREY_128_LOGITS: [f32; 3] = [-1.892_89, 0.407_29, 1.485_57];
const UPSTREAM_GRADIENT_LOGITS: [f32; 3] = [-4.305_51, 0.249_78, 4.055_55];

#[test]
fn test_pva11_pad_input_keeps_raw_0_255_values_in_bgr_order() {
    let mut rgb = Vec::with_capacity(PLANE * 3);
    for _ in 0..PLANE {
        rgb.extend_from_slice(&[200u8, 100, 50]);
    }
    let tensor = OrtPadDetector::prepare_input(&rgb, 80, 80).expect("prepare_input");
    assert_eq!(
        tensor[0], 50.0,
        "channel 0 is Blue, raw value (not 50 / 255)"
    );
    assert_eq!(tensor[PLANE], 100.0, "channel 1 is Green, raw value");
    assert_eq!(
        tensor[2 * PLANE],
        200.0,
        "channel 2 is Red, raw value (not 0.784)"
    );

    let white = vec![255u8; PLANE * 3];
    let tensor = OrtPadDetector::prepare_input(&white, 80, 80).expect("prepare_input");
    assert!(tensor.iter().all(|&v| v == 255.0), "white maps to 255.0");
}

fn load_real_pad_session(test_name: &str) -> Option<SharedSession> {
    let dir = std::env::var_os("SOOS_MODELS_DIR")
        .map_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR), PathBuf::from);
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
            .expect("installed PAD model matches the committed manifest"),
    )
}

fn raw_logits(session: &SharedSession, rgb: &[u8]) -> Vec<f32> {
    let input = OrtPadDetector::prepare_input(rgb, 80, 80).expect("prepare_input");
    let tensor = ort::value::TensorRef::from_array_view(([1usize, 3, 80, 80], input.as_slice()))
        .expect("tensor");
    let mut guard = session.lock().expect("session mutex");
    let outputs = guard.run(ort::inputs![tensor]).expect("run");
    let (_, value) = outputs.into_iter().next().expect("one output tensor");
    let (_, logits) = value.try_extract_tensor::<f32>().expect("f32 logits");
    logits.to_vec()
}

#[test]
fn test_pva11_real_pad_logits_reproduce_upstream_preprocessing() {
    let Some(session) = load_real_pad_session("test_pva11_real_pad_logits_reproduce_upstream")
    else {
        return;
    };
    let grey = vec![128u8; PLANE * 3];
    let mut gradient = Vec::with_capacity(PLANE * 3);
    for y in 0..80usize {
        for x in 0..80usize {
            gradient.extend_from_slice(&[(x * 3) as u8, (y * 3) as u8, ((x + y) * 3 / 2) as u8]);
        }
    }
    for (name, rgb, upstream) in [
        ("grey_128", grey, UPSTREAM_GREY_128_LOGITS),
        ("gradient_80", gradient, UPSTREAM_GRADIENT_LOGITS),
    ] {
        let logits = raw_logits(&session, &rgb);
        println!("REAL PAD {name}: soos logits {logits:?}, upstream {upstream:?}");
        assert_eq!(logits.len(), 3);
        for (i, (a, u)) in logits.iter().zip(upstream.iter()).enumerate() {
            assert!(
                (a - u).abs() <= UPSTREAM_LOGIT_TOLERANCE,
                "{name}: logit[{i}] {a} differs from upstream {u}"
            );
        }
    }
}

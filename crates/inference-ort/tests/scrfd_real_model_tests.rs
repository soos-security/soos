//! Real-model evidence for the SCRFD detector contract (GitHub #247 VIS-05, #249 VIS-07, #246
//! VIS-04).
//!
//! Gating identical to `embedding_real_model_tests`: models directory `SOOS_MODELS_DIR`, falling
//! back to `/var/lib/soos/models`; an absent model prints `SKIPPED` and returns, unless
//! `SOOS_REQUIRE_REAL_MODELS=1`. Only synthetic, non-biometric frames are used.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::print_stdout,
    reason = "Evidence test suite utilizes direct assertions, unwraps and prints a report"
)]

use std::path::{Path, PathBuf};

use soos_inference_ort::detector::{FaceDetector, OrtScrfdDetector, ScoreActivation};
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig, SharedSession};

const SCRFD_MODEL_ID: &str = "scrfd_500m_kps";
const SCRFD_MODEL_FILE: &str = "scrfd_500m_kps.onnx";
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";

fn models_dir() -> PathBuf {
    std::env::var_os("SOOS_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR))
}

fn repo_manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml")
}

fn load_real_scrfd_session(test_name: &str) -> Option<SharedSession> {
    let dir = models_dir();
    if !dir.join(SCRFD_MODEL_FILE).is_file() {
        assert!(
            !std::env::var("SOOS_REQUIRE_REAL_MODELS").is_ok_and(|v| v == "1"),
            "SOOS_REQUIRE_REAL_MODELS=1 but {SCRFD_MODEL_FILE} is missing from {}",
            dir.display()
        );
        println!(
            "SKIPPED {test_name}: {SCRFD_MODEL_FILE} not found in {}",
            dir.display()
        );
        return None;
    }
    let mut registry = ModelRegistry::new(RegistryConfig::with_manifest(dir, repo_manifest_path()))
        .expect("committed manifest must parse");
    registry
        .verify_integrity()
        .expect("installed models must verify");
    Some(
        registry
            .get_or_load_session(SCRFD_MODEL_ID)
            .expect("installed SCRFD model must load from its verified bytes"),
    )
}

/// Synthetic, non-biometric 640x480 RGB gradient.
fn gradient(width: usize, height: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            buf.push((x % 256) as u8);
            buf.push((y % 256) as u8);
            buf.push(((x + y) % 256) as u8);
        }
    }
    buf
}

/// The attested SCRFD session exposes the 9 stride outputs with the expected shape patterns,
/// so the startup validation in `OrtScrfdDetector::new` accepts it.
#[test]
fn test_real_scrfd_session_passes_startup_shape_validation() {
    let Some(session) =
        load_real_scrfd_session("test_real_scrfd_session_passes_startup_shape_validation")
    else {
        return;
    };
    let dims: Vec<Vec<i64>> = {
        let guard = session.lock().expect("session mutex");
        guard
            .outputs()
            .iter()
            .map(|o| o.dtype().tensor_shape().expect("tensor").to_vec())
            .collect()
    };
    println!("scrfd output dims: {dims:?}");
    OrtScrfdDetector::validate_output_dims(&dims).expect("attested SCRFD dims must validate");
    OrtScrfdDetector::new(session, 0.5, 0.45).expect("attested SCRFD must construct");
}

/// The attested SCRFD graph emits probabilities: the production `Probability` activation
/// decodes a synthetic frame without any out-of-range score (fail-closed path not taken).
#[test]
fn test_real_scrfd_scores_are_probabilities() {
    let Some(session) = load_real_scrfd_session("test_real_scrfd_scores_are_probabilities") else {
        return;
    };
    let detector = OrtScrfdDetector::new(session, 0.5, 0.45).expect("construct");
    assert_eq!(detector.score_activation, ScoreActivation::Probability);
    for (w, h) in [(640usize, 480usize), (320, 240)] {
        let frame = gradient(w, h);
        let detections = detector
            .detect(&frame, w as u32, h as u32)
            .expect("attested SCRFD scores must all lie in [0, 1]");
        for det in &detections {
            assert!((0.0..=1.0).contains(&det.score));
        }
    }
}

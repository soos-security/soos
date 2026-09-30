//! Real-model evidence for the SCRFD detector hot path (GitHub #252, review finding VIS-10).
//!
//! Exercises the production `OrtScrfdDetector` on the attested `scrfd_500m_kps` model with its
//! reusable letterbox scratch tensor, borrowed output decoding and in-place output wiping, and
//! prints a p50/p95 latency report under the default intra-op thread count.
//!
//! Gating (CI stays green on runners without models), identical to
//! `embedding_real_model_tests`: models directory `SOOS_MODELS_DIR`, falling back to
//! `/var/lib/soos/models`; an absent model prints `SKIPPED` and returns, unless
//! `SOOS_REQUIRE_REAL_MODELS=1`. Only synthetic, non-biometric frames are used. The latency
//! numbers are reported, not asserted (shared CI hosts make wall-clock bounds flaky).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::print_stdout,
    reason = "Evidence test suite utilizes direct assertions, unwraps and prints a measurement report"
)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use soos_inference_ort::detector::{FaceDetector, OrtScrfdDetector};
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig};

const DETECTOR_MODEL_ID: &str = "scrfd_500m_kps";
const DETECTOR_MODEL_FILE: &str = "scrfd_500m_kps.onnx";
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
const LATENCY_ITERATIONS: usize = 20;
const LATENCY_WARMUP: usize = 3;

fn models_dir() -> PathBuf {
    std::env::var_os("SOOS_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR))
}

fn repo_manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml")
}

fn load_detector(test_name: &str) -> Option<OrtScrfdDetector> {
    let dir = models_dir();
    if !dir.join(DETECTOR_MODEL_FILE).is_file() {
        assert!(
            !std::env::var("SOOS_REQUIRE_REAL_MODELS").is_ok_and(|v| v == "1"),
            "SOOS_REQUIRE_REAL_MODELS=1 but {DETECTOR_MODEL_FILE} is missing from {}",
            dir.display()
        );
        println!(
            "SKIPPED {test_name}: {DETECTOR_MODEL_FILE} not found in {} (set SOOS_MODELS_DIR)",
            dir.display()
        );
        return None;
    }
    let mut registry = ModelRegistry::new(RegistryConfig::with_manifest(dir, repo_manifest_path()))
        .expect("committed models/manifest.toml must parse");
    let session = registry
        .get_or_load_session(DETECTOR_MODEL_ID)
        .expect("installed SCRFD model must match the committed manifest");
    Some(OrtScrfdDetector::new(session, 0.5, 0.45).expect("SCRFD detector"))
}

fn synthetic_frame(width: u32, height: u32, seed: u32) -> Vec<u8> {
    (0..(width * height * 3))
        .map(|i| ((i.wrapping_mul(2_654_435_761).wrapping_add(seed)) >> 24) as u8)
        .collect()
}

fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    let last = sorted_ms.len() - 1;
    let rank = (0..=last)
        .find(|&i| i as f64 >= last as f64 * p - 0.5)
        .unwrap_or(last);
    sorted_ms[rank]
}

#[test]
fn test_real_scrfd_scratch_reuse_is_deterministic_across_frames() {
    let Some(detector) = load_detector("test_real_scrfd_scratch_reuse_is_deterministic") else {
        return;
    };
    let frame_a = synthetic_frame(640, 480, 1);
    let frame_b = synthetic_frame(320, 240, 7);

    let first = detector.detect(&frame_a, 640, 480).expect("detect A");
    let other = detector.detect(&frame_b, 320, 240).expect("detect B");
    let again = detector.detect(&frame_a, 640, 480).expect("detect A again");
    assert_eq!(
        first, again,
        "reusing the scratch tensor across frames must not change detections"
    );
    for det in first.iter().chain(other.iter()) {
        assert!(det.score.is_finite());
        if let Some(lm) = det.landmarks {
            assert!(lm
                .as_array()
                .iter()
                .all(|p| p.x.is_finite() && p.y.is_finite()));
        }
    }
}

#[test]
fn test_real_scrfd_detect_latency_report() {
    let Some(detector) = load_detector("test_real_scrfd_detect_latency_report") else {
        return;
    };
    let frame = synthetic_frame(640, 480, 3);
    for _ in 0..LATENCY_WARMUP {
        detector.detect(&frame, 640, 480).expect("warm-up detect");
    }
    let mut samples = Vec::with_capacity(LATENCY_ITERATIONS);
    for _ in 0..LATENCY_ITERATIONS {
        let start = Instant::now();
        detector.detect(&frame, 640, 480).expect("timed detect");
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "REAL SCRFD latency (640x480 -> 640x640, intra_threads={}): p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        soos_inference_ort::registry::default_intra_threads(),
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        samples[samples.len() - 1]
    );
}

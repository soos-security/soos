//! Static contract for the attested inference path (GitHub #246 VIS-04, #247 VIS-05,
//! #249 VIS-07).
//!
//! - The registry builds ORT sessions from the verified in-memory bytes, never by re-opening
//!   the model path (`commit_from_file` is banned from `crates/inference-ort/src`).
//! - The SCRFD detector has no per-element sigmoid fallback: the score activation is an
//!   explicit `ScoreActivation`, and the production `detect` path uses the fail-closed decoder.
//! - The legacy UltraFace inference path, the unused `ndarray` dependency and the legacy
//!   4-model download URLs are gone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Every `.rs` file under `crates/inference-ort/src` must build sessions from memory.
#[test]
fn test_inference_ort_never_loads_a_session_by_path() {
    let dir = workspace_root().join("crates/inference-ort/src");
    for entry in fs::read_dir(&dir).expect("read inference-ort/src") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).expect("read source");
            assert!(
                !text.contains("commit_from_file"),
                "{} re-opens a model by path after hashing (use commit_from_memory on the \
                 verified bytes)",
                path.display()
            );
        }
    }
    let registry = read("crates/inference-ort/src/registry.rs");
    assert!(registry.contains("commit_from_memory(&bytes)"));
    assert!(registry.contains("read_verified_model"));
}

/// The SCRFD decoder has no per-element activation heuristic, and `detect` fails closed.
#[test]
fn test_scrfd_detect_uses_explicit_fail_closed_activation() {
    let detector = read("crates/inference-ort/src/detector.rs");
    assert!(
        !detector.contains("if (0.0..=1.0).contains(&raw_score)"),
        "per-element sigmoid fallback must not come back (VIS-05)"
    );
    let detect_impl = detector
        .split("impl FaceDetector for OrtScrfdDetector")
        .nth(1)
        .expect("OrtScrfdDetector implements FaceDetector");
    assert!(
        detect_impl.contains("Self::decode_stride_checked(")
            && detect_impl.contains("self.score_activation"),
        "OrtScrfdDetector::detect must decode with its configured activation, fail closed"
    );
    assert!(
        !detect_impl.contains("Self::decode_stride("),
        "the heuristic decode_stride entry point must not be used by detect"
    );
}

/// Legacy 4-model artefacts are gone from the production crate, its manifest and the
/// download script.
#[test]
fn test_legacy_four_model_pipeline_artefacts_are_removed() {
    let detector = read("crates/inference-ort/src/detector.rs");
    assert!(
        !detector.contains("impl FaceDetector for OrtFaceDetector"),
        "the UltraFace inference path must not be a FaceDetector any more"
    );
    assert!(!detector.contains("generate_priors"));

    let cargo = read("crates/inference-ort/Cargo.toml");
    assert!(
        !cargo.contains("ndarray"),
        "soos-inference-ort must not depend on the unused ndarray crate"
    );

    let script = read("scripts/download_models.sh");
    for legacy in [
        "ultraface_slim_320",
        "landmark_5point",
        "mobilefacenet_arcface",
        "minifasnet_pad)",
        "version-slim-320.onnx",
    ] {
        assert!(
            !script.contains(legacy),
            "scripts/download_models.sh still resolves legacy model '{legacy}'"
        );
    }
}

/// VTS8 (GitHub #249, user-approved test removal 2026-09-30): the legacy UltraFace detector
/// input helper and the legacy landmark detector trait and mock are gone for good.
#[test]
fn test_vts8_legacy_detector_types_removed() {
    for (file, needle) in [
        (
            "crates/inference-ort/src/detector.rs",
            "pub struct OrtFaceDetector",
        ),
        (
            "crates/inference-ort/src/landmarks.rs",
            "pub trait LandmarkDetector",
        ),
        (
            "crates/inference-ort/src/mock.rs",
            "pub struct MockLandmarkDetector",
        ),
    ] {
        assert!(
            !read(file).contains(needle),
            "{file} must not reintroduce `{needle}` (removed with the 3-model pipeline, VTS8)"
        );
    }
    let lib = read("crates/inference-ort/src/lib.rs");
    for name in [
        "OrtFaceDetector",
        "LandmarkDetector",
        "MockLandmarkDetector",
    ] {
        assert!(
            !lib.split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|t| t == name),
            "crates/inference-ort/src/lib.rs must not re-export `{name}`"
        );
    }
}

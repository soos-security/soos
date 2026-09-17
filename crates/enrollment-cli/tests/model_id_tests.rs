#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_enrollment_cli::{
    resolve_camera_device, MODEL_ID_EMBEDDING, MODEL_ID_FACE_DETECTOR, MODEL_ID_LANDMARKS,
    MODEL_ID_PAD, REQUIRED_MODEL_IDS,
};
use soos_inference_ort::ModelManifest;
use std::path::PathBuf;

#[test]
fn test_enrollment_cli_model_ids_match_manifest() {
    // 1. Assert exact required model IDs per Sub-issue #19.1
    assert_eq!(MODEL_ID_FACE_DETECTOR, "ultraface_slim_320");
    assert_eq!(MODEL_ID_LANDMARKS, "landmark_5point");
    assert_eq!(MODEL_ID_PAD, "minifasnet_pad");
    assert_eq!(MODEL_ID_EMBEDDING, "mobilefacenet_arcface");

    assert_eq!(REQUIRED_MODEL_IDS.len(), 4);
    assert_eq!(REQUIRED_MODEL_IDS[0], "ultraface_slim_320");
    assert_eq!(REQUIRED_MODEL_IDS[1], "landmark_5point");
    assert_eq!(REQUIRED_MODEL_IDS[2], "minifasnet_pad");
    assert_eq!(REQUIRED_MODEL_IDS[3], "mobilefacenet_arcface");

    // 2. Assert against official models/manifest.toml
    let manifest_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    let manifest =
        ModelManifest::from_file(&manifest_path).expect("Failed to load models/manifest.toml");

    for model_id in &REQUIRED_MODEL_IDS {
        let model = manifest.get_model(model_id).unwrap_or_else(|| {
            panic!("Model '{model_id}' attested by enrollment-cli must be in manifest.toml")
        });
        assert_eq!(model.id, *model_id);
        assert!(!model.filename.is_empty(), "Model filename cannot be empty");
        assert_eq!(
            model.sha256.len(),
            64,
            "SHA-256 hash must be 64 hex characters"
        );
    }
}

#[test]
fn test_camera_device_path_uses_stable_by_id() {
    // Assert Criterion C4 per Sub-issue #19.3
    let default_path = resolve_camera_device(None);
    assert!(
        default_path
            .to_string_lossy()
            .starts_with("/dev/v4l/by-id/"),
        "Default camera device path must start with /dev/v4l/by-id/, got: {}",
        default_path.display()
    );

    // Assert explicit user override is respected
    let custom_path = PathBuf::from("/dev/video7");
    let resolved = resolve_camera_device(Some(custom_path.clone()));
    assert_eq!(resolved, custom_path);
}

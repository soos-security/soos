//! Contractual test suite for manifest parsing and SHA-256 verification.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use soos_inference_ort::error::InferenceError;
use soos_inference_ort::manifest::ModelManifest;
use tempfile::tempdir;

fn workspace_models_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("Failed to locate workspace root")
        .join("models")
}

#[test]
fn test_parse_workspace_manifest_file() {
    let manifest_path = workspace_models_dir().join("manifest.toml");
    assert!(
        manifest_path.exists(),
        "models/manifest.toml must exist in repository root: {:?}",
        manifest_path
    );

    let manifest = ModelManifest::from_file(&manifest_path).expect("Failed to parse manifest.toml");
    assert_eq!(manifest.manifest.version, "1.0.0");

    let ultraface = manifest
        .get_model("ultraface_slim_320")
        .expect("ultraface_slim_320 missing");
    assert_eq!(ultraface.filename, "version-slim-320.onnx");
    assert_eq!(ultraface.license, "MIT");
    assert_eq!(ultraface.input_shape, vec![1, 3, 240, 320]);
    assert_eq!(
        ultraface.output_shapes,
        vec![vec![1, 4420, 2], vec![1, 4420, 4]]
    );

    let landmark = manifest
        .get_model("landmark_5point")
        .expect("landmark_5point missing");
    assert_eq!(landmark.filename, "landmark_5point.onnx");
    assert_eq!(landmark.input_shape, vec![1, 3, 112, 112]);

    let mobilefacenet = manifest
        .get_model("mobilefacenet_arcface")
        .expect("mobilefacenet_arcface missing");
    assert_eq!(mobilefacenet.filename, "mobilefacenet_arcface.onnx");
    assert_eq!(mobilefacenet.input_shape, vec![1, 3, 112, 112]);
    assert_eq!(mobilefacenet.output_shapes, vec![vec![1, 128]]);
}

#[test]
fn test_compute_sha256_known_string() {
    let dir = tempdir().expect("tempdir creation failed");
    let file_path = dir.path().join("test.bin");

    // SHA-256 of "hello world\n" is d7a8fbb307d7809469ca933b02d8cf82a2a3b9c45fbb7563d088910be45ed4f2
    // SHA-256 of "soos biometric pam"
    let content = b"soos biometric pam";
    {
        let mut file = File::create(&file_path).expect("failed to create file");
        file.write_all(content).expect("failed to write content");
    }

    let hash = ModelManifest::compute_sha256(&file_path).expect("failed to compute hash");
    // Verify hash matches independent sha2 computation
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content);
    let expected = format!("{:x}", hasher.finalize());

    assert_eq!(hash, expected);
}

#[test]
fn test_verify_model_checksum_success_and_tamper_detection() {
    let dir = tempdir().expect("tempdir creation failed");
    let model_file = dir.path().join("model.onnx");

    let valid_bytes = b"authentic ONNX model weights content";
    {
        let mut f = File::create(&model_file).expect("create file");
        f.write_all(valid_bytes).expect("write bytes");
    }

    let valid_hash = ModelManifest::compute_sha256(&model_file).expect("compute hash");

    let toml_content = format!(
        r#"
[manifest]
version = "1.0.0"

[models.test_model]
id = "test_model"
filename = "model.onnx"
sha256 = "{}"
license = "MIT"
source_url = "https://example.org/model"
description = "Test Model"
input_shape = [1, 3, 112, 112]
"#,
        valid_hash
    );

    let manifest = ModelManifest::from_toml_str(&toml_content).expect("parse manifest");

    // 1. Nominal verification
    assert!(manifest
        .verify_model_checksum("test_model", &model_file)
        .is_ok());

    // 2. Tampering detection
    {
        let mut f = File::create(&model_file).expect("reopen file");
        f.write_all(b"corrupted or tampered weights")
            .expect("write bytes");
    }

    let err = manifest
        .verify_model_checksum("test_model", &model_file)
        .expect_err("tampered file must fail verification");

    match err {
        InferenceError::ChecksumMismatch {
            id,
            expected,
            actual,
        } => {
            assert_eq!(id, "test_model");
            assert_eq!(expected, valid_hash);
            assert_ne!(actual, valid_hash);
        }
        other => panic!("Expected ChecksumMismatch, got: {:?}", other),
    }
}

#[test]
fn test_verify_model_checksum_missing_file() {
    let toml_content = r#"
[manifest]
version = "1.0.0"

[models.missing_model]
id = "missing_model"
filename = "non_existent.onnx"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
license = "MIT"
source_url = "https://example.org/missing"
description = "Missing Model"
input_shape = [1, 3, 112, 112]
"#;
    let manifest = ModelManifest::from_toml_str(toml_content).expect("parse manifest");
    let missing_path = PathBuf::from("/path/to/definitely/non/existent/model.onnx");

    let err = manifest
        .verify_model_checksum("missing_model", &missing_path)
        .expect_err("missing file must return error");

    match err {
        InferenceError::ModelNotFound { id, path } => {
            assert_eq!(id, "missing_model");
            assert_eq!(path, missing_path);
        }
        other => panic!("Expected ModelNotFound, got: {:?}", other),
    }
}

#[test]
fn test_invalid_toml_fails_closed() {
    let invalid_toml = "this is not valid toml = [[[";
    let err = ModelManifest::from_toml_str(invalid_toml).expect_err("invalid toml must fail");
    match err {
        InferenceError::ManifestParse(_) => {}
        other => panic!("Expected ManifestParse error, got: {:?}", other),
    }
}

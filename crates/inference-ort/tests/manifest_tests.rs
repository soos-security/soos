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
    assert_eq!(manifest.manifest.version, "2.0.0");
    assert_eq!(
        manifest.models.len(),
        3,
        "Manifest v2.0.0 must contain exactly 3 models"
    );

    // 1. SCRFD 500M KPS
    let scrfd = manifest
        .get_model("scrfd_500m_kps")
        .expect("scrfd_500m_kps missing from manifest");
    assert_eq!(scrfd.filename, "scrfd_500m_kps.onnx");
    assert_eq!(scrfd.license, "MIT");
    assert_eq!(scrfd.input_shape, vec![1, 3, 640, 640]);
    assert_eq!(
        scrfd.output_shapes,
        vec![
            vec![1, 12800, 1],
            vec![1, 3200, 1],
            vec![1, 800, 1],
            vec![1, 12800, 4],
            vec![1, 3200, 4],
            vec![1, 800, 4],
            vec![1, 12800, 10],
            vec![1, 3200, 10],
            vec![1, 800, 10],
        ]
    );
    assert_eq!(
        scrfd.sha256,
        "a3562ef62592bf387f6ef19151282ac127518e51c77696e62e0661bee95ba1ad"
    );

    // 2. SFace 2021dec embedding (replaces the retired ArcFace ResNet34, GitHub #278,
    //    owner decision 2026-10-01)
    let sface = manifest
        .get_model("sface_2021dec")
        .expect("sface_2021dec missing from manifest");
    assert_eq!(sface.filename, "sface_2021dec.onnx");
    assert_eq!(sface.license, "Apache-2.0");
    assert_eq!(sface.input_shape, vec![1, 3, 112, 112]);
    assert_eq!(sface.output_shapes, vec![vec![1, 128]]);
    assert_eq!(
        sface.sha256,
        "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79"
    );
    assert!(
        manifest.get_model("arcface_w600k_mbf").is_none(),
        "the retired ArcFace entry must not be attested by the shipped manifest"
    );

    // 3. MiniFASNetV2 PAD
    let pad = manifest
        .get_model("minifasnet_v2_pad")
        .expect("minifasnet_v2_pad missing from manifest");
    assert_eq!(pad.filename, "minifasnet_v2_80x80.onnx");
    assert_eq!(pad.license, "Apache-2.0");
    assert_eq!(pad.input_shape, vec![1, 3, 80, 80]);
    assert_eq!(pad.output_shapes, vec![vec![1, 3]]);
    assert_eq!(
        pad.sha256,
        "0cbe5caec95c31de9d2ef845cb85407d76aecd1b6a2c0e343f7d35306bfbccb8"
    );

    // Ensure legacy models are strictly removed
    assert!(
        manifest.get_model("ultraface_slim_320").is_none(),
        "ultraface_slim_320 must be removed in manifest v2.0.0"
    );
    assert!(
        manifest.get_model("landmark_5point").is_none(),
        "landmark_5point must be removed in manifest v2.0.0"
    );
    assert!(
        manifest.get_model("mobilefacenet_arcface").is_none(),
        "mobilefacenet_arcface must be removed in manifest v2.0.0"
    );
    assert!(
        manifest.get_model("minifasnet_pad").is_none(),
        "minifasnet_pad must be removed in manifest v2.0.0"
    );
}

#[test]
fn test_manifest_v2_model_count_and_checksum_attestation() {
    let manifest_path = workspace_models_dir().join("manifest.toml");
    let manifest = ModelManifest::from_file(&manifest_path).expect("Failed to parse manifest.toml");

    assert_eq!(manifest.manifest.version, "2.0.0");
    assert_eq!(manifest.models.len(), 3);

    for (id, meta) in &manifest.models {
        assert_eq!(
            &meta.id, id,
            "Model table key must match internal id for {id}"
        );
        assert!(
            !meta.filename.is_empty(),
            "Filename must not be empty for {id}"
        );
        assert_eq!(
            meta.sha256.len(),
            64,
            "SHA-256 digest must be 64 characters for {id}"
        );
        assert!(
            meta.sha256.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA-256 digest must be hex characters for {id}"
        );
        assert!(
            !meta.source_url.is_empty(),
            "Source URL must not be empty for {id}"
        );
        assert!(
            !meta.description.is_empty(),
            "Description must not be empty for {id}"
        );
    }
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

/// SFC1 (GitHub #278): the shipped manifest attests the SFace file from a pinned Hugging Face
/// revision (never a branch), with its exact size, NCHW layout and Apache-2.0 licence, and the
/// retired ArcFace attestation lives only in `models/retired_models.toml`.
#[test]
fn test_workspace_manifest_attests_sface_from_pinned_revision() {
    let text = std::fs::read_to_string(workspace_models_dir().join("manifest.toml"))
        .expect("read manifest.toml");
    let raw: toml::Value = toml::from_str(&text).expect("manifest is TOML");
    let entry = &raw["models"]["sface_2021dec"];
    assert_eq!(entry["size_bytes"].as_integer(), Some(38_696_353));
    assert_eq!(entry["input_layout"].as_str(), Some("NCHW"));
    assert_eq!(entry["license"].as_str(), Some("Apache-2.0"));
    assert_eq!(
        entry["source_url"].as_str(),
        Some(
            "https://huggingface.co/opencv/face_recognition_sface/resolve/\
             3d7082438a6e4551e840c9b2bb60b71e8da4b524/face_recognition_sface_2021dec.onnx"
        )
    );
    assert!(raw["models"].get("arcface_w600k_mbf").is_none());

    let retired = ModelManifest::from_file(workspace_models_dir().join("retired_models.toml"))
        .expect("models/retired_models.toml parses with the manifest schema");
    let arcface = retired
        .get_model("arcface_w600k_mbf")
        .expect("retired ArcFace attestation");
    assert_eq!(
        arcface.sha256,
        "ffe014a45c9488506719d37fd578ece6661bb385535b36e8039975fa5d4683db"
    );
    assert_eq!(arcface.license, "NOASSERTION");
    assert!(retired.get_model("sface_2021dec").is_none());
}

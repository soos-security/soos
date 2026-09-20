//! Contractual test suite for ModelRegistry attestation and session configuration.

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
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig};
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
fn test_registry_initialization_with_workspace_manifest() {
    let models_dir = workspace_models_dir();
    let config = RegistryConfig::new(&models_dir);
    let registry = ModelRegistry::new(config).expect("failed to load registry");

    assert_eq!(registry.manifest().manifest.version, "2.0.0");
    assert!(registry.manifest().get_model("scrfd_500m_kps").is_some());
    assert!(registry.manifest().get_model("arcface_w600k_mbf").is_some());
    assert!(registry.manifest().get_model("minifasnet_v2_pad").is_some());
}

#[test]
fn test_registry_verify_integrity_missing_files_fails_closed() {
    let dir = tempdir().expect("tempdir");
    let manifest_path = dir.path().join("manifest.toml");

    let toml_content = r#"
[manifest]
version = "1.0.0"

[models.dummy]
id = "dummy"
filename = "dummy.onnx"
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
license = "MIT"
source_url = "https://example.org"
description = "Dummy"
input_shape = [1, 3, 112, 112]
"#;
    {
        let mut f = File::create(&manifest_path).expect("create manifest");
        f.write_all(toml_content.as_bytes())
            .expect("write manifest");
    }

    let config = RegistryConfig::with_manifest(dir.path(), &manifest_path);
    let registry = ModelRegistry::new(config).expect("registry init");

    // dummy.onnx does not exist in dir.path()
    let err = registry
        .verify_integrity()
        .expect_err("missing model file must fail verification");

    match err {
        InferenceError::ModelNotFound { id, .. } => {
            assert_eq!(id, "dummy");
        }
        other => panic!("Expected ModelNotFound, got {:?}", other),
    }
}

#[test]
fn test_registry_resolve_model_path() {
    let dir = tempdir().expect("tempdir");
    let manifest_path = dir.path().join("manifest.toml");
    let model_file = dir.path().join("test.onnx");

    {
        let mut f = File::create(&model_file).expect("create model");
        f.write_all(b"model data").expect("write model data");
    }

    let hash = ModelManifest::compute_sha256(&model_file).expect("hash");

    let toml_content = format!(
        r#"
[manifest]
version = "1.0.0"

[models.test]
id = "test"
filename = "test.onnx"
sha256 = "{}"
license = "MIT"
source_url = "https://example.org"
description = "Test"
input_shape = [1, 3, 112, 112]
"#,
        hash
    );

    {
        let mut f = File::create(&manifest_path).expect("create manifest");
        f.write_all(toml_content.as_bytes())
            .expect("write manifest");
    }

    let config = RegistryConfig::with_manifest(dir.path(), &manifest_path);
    let registry = ModelRegistry::new(config).expect("registry init");

    let resolved = registry
        .resolve_model_path("test")
        .expect("resolve model path failed");
    assert_eq!(resolved, model_file);

    // Unknown model
    let err = registry
        .resolve_model_path("unknown")
        .expect_err("unknown model must fail");
    match err {
        InferenceError::ModelNotFound { id, .. } => {
            assert_eq!(id, "unknown");
        }
        other => panic!("Expected ModelNotFound, got {:?}", other),
    }
}

//! Contractual test suite for Issue #18: ONNX model download, verification, and deployment.
//!
//! Sub-issues:
//! - #18.1: test_download_script_verifies_checksums
//! - #18.2: test_models_readme_complete_and_accurate
//! - #18.3: test_daemon_refuses_start_with_missing_models & test_daemon_refuses_start_with_tampered_models

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

use soos_daemon::config::PipelineConfig;
use soos_daemon::error::DaemonError;
use soos_daemon::pipeline::initialize_pipeline;
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::manifest::ModelManifest;

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("Failed to locate workspace root")
        .to_path_buf()
}

#[test]
fn test_download_script_verifies_checksums() {
    let root = workspace_root();
    let script_path = root.join("scripts").join("download_models.sh");

    assert!(
        script_path.exists(),
        "scripts/download_models.sh must exist at: {:?}",
        script_path
    );

    let test_dir = tempdir().expect("Failed to create tempdir");
    let target_models_dir = test_dir.path().join("soos_models");
    let manifest_path = test_dir.path().join("test_manifest.toml");

    // Create 2 test model files with known content
    let model1_content = b"fake-onnx-model-ultraface-content";
    let model1_src = test_dir.path().join("model1_src.onnx");
    fs::write(&model1_src, model1_content).expect("write model1");
    let model1_hash = ModelManifest::compute_sha256(&model1_src).expect("hash model1");

    let model2_content = b"fake-onnx-model-landmarks-content";
    let model2_src = test_dir.path().join("model2_src.onnx");
    fs::write(&model2_src, model2_content).expect("write model2");
    let model2_hash = ModelManifest::compute_sha256(&model2_src).expect("hash model2");

    // Write a test manifest pointing to local file paths
    let manifest_content = format!(
        r#"[manifest]
version = "1.0.0"

[models.test_m1]
id = "test_m1"
filename = "model1.onnx"
sha256 = "{m1_hash}"
license = "MIT"
source_url = "file://{m1_path}"
description = "Test Model 1"
input_shape = [1, 3, 240, 320]
output_shapes = [[1, 10]]

[models.test_m2]
id = "test_m2"
filename = "model2.onnx"
sha256 = "{m2_hash}"
license = "Apache-2.0"
source_url = "file://{m2_path}"
description = "Test Model 2"
input_shape = [1, 3, 112, 112]
output_shapes = [[1, 3]]
"#,
        m1_hash = model1_hash,
        m1_path = model1_src.display(),
        m2_hash = model2_hash,
        m2_path = model2_src.display()
    );
    fs::write(&manifest_path, manifest_content).expect("write manifest");

    // Nominal Case: run download_models.sh with --manifest and --target-dir
    let status = Command::new("bash")
        .arg(&script_path)
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--target-dir")
        .arg(&target_models_dir)
        .status()
        .expect("Failed to execute download_models.sh");

    assert!(
        status.success(),
        "scripts/download_models.sh should exit with 0 on valid checksums"
    );

    // Verify models and manifest are deployed in target directory
    let deployed_m1 = target_models_dir.join("model1.onnx");
    let deployed_m2 = target_models_dir.join("model2.onnx");
    let deployed_manifest = target_models_dir.join("manifest.toml");

    assert!(deployed_m1.is_file(), "model1.onnx must be deployed");
    assert!(deployed_m2.is_file(), "model2.onnx must be deployed");
    assert!(
        deployed_manifest.is_file(),
        "manifest.toml must be copied to target directory"
    );

    // Tampered / Checksum Mismatch Case:
    // Modify model1_src to corrupt its hash
    fs::write(&model1_src, b"corrupted-tampered-onnx-bytes").expect("corrupt model1");

    let tampered_target_dir = test_dir.path().join("soos_models_tampered");
    let fail_status = Command::new("bash")
        .arg(&script_path)
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--target-dir")
        .arg(&tampered_target_dir)
        .status()
        .expect("Failed to execute download_models.sh");

    assert!(
        !fail_status.success(),
        "scripts/download_models.sh MUST fail loudly with non-zero exit code on checksum mismatch"
    );
}

#[test]
fn test_models_readme_complete_and_accurate() {
    let root = workspace_root();
    let readme_path = root.join("models").join("README.md");

    assert!(
        readme_path.exists(),
        "models/README.md must exist at: {:?}",
        readme_path
    );

    let content = fs::read_to_string(&readme_path).expect("Read models/README.md");

    // Must document all 3 next-gen models
    assert!(content.contains("scrfd_500m_kps"));
    assert!(content.contains("arcface_w600k_mbf"));
    assert!(content.contains("minifasnet_v2_pad"));

    // Must document licenses
    assert!(content.contains("MIT"));
    assert!(content.contains("Apache-2.0"));

    // Must document acquisition and verification
    assert!(content.contains("download_models.sh") || content.contains("Acquisition"));
    assert!(content.contains("SHA-256") || content.contains("sha256"));

    // Must include legal notice for redistribution restrictions
    assert!(
        content.contains("Legal Notice")
            || content.contains("Redistribution")
            || content.contains("License"),
        "models/README.md must contain legal and redistribution notices"
    );
}

#[test]
fn test_daemon_refuses_start_with_missing_models() {
    let dir = tempdir().expect("tempdir");
    let models_dir = dir.path().join("models");
    fs::create_dir_all(&models_dir).expect("create models dir");

    // Put an attested manifest requiring scrfd_500m_kps, but don't place the model file
    let manifest_path = models_dir.join("manifest.toml");
    let manifest_content = r#"[manifest]
version = "2.0.0"

[models.scrfd_500m_kps]
id = "scrfd_500m_kps"
filename = "scrfd_500m_kps.onnx"
sha256 = "a3562ef62592bf387f6ef19151282ac127518e51c77696e62e0661bee95ba1ad"
license = "MIT"
source_url = "https://huggingface.co/ykk648/face_lib/resolve/main/face_detect/scrfd_onnx/scrfd_500m_bnkps.onnx"
description = "SCRFD 500M KPS unified face detection and 5-point facial landmarks"
input_shape = [1, 3, 640, 640]
output_shapes = [[1, 12800, 1], [1, 3200, 1], [1, 800, 1], [1, 12800, 4], [1, 3200, 4], [1, 800, 4], [1, 12800, 10], [1, 3200, 10], [1, 800, 10]]
"#;
    fs::write(&manifest_path, manifest_content).expect("write manifest");

    let mut config = PipelineConfig {
        use_mock_camera: true,
        ..Default::default()
    };
    config.models_dir = models_dir;
    config.biometrics_dir = dir.path().join("biometrics");
    config.master_key_path = dir.path().join("master.key");
    config.evidence.base_dir = dir.path().join("evidence");
    config.evidence.key_path = dir.path().join("evidence.key");

    let res = initialize_pipeline(&config);
    match res {
        Err(DaemonError::Inference(InferenceError::ModelNotFound { id, .. })) => {
            assert_eq!(id, "scrfd_500m_kps");
        }
        other => panic!(
            "Expected DaemonError::Inference(ModelNotFound), got: {:?}",
            other
        ),
    }
}

#[test]
fn test_daemon_refuses_start_with_tampered_models() {
    let dir = tempdir().expect("tempdir");
    let models_dir = dir.path().join("models");
    fs::create_dir_all(&models_dir).expect("create models dir");

    // Put manifest with expected sha256
    let manifest_path = models_dir.join("manifest.toml");
    let manifest_content = r#"[manifest]
version = "2.0.0"

[models.scrfd_500m_kps]
id = "scrfd_500m_kps"
filename = "scrfd_500m_kps.onnx"
sha256 = "a3562ef62592bf387f6ef19151282ac127518e51c77696e62e0661bee95ba1ad"
license = "MIT"
source_url = "https://huggingface.co/ykk648/face_lib/resolve/main/face_detect/scrfd_onnx/scrfd_500m_bnkps.onnx"
description = "SCRFD 500M KPS unified face detection and 5-point facial landmarks"
input_shape = [1, 3, 640, 640]
output_shapes = [[1, 12800, 1], [1, 3200, 1], [1, 800, 1], [1, 12800, 4], [1, 3200, 4], [1, 800, 4], [1, 12800, 10], [1, 3200, 10], [1, 800, 10]]
"#;
    fs::write(&manifest_path, manifest_content).expect("write manifest");

    // Create the file but with tampered/wrong content
    let model_path = models_dir.join("scrfd_500m_kps.onnx");
    fs::write(&model_path, b"corrupted-tampered-weights-data").expect("write tampered model");

    let mut config = PipelineConfig {
        use_mock_camera: true,
        ..Default::default()
    };
    config.models_dir = models_dir;
    config.biometrics_dir = dir.path().join("biometrics");
    config.master_key_path = dir.path().join("master.key");
    config.evidence.base_dir = dir.path().join("evidence");
    config.evidence.key_path = dir.path().join("evidence.key");

    let res = initialize_pipeline(&config);
    match res {
        Err(DaemonError::Inference(InferenceError::ChecksumMismatch { id, expected, .. })) => {
            assert_eq!(id, "scrfd_500m_kps");
            assert_eq!(
                expected,
                "a3562ef62592bf387f6ef19151282ac127518e51c77696e62e0661bee95ba1ad"
            );
        }
        other => panic!(
            "Expected DaemonError::Inference(ChecksumMismatch), got: {:?}",
            other
        ),
    }
}

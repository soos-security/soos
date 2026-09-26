//! Contractual tests for template import and camera device config resolution.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    clippy::unreachable,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

use soos_biometric_store::{BiometricStore, MasterKey};
use soos_enrollment_cli::args::ImportArgs;
use soos_enrollment_cli::service::{resolve_camera_device_from_config, EnrollmentService};

#[test]
fn test_resolve_camera_device_from_daemon_config() {
    let tmp = tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("daemon.toml");
    let mut file = File::create(&cfg_path).expect("create config");
    writeln!(
        file,
        "[pipeline]\ncamera_device = \"/dev/video42\"\nmodels_dir = \"/tmp\"\n"
    )
    .expect("write config");

    let resolved = resolve_camera_device_from_config(None, Some(&cfg_path));
    assert_eq!(
        resolved,
        PathBuf::from("/dev/video42"),
        "Must read camera_device from daemon config"
    );

    // Explicit CLI override takes precedence over daemon config
    let cli_override = PathBuf::from("/dev/video99");
    let resolved_override =
        resolve_camera_device_from_config(Some(cli_override.clone()), Some(&cfg_path));
    assert_eq!(
        resolved_override, cli_override,
        "CLI override must take precedence over config"
    );
}

#[test]
fn test_import_embedding_success() {
    let tmp = tempdir().expect("tempdir");
    let key_path = tmp.path().join("master.key");
    let bio_dir = tmp.path().join("biometrics");
    std::fs::create_dir_all(&bio_dir).expect("mkdir biometrics");

    let key = MasterKey::load_or_create(&key_path).expect("master key");
    let store = Arc::new(BiometricStore::new(&bio_dir, key).expect("store"));
    let service = EnrollmentService::new_store_only(store.clone(), false);

    // Create 512-dim embedding JSON file
    let embedding_file = tmp.path().join("embedding.json");
    let mock_embedding = vec![0.042f32; 512];
    let json_data = serde_json::to_string(&mock_embedding).expect("serialize");
    std::fs::write(&embedding_file, json_data).expect("write json");

    let args = ImportArgs {
        uid: Some(1000),
        username: Some("hadrien".to_string()),
        file: embedding_file,
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    };

    let outcome = service.import(&args).expect("import should succeed");
    assert_eq!(outcome.uid, 1000);
    assert_eq!(outcome.embedding_dim, 512);

    // Verify stored template can be retrieved and matches
    let loaded = store
        .get(1000)
        .expect("get template")
        .expect("template exists");
    assert_eq!(loaded.uid, 1000);
    assert_eq!(loaded.model_id, "arcface_w600k_mbf");
    assert_eq!(loaded.embedding.len(), 512);
    assert!((loaded.embedding[0] - 0.042).abs() < 1e-5);
}

#[test]
fn test_import_embedding_dimension_mismatch_fails() {
    let tmp = tempdir().expect("tempdir");
    let key_path = tmp.path().join("master.key");
    let bio_dir = tmp.path().join("biometrics");
    std::fs::create_dir_all(&bio_dir).expect("mkdir biometrics");

    let key = MasterKey::load_or_create(&key_path).expect("master key");
    let store = Arc::new(BiometricStore::new(&bio_dir, key).expect("store"));
    let service = EnrollmentService::new_store_only(store, false);

    // Create 128-dim embedding (invalid dimension for 512-dim ArcFace)
    let embedding_file = tmp.path().join("invalid_embedding.json");
    let mock_embedding = vec![0.042f32; 128];
    let json_data = serde_json::to_string(&mock_embedding).expect("serialize");
    std::fs::write(&embedding_file, json_data).expect("write json");

    let args = ImportArgs {
        uid: Some(1000),
        username: None,
        file: embedding_file,
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    };

    let res = service.import(&args);
    assert!(res.is_err(), "Must reject embedding with dimension != 512");
}

#[test]
fn test_import_nonexistent_file_fails() {
    let tmp = tempdir().expect("tempdir");
    let key_path = tmp.path().join("master.key");
    let bio_dir = tmp.path().join("biometrics");
    std::fs::create_dir_all(&bio_dir).expect("mkdir biometrics");

    let key = MasterKey::load_or_create(&key_path).expect("master key");
    let store = Arc::new(BiometricStore::new(&bio_dir, key).expect("store"));
    let service = EnrollmentService::new_store_only(store, false);

    let args = ImportArgs {
        uid: Some(1000),
        username: None,
        file: PathBuf::from("/nonexistent/path/embedding.json"),
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    };

    let res = service.import(&args);
    assert!(res.is_err(), "Must fail closed on non-existent file");
}

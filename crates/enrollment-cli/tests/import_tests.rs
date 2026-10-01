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
use soos_enrollment_cli::service::{
    resolve_camera_device_from_config, EnrollmentService, EMBEDDING_MODEL_VERSION,
    IMPORT_EMBEDDING_DIM, MODEL_ID_EMBEDDING,
};

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

    // Create a 128-dim (SFace) embedding JSON file
    let embedding_file = tmp.path().join("embedding.json");
    let mock_embedding = vec![0.042f32; 128];
    let json_data = serde_json::to_string(&mock_embedding).expect("serialize");
    std::fs::write(&embedding_file, json_data).expect("write json");

    let args = ImportArgs {
        uid: Some(1000),
        username: Some("hadrien".to_string()),
        file: embedding_file,
        model_id: MODEL_ID_EMBEDDING.to_string(),
        model_version: EMBEDDING_MODEL_VERSION.to_string(),
    };

    let outcome = service.import(&args).expect("import should succeed");
    assert_eq!(outcome.uid, 1000);
    assert_eq!(outcome.embedding_dim, 128);

    // Verify stored template can be retrieved and matches
    let loaded = store
        .get(1000)
        .expect("get template")
        .expect("template exists");
    assert_eq!(loaded.uid, 1000);
    assert_eq!(loaded.model_id, "sface_2021dec");
    assert_eq!(loaded.embedding.len(), 128);
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

    // 512-dim (retired ArcFace) and 127-dim embeddings are invalid for the 128-dim SFace model
    for dim in [512usize, 127] {
        let embedding_file = tmp.path().join(format!("invalid_embedding_{dim}.json"));
        let mock_embedding = vec![0.042f32; dim];
        let json_data = serde_json::to_string(&mock_embedding).expect("serialize");
        std::fs::write(&embedding_file, json_data).expect("write json");

        let args = ImportArgs {
            uid: Some(1000),
            username: None,
            file: embedding_file,
            model_id: MODEL_ID_EMBEDDING.to_string(),
            model_version: EMBEDDING_MODEL_VERSION.to_string(),
        };

        let res = service.import(&args);
        assert!(res.is_err(), "Must reject embedding with dimension {dim} != 128");
    }
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

/// SFC13 (GitHub #278, owner decision 2026-10-01): `import` defaults to the loaded embedding
/// model and refuses a template of any other model (embeddings are not convertible), even
/// with the right length; nothing is stored.
#[test]
fn test_import_refuses_a_template_of_another_model() {
    use clap::Parser;
    use soos_enrollment_cli::args::{Cli, Commands};

    assert_eq!(IMPORT_EMBEDDING_DIM, 128);
    let cli = Cli::try_parse_from(["soos-enroll", "import", "--file", "x.json"]).unwrap();
    match cli.command {
        Commands::Import(cmd) => {
            assert_eq!(cmd.args.model_id, MODEL_ID_EMBEDDING);
            assert_eq!(cmd.args.model_version, EMBEDDING_MODEL_VERSION);
        }
        other => panic!("unexpected command {other:?}"),
    }

    let tmp = tempdir().expect("tempdir");
    let key = MasterKey::load_or_create(tmp.path().join("master.key")).expect("master key");
    let store = Arc::new(BiometricStore::new(tmp.path().join("biometrics"), key).expect("store"));
    let service = EnrollmentService::new_store_only(store.clone(), false);

    // JSON payload labelled with the retired model.
    let json = tmp.path().join("arcface.json");
    std::fs::write(&json, serde_json::to_string(&vec![0.042f32; 128]).unwrap()).unwrap();
    let args = ImportArgs {
        uid: Some(1000),
        username: None,
        file: json,
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    };
    assert!(service.import(&args).is_err(), "a retired-model JSON import must be refused");

    // CBOR template recorded with the retired model (its own id wins over the arguments).
    let template = soos_biometric_store::BiometricTemplate::new(
        1000,
        "arcface_w600k_mbf".to_string(),
        "2.0.0".to_string(),
        1,
        zeroize::Zeroizing::new(vec![0.042f32; 512]),
    )
    .unwrap();
    let cbor = tmp.path().join("arcface.cbor");
    std::fs::write(&cbor, template.to_cbor().unwrap().as_slice()).unwrap();
    let args = ImportArgs {
        file: cbor,
        model_id: MODEL_ID_EMBEDDING.to_string(),
        ..args
    };
    assert!(service.import(&args).is_err(), "a retired-model CBOR import must be refused");
    assert!(store.get(1000).unwrap().is_none(), "nothing may be stored");
}

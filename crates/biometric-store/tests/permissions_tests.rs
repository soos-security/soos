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

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;
use zeroize::Zeroizing;

#[test]
fn test_b2_permissions_and_atomic_writes() {
    let tmp = TempDir::new().expect("tempdir");
    let store_dir = tmp.path().join("biometrics");
    let key = MasterKey::generate().expect("key");

    let store = BiometricStore::new(&store_dir, key).expect("store init");

    // Base directory permissions must be 0700 (drwx------)
    let dir_meta = std::fs::metadata(&store_dir).expect("dir metadata");
    let dir_mode = dir_meta.permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "storage directory must be mode 0700");

    let template = BiometricTemplate::new(
        1001,
        "facenet_v1".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(vec![0.5_f32; 128]),
    )
    .expect("template");

    store.enroll(&template).expect("enroll");

    let file_path = store.template_path(1001).expect("template path");
    assert!(file_path.exists());

    // File permissions must be 0600 (-rw-------)
    let file_meta = std::fs::metadata(&file_path).expect("file metadata");
    let file_mode = file_meta.permissions().mode() & 0o777;
    assert_eq!(file_mode, 0o600, "template file must be mode 0600");

    // Atomic write must leave zero leftover temporary files
    let entries: Vec<_> = std::fs::read_dir(&store_dir)
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().to_string())
        .collect();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], "1001.cbor.enc");
    assert!(!entries.iter().any(|name| name.contains(".tmp")));
}

#[test]
fn test_b2_master_key_file_permissions() {
    let tmp = TempDir::new().expect("tempdir");
    let key_path = tmp.path().join("master.key");

    let key = MasterKey::load_or_create(&key_path).expect("load or create key");
    assert!(key_path.exists());

    let key_meta = std::fs::metadata(&key_path).expect("key metadata");
    let key_mode = key_meta.permissions().mode() & 0o777;
    assert_eq!(
        key_mode, 0o600,
        "master key file must have 0600 permissions"
    );

    // Reloading the same key preserves key bytes
    let reloaded = MasterKey::load_or_create(&key_path).expect("reload key");
    assert_eq!(key.as_bytes(), reloaded.as_bytes());
}

#[test]
fn test_master_key_created_with_0600_from_inception() {
    let tmp = TempDir::new().expect("tempdir");
    let key_path = tmp.path().join("secure_master.key");

    let key = MasterKey::load_or_create(&key_path).expect("load or create key");
    assert!(key_path.exists());

    let key_meta = std::fs::symlink_metadata(&key_path).expect("key metadata");
    let key_mode = key_meta.permissions().mode() & 0o777;
    assert_eq!(
        key_mode, 0o600,
        "master key file must be created with 0600 permissions from inception"
    );
    assert!(!key_meta.file_type().is_symlink());
    assert_eq!(key.as_bytes().len(), 32);

    // Symlink attack prevention: if key_path is a symlink, load_or_create must reject it
    let symlink_dir = TempDir::new().expect("symlink dir");
    let decoy_target = symlink_dir.path().join("decoy_target");
    std::fs::write(&decoy_target, b"decoy").expect("write decoy");
    let symlink_key_path = symlink_dir.path().join("symlink.key");
    std::os::unix::fs::symlink(&decoy_target, &symlink_key_path).expect("create symlink");

    let err = MasterKey::load_or_create(&symlink_key_path);
    assert!(
        err.is_err(),
        "MasterKey::load_or_create must reject symlink path"
    );
}

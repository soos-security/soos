//! GitHub #289 (MLS6): `soos-enroll migrate` opens the template store through the
//! never-create constructor `BiometricStore::open_existing`, so a biometrics directory that
//! vanishes between the existence check and the open is never recreated ("migrate never
//! creates anything", ADR 2026-10-01 "Operator-Run Migration of Legacy v1 Storage Envelopes").

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;

use soos_biometric_store::MasterKey;
use soos_enrollment_cli::service::open_template_store_for_migration;
use tempfile::TempDir;

/// Body of the private `open_template_store_checked` in `service.rs`.
fn open_checked_body() -> &'static str {
    let service = include_str!("../src/service.rs");
    let start = service
        .find("fn open_template_store_checked(")
        .expect("open_template_store_checked exists");
    let rest = &service[start..];
    let end = rest.find("\n}\n").expect("end of the function");
    &rest[..end]
}

#[test]
fn test_mls_migrate_opens_the_template_store_without_creating_it() {
    let body = open_checked_body();
    assert!(
        body.contains("BiometricStore::open_existing("),
        "migrate must open the store with the never-create constructor"
    );
    assert!(
        !body.contains("BiometricStore::new("),
        "BiometricStore::new creates a missing directory; migrate must never call it"
    );
}

#[test]
fn test_mls_migrate_refuses_a_symlinked_biometrics_dir_and_creates_nothing() {
    let temp = TempDir::new().unwrap();
    let key_path = temp.path().join("master.key");
    MasterKey::load_or_create(&key_path).unwrap();
    let real = temp.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
    let link = temp.path().join("biometrics");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    assert!(
        open_template_store_for_migration(&link, &key_path).is_err(),
        "a symlinked biometrics directory is refused, never followed"
    );
    assert_eq!(fs::read_dir(&real).unwrap().count(), 0);
}

#[test]
fn test_mls_migrate_skips_a_missing_dir_under_a_missing_parent() {
    let temp = TempDir::new().unwrap();
    let key_path = temp.path().join("master.key");
    MasterKey::load_or_create(&key_path).unwrap();
    let bio_dir = temp.path().join("var").join("biometrics");

    let opened = open_template_store_for_migration(&bio_dir, &key_path).unwrap();
    assert!(opened.is_none(), "nothing to migrate");
    assert!(
        !temp.path().join("var").exists(),
        "no directory and no parent is created"
    );
}

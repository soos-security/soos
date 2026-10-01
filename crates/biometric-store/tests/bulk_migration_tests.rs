//! GitHub #287 (owner decision 2026-10-01): operator-run bulk migration of every legacy
//! unbound (v1) template to the AAD-bound (v2) envelope. Legacy files stay readable; the
//! migration is idempotent, never aborts on one bad file and never rewrites a refused file.
//!
//! Legacy fixtures are written exactly like `aad_migration_tests.rs`: the CBOR template sealed
//! with the unbound `encrypt_payload` codec.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and slicing"
)]

use std::os::unix::fs::{MetadataExt, PermissionsExt};

use soos_biometric_store::{
    encrypt_payload, BiometricStore, BiometricTemplate, MasterKey, PayloadFormat,
    TemplateMigration, TemplateMigrationReport,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

fn template(uid: u32, value: f32) -> BiometricTemplate {
    BiometricTemplate::new(
        uid,
        "arcface_w600k_mbf".to_string(),
        "2.0.0".to_string(),
        1_700_000_000 + u64::from(uid),
        Zeroizing::new(vec![value; 512]),
    )
    .unwrap()
}

fn open(temp: &TempDir) -> (BiometricStore, MasterKey) {
    let key = MasterKey::generate().unwrap();
    let store = BiometricStore::new(temp.path().join("bio"), key.clone()).unwrap();
    (store, key)
}

fn write_legacy(store: &BiometricStore, key: &MasterKey, template: &BiometricTemplate) {
    let cbor = template.to_cbor().unwrap();
    std::fs::write(
        store.template_path(template.uid).unwrap(),
        encrypt_payload(key, &cbor).unwrap(),
    )
    .unwrap();
}

fn failed_uids(report: &TemplateMigrationReport) -> Vec<u32> {
    report.failed.iter().map(|f| f.uid).collect()
}

#[test]
fn test_smi_bulk_migration_rewrites_only_legacy_templates() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    write_legacy(&store, &key, &template(1000, 0.25));
    write_legacy(&store, &key, &template(1002, 0.75));
    store.enroll(&template(1001, 0.5)).unwrap();
    let bound_before = std::fs::read(store.template_path(1001).unwrap()).unwrap();
    let before_1000 = store.get(1000).unwrap().unwrap();

    let report = store.migrate_legacy_templates(false).unwrap();

    assert!(!report.dry_run);
    assert_eq!(report.migrated, vec![1000, 1002]);
    assert_eq!(report.already_current, vec![1001]);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    for uid in [1000, 1001, 1002] {
        assert_eq!(
            store.template_format(uid).unwrap(),
            Some(PayloadFormat::BoundV2),
            "UID {uid} must be bound after migration"
        );
    }
    assert_eq!(
        std::fs::read(store.template_path(1001).unwrap()).unwrap(),
        bound_before,
        "an already bound template is never rewritten"
    );
    let after_1000 = store.get(1000).unwrap().unwrap();
    assert_eq!(after_1000.model_id, before_1000.model_id);
    assert_eq!(after_1000.model_version, before_1000.model_version);
    assert_eq!(
        after_1000.enrollment_timestamp,
        before_1000.enrollment_timestamp
    );
    assert_eq!(*after_1000.embedding, *before_1000.embedding);
}

#[test]
fn test_smi_bulk_migration_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    write_legacy(&store, &key, &template(1000, 0.25));

    let first = store.migrate_legacy_templates(false).unwrap();
    assert_eq!(first.migrated, vec![1000]);

    let second = store.migrate_legacy_templates(false).unwrap();
    assert!(second.migrated.is_empty(), "second run migrates nothing");
    assert_eq!(second.already_current, vec![1000]);
    assert!(second.failed.is_empty());
}

#[test]
fn test_smi_dry_run_reports_without_writing() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    write_legacy(&store, &key, &template(1000, 0.25));
    store.enroll(&template(1001, 0.5)).unwrap();
    let legacy_bytes = std::fs::read(store.template_path(1000).unwrap()).unwrap();

    let report = store.migrate_legacy_templates(true).unwrap();

    assert!(report.dry_run);
    assert_eq!(report.migrated, vec![1000], "would be migrated");
    assert_eq!(report.already_current, vec![1001]);
    assert!(report.failed.is_empty());
    assert_eq!(
        std::fs::read(store.template_path(1000).unwrap()).unwrap(),
        legacy_bytes,
        "a dry run never rewrites a file"
    );
    assert_eq!(
        store.template_format(1000).unwrap(),
        Some(PayloadFormat::LegacyV1)
    );
}

#[test]
fn test_smi_failure_of_one_file_does_not_abort_or_corrupt() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    write_legacy(&store, &key, &template(1000, 0.25));
    write_legacy(&store, &key, &template(1003, 0.75));

    // 1001: legacy template whose embedded UID is foreign (planted / moved file).
    let foreign = encrypt_payload(&key, &template(1000, 0.5).to_cbor().unwrap()).unwrap();
    std::fs::write(store.template_path(1001).unwrap(), &foreign).unwrap();
    // 1002: junk that authenticates neither as bound nor as legacy.
    let junk = b"SOOSBIO1 definitely not a valid ciphertext body".to_vec();
    std::fs::write(store.template_path(1002).unwrap(), &junk).unwrap();

    let report = store.migrate_legacy_templates(false).unwrap();

    assert_eq!(report.migrated, vec![1000, 1003]);
    assert!(report.already_current.is_empty());
    assert_eq!(failed_uids(&report), vec![1001, 1002]);
    for failure in &report.failed {
        assert!(!failure.error.is_empty(), "each failure carries a reason");
    }
    assert_eq!(
        std::fs::read(store.template_path(1001).unwrap()).unwrap(),
        foreign,
        "a refused file is left byte-for-byte untouched"
    );
    assert_eq!(
        std::fs::read(store.template_path(1002).unwrap()).unwrap(),
        junk
    );
}

#[test]
fn test_smi_symlinked_template_is_reported_and_never_followed() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    let outside = temp.path().join("outside.bin");
    let legacy = encrypt_payload(&key, &template(1000, 0.25).to_cbor().unwrap()).unwrap();
    std::fs::write(&outside, &legacy).unwrap();
    let link = store.template_path(1000).unwrap();
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    let report = store.migrate_legacy_templates(false).unwrap();

    assert!(report.migrated.is_empty());
    assert_eq!(failed_uids(&report), vec![1000]);
    assert_eq!(std::fs::read(&outside).unwrap(), legacy, "target untouched");
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn test_smi_migrated_file_is_mode_0600_and_leaves_no_temporary_file() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    write_legacy(&store, &key, &template(1000, 0.25));
    std::fs::set_permissions(
        store.template_path(1000).unwrap(),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let report = store.migrate_legacy_templates(false).unwrap();
    assert_eq!(report.migrated, vec![1000]);

    let meta = std::fs::metadata(store.template_path(1000).unwrap()).unwrap();
    assert_eq!(meta.mode() & 0o7777, 0o600);
    let names: Vec<String> = std::fs::read_dir(temp.path().join("bio"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["1000.cbor.enc".to_string()], "{names:?}");
}

#[test]
fn test_smi_migrate_template_reports_each_outcome() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    write_legacy(&store, &key, &template(1000, 0.25));
    store.enroll(&template(1001, 0.5)).unwrap();

    assert_eq!(
        store.migrate_template(4242, false).unwrap(),
        TemplateMigration::Missing
    );
    assert_eq!(
        store.migrate_template(1001, false).unwrap(),
        TemplateMigration::AlreadyCurrent
    );
    assert_eq!(
        store.migrate_template(1000, true).unwrap(),
        TemplateMigration::WouldMigrate
    );
    assert_eq!(
        store.template_format(1000).unwrap(),
        Some(PayloadFormat::LegacyV1)
    );
    assert_eq!(
        store.migrate_template(1000, false).unwrap(),
        TemplateMigration::Migrated
    );
    assert_eq!(
        store.migrate_template(1000, false).unwrap(),
        TemplateMigration::AlreadyCurrent
    );
}

#[test]
fn test_smi_failure_reasons_never_contain_embedding_values() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    let foreign = encrypt_payload(&key, &template(1000, 0.123_456).to_cbor().unwrap()).unwrap();
    std::fs::write(store.template_path(1001).unwrap(), foreign).unwrap();

    let report = store.migrate_legacy_templates(false).unwrap();
    let rendered = format!("{report:?}");
    assert_eq!(failed_uids(&report), vec![1001]);
    assert!(
        !rendered.contains("0.123"),
        "reports carry UIDs and reasons only: {rendered}"
    );
}

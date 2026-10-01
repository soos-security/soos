//! GitHub #287 (owner decision 2026-10-01): operator-run migration of every legacy unbound
//! (v1) evidence snapshot to the AAD-bound (v2) envelope. Legacy snapshots stay readable;
//! the migration is idempotent, never aborts on one bad file, never rewrites a refused file
//! and never creates a key.
//!
//! Legacy fixtures are written like `aad_binding_tests.rs`: a stored snapshot whose content is
//! replaced by the unbound `encrypt_payload` encryption of the same record.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use soos_evidence_store::crypto::{decrypt_snapshot_payload, encrypt_payload, PayloadFormat};
use soos_evidence_store::{EvidenceConfig, EvidenceStore, EvidenceStoreError, MasterKey};
use tempfile::TempDir;

fn open_store(temp: &TempDir) -> (EvidenceStore, MasterKey) {
    let key = MasterKey::generate().unwrap();
    let config = EvidenceConfig {
        enabled: true,
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: 100,
    };
    (EvidenceStore::new(config, key.clone()), key)
}

fn store_one(store: &EvidenceStore, date: &str, ts: u64) -> PathBuf {
    store
        .store_snapshot(1000, "PasswordFailed", b"frame", Some(date), Some(ts))
        .unwrap()
        .path
}

/// Replaces a stored snapshot with the legacy unbound encryption of the same record.
fn make_legacy(store: &EvidenceStore, key: &MasterKey, path: &Path) -> Vec<u8> {
    let record = store.load_snapshot(path).unwrap();
    let legacy = encrypt_payload(key, &record.to_cbor().unwrap()).unwrap();
    fs::write(path, &legacy).unwrap();
    legacy
}

fn format_of(key: &MasterKey, path: &Path) -> PayloadFormat {
    let date = path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let id = name.split('.').next().unwrap();
    decrypt_snapshot_payload(key, date, id, &fs::read(path).unwrap())
        .unwrap()
        .1
}

#[test]
fn test_smi_snapshot_migration_rewrites_only_legacy_files() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    let legacy_a = store_one(&store, "2026-09-14", 1);
    let legacy_b = store_one(&store, "2026-09-15", 2);
    let bound = store_one(&store, "2026-09-15", 3);
    make_legacy(&store, &key, &legacy_a);
    make_legacy(&store, &key, &legacy_b);
    let bound_bytes = fs::read(&bound).unwrap();
    let before = store.load_snapshot(&legacy_a).unwrap();

    let report = store.migrate_legacy_snapshots(false).unwrap();

    assert!(!report.dry_run);
    let mut expected = vec![legacy_a.clone(), legacy_b.clone()];
    expected.sort();
    assert_eq!(report.migrated, expected);
    assert_eq!(report.already_current, vec![bound.clone()]);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(format_of(&key, &legacy_a), PayloadFormat::BoundV2);
    assert_eq!(format_of(&key, &legacy_b), PayloadFormat::BoundV2);
    assert_eq!(
        fs::read(&bound).unwrap(),
        bound_bytes,
        "bound file untouched"
    );
    assert_eq!(store.load_snapshot(&legacy_a).unwrap(), before);
}

#[test]
fn test_smi_snapshot_migration_is_idempotent_and_dry_run_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    let path = store_one(&store, "2026-09-14", 1);
    let legacy = make_legacy(&store, &key, &path);

    let dry = store.migrate_legacy_snapshots(true).unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.migrated, vec![path.clone()]);
    assert_eq!(fs::read(&path).unwrap(), legacy, "dry run never writes");

    let first = store.migrate_legacy_snapshots(false).unwrap();
    assert_eq!(first.migrated, vec![path.clone()]);
    let second = store.migrate_legacy_snapshots(false).unwrap();
    assert!(second.migrated.is_empty());
    assert_eq!(second.already_current, vec![path.clone()]);
    assert!(second.failed.is_empty());
}

#[test]
fn test_smi_snapshot_failure_does_not_abort_or_corrupt() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    let good = store_one(&store, "2026-09-14", 1);
    make_legacy(&store, &key, &good);
    let junk = temp
        .path()
        .join("evidence/2026-09-14/00000000-0000-4000-8000-000000000000.opaque.enc");
    fs::write(&junk, b"SOOSEVD1 not a ciphertext at all, never decrypts").unwrap();
    let junk_bytes = fs::read(&junk).unwrap();

    // A legacy record whose embedded snapshot id differs from its file name is refused.
    let other = store_one(&store, "2026-09-14", 2);
    let record = store.load_snapshot(&other).unwrap();
    let renamed = other.with_file_name("11111111-1111-4111-8111-111111111111.opaque.enc");
    let renamed_bytes = encrypt_payload(&key, &record.to_cbor().unwrap()).unwrap();
    fs::write(&renamed, &renamed_bytes).unwrap();

    let report = store.migrate_legacy_snapshots(false).unwrap();

    assert_eq!(report.migrated, vec![good.clone()]);
    assert_eq!(report.already_current, vec![other.clone()]);
    let mut failed: Vec<PathBuf> = report.failed.iter().map(|f| f.path.clone()).collect();
    failed.sort();
    let mut expected = vec![junk.clone(), renamed.clone()];
    expected.sort();
    assert_eq!(failed, expected);
    for failure in &report.failed {
        assert!(!failure.error.is_empty());
    }
    assert_eq!(fs::read(&junk).unwrap(), junk_bytes);
    assert_eq!(fs::read(&renamed).unwrap(), renamed_bytes);
}

#[test]
fn test_smi_snapshot_migration_writes_0600_and_leaves_no_temporary_file() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    let path = store_one(&store, "2026-09-14", 1);
    make_legacy(&store, &key, &path);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let names_before: Vec<String> = dir_names(path.parent().unwrap());

    let report = store.migrate_legacy_snapshots(false).unwrap();
    assert_eq!(report.migrated, vec![path.clone()]);

    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o7777, 0o600);
    assert_eq!(dir_names(path.parent().unwrap()), names_before);
    // The daily counters are not touched: migration is not a capture.
    assert_eq!(store.daily_count(1000, "2026-09-14"), 1);
}

#[test]
fn test_smi_snapshot_migration_never_follows_symlinks() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    let path = store_one(&store, "2026-09-14", 1);
    let legacy = make_legacy(&store, &key, &path);

    // A symlinked date partition and a symlinked snapshot are skipped, their targets untouched.
    let outside_dir = temp.path().join("outside");
    fs::create_dir(&outside_dir).unwrap();
    let outside_file = outside_dir.join("22222222-2222-4222-8222-222222222222.opaque.enc");
    fs::write(&outside_file, &legacy).unwrap();
    std::os::unix::fs::symlink(&outside_dir, temp.path().join("evidence/2026-09-13")).unwrap();
    std::os::unix::fs::symlink(
        &outside_file,
        temp.path()
            .join("evidence/2026-09-14/22222222-2222-4222-8222-222222222222.opaque.enc"),
    )
    .unwrap();

    let report = store.migrate_legacy_snapshots(false).unwrap();
    assert_eq!(report.migrated, vec![path.clone()]);
    assert_eq!(fs::read(&outside_file).unwrap(), legacy);
}

#[test]
fn test_smi_snapshot_migration_of_missing_store_is_empty_and_symlinked_root_refused() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let report = store.migrate_legacy_snapshots(false).unwrap();
    assert!(report.migrated.is_empty() && report.already_current.is_empty());
    assert!(report.failed.is_empty());
    assert!(!temp.path().join("evidence").exists(), "nothing is created");

    let real = temp.path().join("real");
    fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, temp.path().join("evidence")).unwrap();
    assert!(matches!(
        store.migrate_legacy_snapshots(false),
        Err(EvidenceStoreError::InvalidPath(_))
    ));
}

#[test]
fn test_smi_load_existing_key_never_creates_a_key() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("evidence.key");
    assert!(MasterKey::load_existing(&path).is_err());
    assert!(!path.exists(), "a missing key is never created");

    let created = MasterKey::load_or_create(&path).unwrap();
    let loaded = MasterKey::load_existing(&path).unwrap();
    assert_eq!(created, loaded);

    let link = temp.path().join("link.key");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(MasterKey::load_existing(&link).is_err(), "symlinks refused");
}

fn dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn test_smi_snapshot_migration_overwrites_legacy_ciphertext_best_effort() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    let path = store_one(&store, "2026-09-14", 1);
    let legacy = make_legacy(&store, &key, &path);
    // A second name for the legacy inode, outside the store, observes its content after the
    // migration replaced the path with a new file.
    let old_inode = temp.path().join("legacy-inode");
    fs::hard_link(&path, &old_inode).unwrap();

    let report = store.migrate_legacy_snapshots(false).unwrap();
    assert_eq!(report.migrated, vec![path.clone()]);
    assert!(report.failed.is_empty(), "{:?}", report.failed);

    let residue = fs::read(&old_inode).unwrap();
    assert_eq!(residue.len(), legacy.len(), "overwritten in place");
    assert_ne!(residue, legacy, "the legacy ciphertext is overwritten");
    assert_eq!(format_of(&key, &path), PayloadFormat::BoundV2);
}

#[test]
fn test_smi_snapshot_dry_run_never_overwrites_legacy_ciphertext() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    let path = store_one(&store, "2026-09-14", 1);
    let legacy = make_legacy(&store, &key, &path);
    let old_inode = temp.path().join("legacy-inode");
    fs::hard_link(&path, &old_inode).unwrap();

    store.migrate_legacy_snapshots(true).unwrap();
    assert_eq!(fs::read(&old_inode).unwrap(), legacy);
    assert_eq!(fs::read(&path).unwrap(), legacy);
}

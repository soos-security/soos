//! GitHub #289 (storage follow-ups of #287): the advisory store lock and the never-create
//! constructor.
//!
//! - MLS3: `enroll`, `delete` and a real `migrate_template` serialize on an exclusive
//!   `flock` of the store directory; a contended lock gives up after the lock timeout with a
//!   clear `LockTimeout` error and changes nothing. Dry runs and reads never wait for it.
//! - MLS4: the lock leaves no file in the store directory and is released after every call.
//! - MLS5: `BiometricStore::open_existing` never creates the directory and validates an
//!   existing one exactly like `BiometricStore::new`.
//!
//! The lock is held from the test through `nix::fcntl::Flock` on the store directory, the
//! documented on-disk protocol, so the contention is deterministic.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and slicing"
)]

use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, FlockArg};
use soos_biometric_store::{
    encrypt_payload, BiometricStore, BiometricStoreError, BiometricTemplate, MasterKey,
    PayloadFormat, TemplateMigration, STORE_LOCK_TIMEOUT,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

const SHORT_TIMEOUT: Duration = Duration::from_millis(150);

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
    let store = BiometricStore::new(temp.path().join("bio"), key.clone())
        .unwrap()
        .with_lock_timeout(SHORT_TIMEOUT);
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

fn hold_lock(dir: &Path) -> Flock<File> {
    Flock::lock(File::open(dir).unwrap(), FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .expect("the test takes the store lock first")
}

fn assert_lock_timeout(result: Result<impl std::fmt::Debug, BiometricStoreError>, dir: &Path) {
    match result {
        Err(BiometricStoreError::LockTimeout(msg)) => {
            assert!(
                msg.contains(&dir.display().to_string()),
                "the error names the store directory: {msg}"
            );
            assert!(
                msg.contains("locked by another"),
                "the error says why it gave up: {msg}"
            );
        }
        other => panic!("expected LockTimeout, got {other:?}"),
    }
}

#[test]
fn test_mls_default_lock_timeout_is_bounded() {
    assert!(STORE_LOCK_TIMEOUT > Duration::ZERO);
    assert!(STORE_LOCK_TIMEOUT <= Duration::from_secs(30));
}

#[test]
fn test_mls_enroll_times_out_on_a_held_lock_and_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let (store, _key) = open(&temp);
    let dir = temp.path().join("bio");
    let _held = hold_lock(&dir);

    let started = Instant::now();
    let result = store.enroll(&template(1000, 0.25));
    let elapsed = started.elapsed();

    assert_lock_timeout(result, &dir);
    assert!(
        elapsed >= SHORT_TIMEOUT,
        "the wait lasts the lock timeout, took {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "the wait is bounded, took {elapsed:?}"
    );
    assert!(!store.exists(1000).unwrap(), "nothing is written");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "no temporary file is left"
    );
}

#[test]
fn test_mls_delete_times_out_on_a_held_lock_and_keeps_the_template() {
    let temp = TempDir::new().unwrap();
    let (store, _key) = open(&temp);
    let dir = temp.path().join("bio");
    store.enroll(&template(1000, 0.25)).unwrap();
    let before = std::fs::read(store.template_path(1000).unwrap()).unwrap();
    let _held = hold_lock(&dir);

    assert_lock_timeout(store.delete(1000), &dir);
    assert_eq!(
        std::fs::read(store.template_path(1000).unwrap()).unwrap(),
        before,
        "the template is untouched"
    );
}

#[test]
fn test_mls_migrate_times_out_on_a_held_lock_and_keeps_the_legacy_file() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    let dir = temp.path().join("bio");
    write_legacy(&store, &key, &template(1000, 0.25));
    let before = std::fs::read(store.template_path(1000).unwrap()).unwrap();
    let _held = hold_lock(&dir);

    assert_lock_timeout(store.migrate_template(1000, false), &dir);
    let report = store.migrate_legacy_templates(false).unwrap();
    assert!(report.migrated.is_empty());
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].uid, 1000);
    assert!(
        report.failed[0].error.contains("locked by another"),
        "the bulk report carries the lock failure: {}",
        report.failed[0].error
    );
    assert_eq!(
        std::fs::read(store.template_path(1000).unwrap()).unwrap(),
        before,
        "the legacy file is untouched"
    );
}

#[test]
fn test_mls_dry_run_and_reads_do_not_wait_for_the_lock() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    let dir = temp.path().join("bio");
    write_legacy(&store, &key, &template(1000, 0.25));
    let _held = hold_lock(&dir);

    assert_eq!(
        store.migrate_template(1000, true).unwrap(),
        TemplateMigration::WouldMigrate
    );
    assert!(store.get(1000).unwrap().is_some());
    assert_eq!(store.list_enrolled().unwrap(), vec![1000]);
    assert_eq!(
        store.template_format(1000).unwrap(),
        Some(PayloadFormat::LegacyV1)
    );
}

#[test]
fn test_mls_lock_is_released_and_leaves_no_file() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    let dir = temp.path().join("bio");

    store.enroll(&template(1000, 0.25)).unwrap();
    write_legacy(&store, &key, &template(1001, 0.5));
    assert_eq!(
        store.migrate_template(1001, false).unwrap(),
        TemplateMigration::Migrated
    );
    assert!(store.delete(1000).unwrap());
    // A failed write (oversized template) must release the lock as well.
    let huge = BiometricTemplate::new(
        1002,
        "arcface_w600k_mbf".to_string(),
        "2.0.0".to_string(),
        1,
        Zeroizing::new(vec![0.123; 20_000]),
    )
    .unwrap();
    assert!(store.enroll(&huge).is_err());

    // Every call above released the lock: the test can take it without waiting.
    drop(hold_lock(&dir));
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["1001.cbor.enc".to_string()],
        "the lock leaves no file in the store directory"
    );
}

#[test]
fn test_mls_open_existing_never_creates_the_directory() {
    let temp = TempDir::new().unwrap();
    let missing = temp.path().join("parent").join("bio");
    let key = MasterKey::generate().unwrap();

    match BiometricStore::open_existing(&missing, key) {
        Err(BiometricStoreError::Io(e)) => {
            assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
    assert!(!missing.exists(), "the directory is never created");
    assert!(
        !temp.path().join("parent").exists(),
        "no parent directory is created"
    );
}

#[test]
fn test_mls_open_existing_validates_like_new() {
    let temp = TempDir::new().unwrap();
    let key = MasterKey::generate().unwrap();

    let dir = temp.path().join("bio");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = BiometricStore::open_existing(&dir, key.clone()).unwrap();
    store.enroll(&template(1000, 0.25)).unwrap();
    assert!(store.get(1000).unwrap().is_some());

    let link = temp.path().join("link");
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    assert!(matches!(
        BiometricStore::open_existing(&link, key.clone()),
        Err(BiometricStoreError::InvalidPath(_))
    ));

    let writable = temp.path().join("writable");
    std::fs::create_dir(&writable).unwrap();
    std::fs::set_permissions(&writable, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        BiometricStore::open_existing(&writable, key.clone()),
        Err(BiometricStoreError::InvalidPath(_))
    ));
    assert_eq!(
        std::fs::metadata(&writable).unwrap().permissions().mode() & 0o777,
        0o777,
        "the permissions of a refused directory are never changed"
    );

    let file = temp.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    assert!(matches!(
        BiometricStore::open_existing(&file, key),
        Err(BiometricStoreError::InvalidPath(_))
    ));
}

//! GitHub #266 (STO-22): evidence snapshots are bound to their date partition and snapshot id
//! (and to the format version and file role) through AES-GCM associated data. Snapshots
//! written before the change (unbound) stay readable.
//!
//! These tests use only the pre-existing public API so that they fail at runtime (not at
//! compile time) on the unbound implementation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::fs;

use soos_evidence_store::crypto::{decrypt_payload, encrypt_payload, MAGIC_HEADER};
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey};
use tempfile::TempDir;

const EXPECTED_BOUND_MARKER: &[u8; 4] = b"AAD\x02";

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

#[test]
fn test_sad_stored_snapshot_declares_bound_format() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let stored = store
        .store_snapshot(
            1000,
            "PasswordFailed",
            b"frame",
            Some("2026-09-14"),
            Some(1),
        )
        .unwrap();
    let bytes = fs::read(&stored.path).unwrap();
    assert_eq!(&bytes[..8], MAGIC_HEADER);
    assert_eq!(&bytes[8..12], EXPECTED_BOUND_MARKER);
}

#[test]
fn test_sad_stored_snapshot_is_not_readable_as_unbound_payload() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    let stored = store
        .store_snapshot(
            1000,
            "PasswordFailed",
            b"frame",
            Some("2026-09-14"),
            Some(1),
        )
        .unwrap();
    let bytes = fs::read(&stored.path).unwrap();
    assert!(
        decrypt_payload(&key, &bytes).is_err(),
        "a new snapshot must not authenticate without its associated data"
    );
}

#[test]
fn test_sad_snapshot_moved_to_another_date_partition_is_refused() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let stored = store
        .store_snapshot(
            1000,
            "PasswordFailed",
            b"frame",
            Some("2026-09-14"),
            Some(1),
        )
        .unwrap();
    let other_day = temp.path().join("evidence").join("2026-09-13");
    fs::create_dir_all(&other_day).unwrap();
    let moved = other_day.join(stored.path.file_name().unwrap());
    fs::copy(&stored.path, &moved).unwrap();

    assert!(store.load_snapshot(&stored.path).is_ok());
    assert!(
        store.load_snapshot(&moved).is_err(),
        "a snapshot moved to another date partition must fail authentication"
    );
}

#[test]
fn test_sad_snapshot_renamed_to_another_id_is_refused() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let stored = store
        .store_snapshot(
            1000,
            "PasswordFailed",
            b"frame",
            Some("2026-09-14"),
            Some(1),
        )
        .unwrap();
    let renamed = stored
        .path
        .with_file_name("00000000-0000-4000-8000-000000000000.opaque.enc");
    fs::copy(&stored.path, &renamed).unwrap();
    assert!(
        store.load_snapshot(&renamed).is_err(),
        "a snapshot renamed to another snapshot id must fail authentication"
    );
}

#[test]
fn test_sad_legacy_unbound_snapshot_in_partition_is_still_readable() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    // Write a snapshot, then replace its content with the legacy unbound encryption of the
    // same record, exactly as releases before #266 stored it.
    let stored = store
        .store_snapshot(
            1000,
            "PasswordFailed",
            b"frame",
            Some("2026-09-14"),
            Some(7),
        )
        .unwrap();
    let record = store.load_snapshot(&stored.path).unwrap();
    let legacy = encrypt_payload(&key, &record.to_cbor().unwrap()).unwrap();
    fs::write(&stored.path, legacy).unwrap();

    let loaded = store.load_snapshot(&stored.path).unwrap();
    assert_eq!(loaded.snapshot_id, stored.snapshot_id);
    assert_eq!(loaded.uid, 1000);
    assert_eq!(loaded.timestamp, 7);
    assert_eq!(loaded.image_data, b"frame".to_vec());
}

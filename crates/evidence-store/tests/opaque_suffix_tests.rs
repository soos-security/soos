//! Contractual tests for the opaque evidence snapshot suffix (GitHub #278 follow-up, matrix
//! row SFU4).
//!
//! Contract:
//! - `store_snapshot` writes `<uuid>.opaque.enc`; the suffix no longer claims a WebP payload.
//! - Legacy `<uuid>.webp.enc` files written by earlier releases are still listed, still
//!   decrypt and are still purged by retention rotation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions"
)]

use soos_evidence_store::{EvidenceConfig, EvidenceStore, OPAQUE_SNAPSHOT_EXTENSION};
use std::fs;
use tempfile::TempDir;

fn store(temp: &TempDir) -> EvidenceStore {
    EvidenceStore::open(EvidenceConfig {
        enabled: true,
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: 10,
    })
    .unwrap()
}

#[test]
fn test_opaque_snapshot_suffix_is_opaque_enc() {
    assert_eq!(OPAQUE_SNAPSHOT_EXTENSION, ".opaque.enc");
    assert!(!OPAQUE_SNAPSHOT_EXTENSION.contains("webp"));

    let temp = TempDir::new().unwrap();
    let result = store(&temp)
        .store_snapshot(1000, "tamper_detected", b"opaque", Some("2026-09-14"), None)
        .unwrap();
    let name = result
        .path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(name.ends_with(".opaque.enc"), "unexpected name {name}");
}

#[test]
fn test_legacy_webp_enc_snapshot_is_listed_decrypted_and_purged() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);
    let result = store
        .store_snapshot(1000, "tamper_detected", b"legacy", Some("2026-09-01"), None)
        .unwrap();

    // Simulate a file written by a release that still used the `.webp.enc` suffix.
    let legacy = result
        .path
        .with_file_name(format!("{}.webp.enc", result.snapshot_id));
    fs::rename(&result.path, &legacy).unwrap();

    assert_eq!(
        store.list_snapshots_for_date("2026-09-01").unwrap(),
        vec![legacy.clone()]
    );
    store
        .load_snapshot(&legacy)
        .expect("legacy file must still decrypt");

    let report = store.rotate_retention("2026-09-14").unwrap();
    assert_eq!(report.pruned_dates, vec!["2026-09-01".to_string()]);
    assert!(
        !legacy.exists(),
        "legacy snapshot must be purged by retention"
    );
}

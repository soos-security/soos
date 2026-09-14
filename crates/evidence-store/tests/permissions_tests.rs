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

use soos_evidence_store::{EvidenceConfig, EvidenceStore};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

#[test]
fn test_evidence_file_and_directory_permissions() {
    let temp = TempDir::new().unwrap();
    let config = EvidenceConfig {
        enabled: true,
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: 10,
    };

    let store = EvidenceStore::open(config).unwrap();
    let frame_bytes = vec![0xCA, 0xFE, 0xBA, 0xBE];

    let result = store
        .store_snapshot(
            1001,
            "tamper_detected",
            &frame_bytes,
            Some("2026-09-14"),
            Some(1726310000),
        )
        .unwrap();

    // 1. Verify snapshot file permissions are strictly 0600 (-rw-------)
    let file_meta = fs::metadata(&result.path).unwrap();
    let file_mode = file_meta.permissions().mode() & 0o777;
    assert_eq!(
        file_mode, 0o600,
        "ACCEPTANCE CRITERION E4 VIOLATION: Evidence snapshot must have mode 0600, got: {:o}",
        file_mode
    );

    // 2. Verify date partition directory permissions are strictly 0700 (drwx------)
    let parent_dir = result.path.parent().unwrap();
    let dir_meta = fs::metadata(parent_dir).unwrap();
    let dir_mode = dir_meta.permissions().mode() & 0o777;
    assert_eq!(
        dir_mode, 0o700,
        "ACCEPTANCE CRITERION E4 VIOLATION: Partition directory must have mode 0700, got: {:o}",
        dir_mode
    );

    // 3. Verify master key permissions are strictly 0600
    let key_meta = fs::metadata(temp.path().join("evidence.key")).unwrap();
    let key_mode = key_meta.permissions().mode() & 0o777;
    assert_eq!(
        key_mode, 0o600,
        "ACCEPTANCE CRITERION E4 VIOLATION: Key file must have mode 0600, got: {:o}",
        key_mode
    );
}

#[test]
fn test_evidence_filename_format() {
    let temp = TempDir::new().unwrap();
    let config = EvidenceConfig {
        enabled: true,
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: 10,
    };

    let store = EvidenceStore::open(config).unwrap();
    let result = store
        .store_snapshot(
            1000,
            "unauthorized_access",
            b"test_bytes",
            Some("2026-09-14"),
            None,
        )
        .unwrap();

    let filename = result.path.file_name().unwrap().to_str().unwrap();
    assert!(
        filename.ends_with(".webp.enc"),
        "Filename must end with .webp.enc, got: {filename}"
    );

    let uuid_part = &filename[..filename.len() - ".webp.enc".len()];
    assert_eq!(
        uuid_part.len(),
        36,
        "Snapshot UUID part must be 36 characters (UUID v4 format), got: {uuid_part}"
    );
    assert_eq!(uuid_part, result.snapshot_id);
}

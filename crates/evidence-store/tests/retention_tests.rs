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
use tempfile::TempDir;

#[test]
fn test_retention_rotation_7_days() {
    let temp = TempDir::new().unwrap();
    let base_dir = temp.path().join("evidence");
    let key_path = temp.path().join("evidence.key");
    fs::create_dir_all(&base_dir).unwrap();

    let config = EvidenceConfig {
        enabled: true,
        base_dir: base_dir.clone(),
        key_path,
        retention_days: 7,
        daily_cap_per_uid: 10,
    };

    let store = EvidenceStore::open(config).unwrap();

    // Create snapshots across multiple dates
    let dates = [
        ("2026-09-01", true),  // 13 days old -> should be pruned
        ("2026-09-04", true),  // 10 days old -> should be pruned
        ("2026-09-06", true),  // 8 days old -> should be pruned
        ("2026-09-07", false), // 7 days old -> kept
        ("2026-09-10", false), // 4 days old -> kept
        ("2026-09-14", false), // today -> kept
    ];

    for (date_str, _) in &dates {
        store
            .store_snapshot(
                1000,
                "test_retention",
                b"test_image_bytes",
                Some(date_str),
                None,
            )
            .unwrap();
        assert!(base_dir.join(date_str).exists());
    }

    // Also create a non-date directory to ensure robust error handling
    let custom_dir = base_dir.join("non_date_folder");
    fs::create_dir_all(&custom_dir).unwrap();

    // Execute rotation as of "2026-09-14"
    let report = store.rotate_retention("2026-09-14").unwrap();

    assert_eq!(
        report.directories_pruned, 3,
        "ACCEPTANCE CRITERION E2 VIOLATION: Expected exactly 3 directories pruned, got {}",
        report.directories_pruned
    );
    assert!(report.pruned_dates.contains(&"2026-09-01".to_string()));
    assert!(report.pruned_dates.contains(&"2026-09-04".to_string()));
    assert!(report.pruned_dates.contains(&"2026-09-06".to_string()));

    // Verify pruned directories are deleted from filesystem
    assert!(!base_dir.join("2026-09-01").exists());
    assert!(!base_dir.join("2026-09-04").exists());
    assert!(!base_dir.join("2026-09-06").exists());

    // Verify retained directories still exist
    assert!(base_dir.join("2026-09-07").exists());
    assert!(base_dir.join("2026-09-10").exists());
    assert!(base_dir.join("2026-09-14").exists());
    assert!(
        custom_dir.exists(),
        "Non-date directories should not be deleted"
    );
}

#[test]
fn test_retention_rotation_custom_days() {
    let temp = TempDir::new().unwrap();
    let base_dir = temp.path().join("evidence");
    let key_path = temp.path().join("evidence.key");
    fs::create_dir_all(&base_dir).unwrap();

    let config = EvidenceConfig {
        enabled: true,
        base_dir: base_dir.clone(),
        key_path,
        retention_days: 3, // custom 3-day retention
        ..Default::default()
    };

    let store = EvidenceStore::open(config).unwrap();

    store
        .store_snapshot(1000, "test", b"bytes", Some("2026-09-10"), None)
        .unwrap();
    store
        .store_snapshot(1000, "test", b"bytes", Some("2026-09-11"), None)
        .unwrap();
    store
        .store_snapshot(1000, "test", b"bytes", Some("2026-09-14"), None)
        .unwrap();

    let report = store.rotate_retention("2026-09-14").unwrap();
    assert_eq!(report.directories_pruned, 1);
    assert!(report.pruned_dates.contains(&"2026-09-10".to_string()));
    assert!(!base_dir.join("2026-09-10").exists());
    assert!(base_dir.join("2026-09-11").exists());
    assert!(base_dir.join("2026-09-14").exists());
}

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

use soos_evidence_store::{
    EvidenceConfig, EvidenceStore, EvidenceStoreError, DEFAULT_DAILY_CAP_PER_UID,
    DEFAULT_EVIDENCE_DIR, DEFAULT_KEY_PATH, DEFAULT_RETENTION_DAYS,
};
use tempfile::TempDir;

#[test]
fn test_evidence_config_disabled_by_default() {
    let config = EvidenceConfig::default();
    assert!(
        !config.enabled,
        "ACCEPTANCE CRITERION E1 VIOLATION: Evidence storage must be disabled by default!"
    );
    assert_eq!(config.retention_days, DEFAULT_RETENTION_DAYS);
    assert_eq!(config.retention_days, 7);
    assert_eq!(config.daily_cap_per_uid, DEFAULT_DAILY_CAP_PER_UID);
    assert_eq!(config.daily_cap_per_uid, 10);
    assert_eq!(config.base_dir.to_str().unwrap(), DEFAULT_EVIDENCE_DIR);
    assert_eq!(config.key_path.to_str().unwrap(), DEFAULT_KEY_PATH);
}

#[test]
fn test_store_snapshot_rejected_when_disabled() {
    let temp = TempDir::new().unwrap();
    let config = EvidenceConfig {
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        ..Default::default()
    };
    assert!(!config.enabled);

    let store = EvidenceStore::open(config).unwrap();
    assert!(!store.config().enabled);

    let dummy_frame = vec![0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00]; // Mock WEBP header
    let result = store.store_snapshot(1000, "auth_failure", &dummy_frame, None, None);

    match result {
        Err(EvidenceStoreError::Disabled) => {
            // Expected
        }
        other => {
            panic!("Expected EvidenceStoreError::Disabled when opt-in is false, got: {other:?}")
        }
    }

    assert!(
        !temp.path().join("evidence").exists(),
        "No evidence directory should be created when disabled"
    );
}

#[test]
fn test_store_snapshot_succeeds_when_explicitly_enabled() {
    let temp = TempDir::new().unwrap();
    let config = EvidenceConfig {
        enabled: true,
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        ..Default::default()
    };

    let store = EvidenceStore::open(config).unwrap();
    assert!(store.config().enabled);

    let dummy_frame = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let res = store.store_snapshot(
        1000,
        "auth_failure",
        &dummy_frame,
        Some("2026-09-14"),
        Some(1726308000),
    );
    assert!(
        res.is_ok(),
        "Snapshot capture should succeed when explicitly enabled"
    );
}

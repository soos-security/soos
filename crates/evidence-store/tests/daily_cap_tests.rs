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

use soos_evidence_store::{EvidenceConfig, EvidenceStore, EvidenceStoreError};
use tempfile::TempDir;

#[test]
fn test_daily_cap_per_uid_enforced() {
    let temp = TempDir::new().unwrap();
    let config = EvidenceConfig {
        enabled: true,
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: 10, // Cap is 10
    };

    let store = EvidenceStore::open(config).unwrap();
    let frame = b"FRAME_DATA";
    let date = "2026-09-14";

    // First 10 captures for UID 1000 must succeed
    for i in 1..=10 {
        let res = store.store_snapshot(1000, "repeated_failure", frame, Some(date), None);
        assert!(
            res.is_ok(),
            "Snapshot capture #{i} within limit of 10 should succeed"
        );
        assert_eq!(store.daily_count(1000, date), i);
    }

    // 11th capture for UID 1000 must be rejected with DailyCapExceeded
    let res = store.store_snapshot(1000, "repeated_failure", frame, Some(date), None);
    match res {
        Err(EvidenceStoreError::DailyCapExceeded {
            uid,
            cap,
            date: err_date,
        }) => {
            assert_eq!(uid, 1000);
            assert_eq!(cap, 10);
            assert_eq!(err_date, date);
        }
        other => {
            panic!("ACCEPTANCE CRITERION E3 VIOLATION: Expected DailyCapExceeded, got {other:?}")
        }
    }

    // Capture count for UID 1000 must remain at 10
    assert_eq!(store.daily_count(1000, date), 10);

    // UID 1001 must have its own independent quota and succeed
    let res_other_user = store.store_snapshot(1001, "failure", frame, Some(date), None);
    assert!(
        res_other_user.is_ok(),
        "Quota for UID 1001 should not be affected by UID 1000"
    );
    assert_eq!(store.daily_count(1001, date), 1);

    // UID 1000 on a different date (e.g. tomorrow) must succeed
    let res_next_day = store.store_snapshot(1000, "failure", frame, Some("2026-09-15"), None);
    assert!(
        res_next_day.is_ok(),
        "Quota on 2026-09-15 should be independent from 2026-09-14"
    );
    assert_eq!(store.daily_count(1000, "2026-09-15"), 1);
}

#[test]
fn test_custom_daily_cap() {
    let temp = TempDir::new().unwrap();
    let config = EvidenceConfig {
        enabled: true,
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: 2, // Custom low cap
    };

    let store = EvidenceStore::open(config).unwrap();
    let date = "2026-09-14";

    assert!(store
        .store_snapshot(1000, "fail", b"1", Some(date), None)
        .is_ok());
    assert!(store
        .store_snapshot(1000, "fail", b"2", Some(date), None)
        .is_ok());

    let res = store.store_snapshot(1000, "fail", b"3", Some(date), None);
    assert!(matches!(
        res,
        Err(EvidenceStoreError::DailyCapExceeded { .. })
    ));
}

//! Contractual tests for GitHub #276 (DMN-03 remainder, walkthrough 95 follow-up): the
//! evidence store enforces a global daily snapshot cap across all UIDs, and its in-memory
//! daily counters are pruned when the day changes, so neither the disk nor the counter map
//! grows with the number of distinct UIDs a caller names.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_evidence_store::{
    EvidenceConfig, EvidenceStore, EvidenceStoreError, DEFAULT_DAILY_CAP_PER_UID,
    DEFAULT_DAILY_CAP_TOTAL,
};
use tempfile::TempDir;

fn store(temp: &TempDir) -> EvidenceStore {
    let config = EvidenceConfig::enabled_with_dir(
        temp.path().join("evidence"),
        temp.path().join("evidence.key"),
    );
    EvidenceStore::open(config).unwrap()
}

#[test]
fn test_default_global_cap_is_bounded_and_above_the_per_uid_cap() {
    const { assert!(DEFAULT_DAILY_CAP_TOTAL > DEFAULT_DAILY_CAP_PER_UID) };
    const { assert!(DEFAULT_DAILY_CAP_TOTAL <= 1000) };
    let temp = TempDir::new().unwrap();
    assert_eq!(store(&temp).daily_cap_total(), DEFAULT_DAILY_CAP_TOTAL);
}

#[test]
fn test_global_daily_cap_spans_all_uids() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp).with_daily_cap_total(3);
    let date = "2026-09-30";

    for uid in 1000..1003 {
        assert!(store
            .store_snapshot(uid, "fail", b"x", Some(date), None)
            .is_ok());
    }
    assert_eq!(store.daily_total(date), 3);

    match store.store_snapshot(1003, "fail", b"x", Some(date), None) {
        Err(EvidenceStoreError::GlobalDailyCapExceeded { cap, date: d }) => {
            assert_eq!(cap, 3);
            assert_eq!(d, date);
        }
        other => panic!("Expected GlobalDailyCapExceeded, got {other:?}"),
    }
    assert_eq!(
        store.daily_total(date),
        3,
        "A refused snapshot is not counted"
    );
    assert_eq!(
        store.daily_count(1003, date),
        0,
        "A globally refused snapshot does not consume the per-UID quota"
    );
    assert_eq!(store.list_snapshots_for_date(date).unwrap().len(), 3);

    // The next day starts a fresh global budget.
    assert!(store
        .store_snapshot(1003, "fail", b"x", Some("2026-10-01"), None)
        .is_ok());
}

#[test]
fn test_default_global_cap_stops_a_many_uid_flood() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);
    let date = "2026-09-30";
    let mut stored = 0u32;
    for uid in 0..(DEFAULT_DAILY_CAP_TOTAL + 50) {
        if store
            .store_snapshot(20_000 + uid, "fail", b"x", Some(date), None)
            .is_ok()
        {
            stored += 1;
        }
    }
    assert_eq!(stored, DEFAULT_DAILY_CAP_TOTAL);
    assert_eq!(
        store.list_snapshots_for_date(date).unwrap().len(),
        DEFAULT_DAILY_CAP_TOTAL as usize
    );
}

#[test]
fn test_daily_counters_are_pruned_when_the_day_changes() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);
    for (day, date) in ["2026-09-26", "2026-09-27", "2026-09-28", "2026-09-29"]
        .iter()
        .enumerate()
    {
        for uid in 0..5u32 {
            let uid = 3000 + uid + u32::try_from(day).unwrap() * 10;
            assert!(store
                .store_snapshot(uid, "fail", b"x", Some(date), None)
                .is_ok());
        }
    }
    assert_eq!(
        store.tracked_daily_counters(),
        5,
        "Only the current day's counters stay in memory"
    );
    assert_eq!(store.daily_count(3000, "2026-09-26"), 0);
    assert_eq!(store.daily_total("2026-09-26"), 0);
    assert_eq!(store.daily_count(3030, "2026-09-29"), 1);
    assert_eq!(store.daily_total("2026-09-29"), 5);
}

#[test]
fn test_tracked_counters_never_exceed_the_global_cap() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp).with_daily_cap_total(4);
    for uid in 0..40u32 {
        let _ = store.store_snapshot(5000 + uid, "fail", b"x", Some("2026-09-30"), None);
    }
    assert!(store.tracked_daily_counters() <= 4);
}

#[test]
fn test_global_cap_error_message_names_no_uid() {
    let err = EvidenceStoreError::GlobalDailyCapExceeded {
        cap: 7,
        date: "2026-09-30".into(),
    };
    let text = err.to_string();
    assert!(text.contains('7') && text.contains("2026-09-30"), "{text}");
}

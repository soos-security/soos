//! Integration contract for the combined evidence daily caps (GitHub #234 persisted per-UID
//! counter and GitHub #276 global cap): the global budget of a day is derived from the
//! snapshot files already stored in its partition, so it survives a daemon restart and a
//! wall clock stepped back across midnight, and a refused snapshot consumes neither quota.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_evidence_store::{EvidenceConfig, EvidenceStore, EvidenceStoreError};
use std::path::Path;
use tempfile::TempDir;

fn open(root: &Path, per_uid: u32, total: u32) -> EvidenceStore {
    let config = EvidenceConfig {
        enabled: true,
        base_dir: root.join("evidence"),
        key_path: root.join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: per_uid,
    };
    EvidenceStore::open(config)
        .unwrap()
        .with_daily_cap_total(total)
}

#[test]
fn test_global_daily_cap_survives_store_restart() {
    let tmp = TempDir::new().unwrap();
    let date = "2026-09-30";
    {
        let first = open(tmp.path(), 5, 3);
        for uid in 1000..1003 {
            first
                .store_snapshot(uid, "fail", b"x", Some(date), None)
                .unwrap();
        }
    }

    let second = open(tmp.path(), 5, 3);
    assert_eq!(
        second.daily_total(date),
        3,
        "persisted snapshots are counted"
    );
    let res = second.store_snapshot(1003, "fail", b"x", Some(date), None);
    assert!(
        matches!(
            res,
            Err(EvidenceStoreError::GlobalDailyCapExceeded { cap: 3, .. })
        ),
        "the global cap must hold across a restart, got {res:?}"
    );
    assert_eq!(second.daily_count(1003, date), 0);
    assert_eq!(second.list_snapshots_for_date(date).unwrap().len(), 3);
}

#[test]
fn test_clock_stepped_back_does_not_reopen_a_stored_day() {
    let tmp = TempDir::new().unwrap();
    let store = open(tmp.path(), 1, 2);
    let day1 = "2026-09-29";
    let day2 = "2026-09-30";
    store
        .store_snapshot(1000, "fail", b"a", Some(day1), None)
        .unwrap();
    store
        .store_snapshot(1001, "fail", b"b", Some(day1), None)
        .unwrap();
    store
        .store_snapshot(1000, "fail", b"c", Some(day2), None)
        .unwrap();

    // Back to day 1: both the per-UID counter and the global total come from disk.
    let per_uid = store.store_snapshot(1000, "fail", b"d", Some(day1), None);
    assert!(
        matches!(
            per_uid,
            Err(EvidenceStoreError::DailyCapExceeded { uid: 1000, .. })
        ),
        "got {per_uid:?}"
    );
    let global = store.store_snapshot(1002, "fail", b"e", Some(day1), None);
    assert!(
        matches!(
            global,
            Err(EvidenceStoreError::GlobalDailyCapExceeded { cap: 2, .. })
        ),
        "got {global:?}"
    );
    assert_eq!(store.list_snapshots_for_date(day1).unwrap().len(), 2);
    assert_eq!(store.daily_total(day1), 2);
}

#[test]
fn test_per_uid_refusal_does_not_consume_the_global_quota() {
    let tmp = TempDir::new().unwrap();
    let store = open(tmp.path(), 1, 2);
    let date = "2026-09-30";
    store
        .store_snapshot(1000, "fail", b"a", Some(date), None)
        .unwrap();
    let refused = store.store_snapshot(1000, "fail", b"b", Some(date), None);
    assert!(matches!(
        refused,
        Err(EvidenceStoreError::DailyCapExceeded { .. })
    ));
    assert_eq!(store.daily_total(date), 1);
    store
        .store_snapshot(1001, "fail", b"c", Some(date), None)
        .expect("the second global slot is still available");
    assert_eq!(store.daily_total(date), 2);
}

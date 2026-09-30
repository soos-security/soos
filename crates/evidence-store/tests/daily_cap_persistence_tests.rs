//! Contract tests for GitHub #234 (review finding STO-18): the per-UID daily evidence cap is
//! persisted on disk next to the snapshots (so it survives a daemon restart), is removed with
//! the date directory by retention (so nothing grows for the daemon lifetime), and a slot is
//! consumed only once the snapshot is durably renamed into place.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and expect"
)]

use soos_evidence_store::{
    EvidenceConfig, EvidenceStore, EvidenceStoreError, DAILY_COUNT_FILE_PREFIX,
};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::TempDir;

fn config(root: &Path, cap: u32) -> EvidenceConfig {
    EvidenceConfig {
        enabled: true,
        base_dir: root.join("evidence"),
        key_path: root.join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: cap,
    }
}

fn is_root() -> bool {
    nix::unistd::geteuid().is_root()
}

#[test]
fn test_234_daily_cap_survives_store_restart() {
    let tmp = TempDir::new().unwrap();
    let date = "2026-09-14";
    {
        let first = EvidenceStore::open(config(tmp.path(), 2)).unwrap();
        first
            .store_snapshot(1000, "fail", b"a", Some(date), None)
            .unwrap();
        first
            .store_snapshot(1000, "fail", b"b", Some(date), None)
            .unwrap();
    }

    // A new store on the same directory (daemon restart) must still see the consumed quota.
    let second = EvidenceStore::open(config(tmp.path(), 2)).unwrap();
    assert_eq!(second.daily_count(1000, date), 2);
    let res = second.store_snapshot(1000, "fail", b"c", Some(date), None);
    assert!(
        matches!(
            res,
            Err(EvidenceStoreError::DailyCapExceeded {
                uid: 1000,
                cap: 2,
                ..
            })
        ),
        "the cap must hold across a restart, got {res:?}"
    );
    assert_eq!(second.list_snapshots_for_date(date).unwrap().len(), 2);

    // Another UID keeps its own independent quota after the restart.
    second
        .store_snapshot(1001, "fail", b"d", Some(date), None)
        .unwrap();
    assert_eq!(second.daily_count(1001, date), 1);
}

#[test]
fn test_234_counter_files_are_not_listed_as_snapshots_and_are_private() {
    let tmp = TempDir::new().unwrap();
    let store = EvidenceStore::open(config(tmp.path(), 5)).unwrap();
    let date = "2026-09-14";
    store
        .store_snapshot(1000, "fail", b"a", Some(date), None)
        .unwrap();
    store
        .store_snapshot(1001, "fail", b"b", Some(date), None)
        .unwrap();

    assert_eq!(store.list_snapshots_for_date(date).unwrap().len(), 2);
    let counter = tmp
        .path()
        .join("evidence")
        .join(date)
        .join(format!("{DAILY_COUNT_FILE_PREFIX}1000"));
    let meta = std::fs::symlink_metadata(&counter).expect("counter file exists");
    assert!(meta.file_type().is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
}

#[test]
fn test_234_retention_forgets_counts_of_pruned_dates() {
    let tmp = TempDir::new().unwrap();
    let store = EvidenceStore::open(config(tmp.path(), 1)).unwrap();
    store
        .store_snapshot(1000, "fail", b"old", Some("2026-09-21"), None)
        .unwrap();
    assert_eq!(store.daily_count(1000, "2026-09-21"), 1);

    let report = store.rotate_retention("2026-09-30").unwrap();
    assert_eq!(report.pruned_dates, vec!["2026-09-21".to_string()]);
    assert_eq!(
        store.daily_count(1000, "2026-09-21"),
        0,
        "a pruned date must not keep a count in memory"
    );
}

#[test]
fn test_234_failed_write_does_not_consume_a_slot() {
    if is_root() {
        // Root bypasses directory write permissions, so the failure cannot be provoked.
        return;
    }
    let tmp = TempDir::new().unwrap();
    let store = EvidenceStore::open(config(tmp.path(), 1)).unwrap();
    let date = "2026-09-14";
    let date_dir = tmp.path().join("evidence").join(date);
    std::fs::create_dir_all(&date_dir).unwrap();
    std::fs::set_permissions(&date_dir, std::fs::Permissions::from_mode(0o500)).unwrap();

    let failed = store.store_snapshot(1000, "fail", b"a", Some(date), None);
    std::fs::set_permissions(&date_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        matches!(failed, Err(EvidenceStoreError::Io(_))),
        "a write into a read-only partition must fail with Io, got {failed:?}"
    );
    assert_eq!(
        store.daily_count(1000, date),
        0,
        "a failed write consumed a slot"
    );

    store
        .store_snapshot(1000, "fail", b"b", Some(date), None)
        .expect("the only slot of the day is still available");
    assert_eq!(store.daily_count(1000, date), 1);
}

#[test]
fn test_234_symlinked_counter_file_is_refused_and_target_untouched() {
    let tmp = TempDir::new().unwrap();
    let store = EvidenceStore::open(config(tmp.path(), 5)).unwrap();
    let date = "2026-09-14";
    let date_dir = tmp.path().join("evidence").join(date);
    std::fs::create_dir_all(&date_dir).unwrap();
    std::fs::set_permissions(&date_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let decoy = tmp.path().join("decoy");
    std::fs::write(&decoy, b"0").unwrap();
    std::os::unix::fs::symlink(
        &decoy,
        date_dir.join(format!("{DAILY_COUNT_FILE_PREFIX}1000")),
    )
    .unwrap();

    let res = store.store_snapshot(1000, "fail", b"a", Some(date), None);
    assert!(
        res.is_err(),
        "a symlinked counter must be refused, got {res:?}"
    );
    assert_eq!(std::fs::read(&decoy).unwrap(), b"0");
    assert!(store.list_snapshots_for_date(date).unwrap().is_empty());
}

#[test]
fn test_234_corrupt_counter_file_fails_closed() {
    let tmp = TempDir::new().unwrap();
    let store = EvidenceStore::open(config(tmp.path(), 5)).unwrap();
    let date = "2026-09-14";
    let date_dir = tmp.path().join("evidence").join(date);
    std::fs::create_dir_all(&date_dir).unwrap();
    std::fs::write(
        date_dir.join(format!("{DAILY_COUNT_FILE_PREFIX}1000")),
        b"not-a-number",
    )
    .unwrap();

    let res = store.store_snapshot(1000, "fail", b"a", Some(date), None);
    assert!(
        matches!(res, Err(EvidenceStoreError::CorruptPayload(_))),
        "an unreadable counter must fail closed, got {res:?}"
    );
    assert!(store.list_snapshots_for_date(date).unwrap().is_empty());
}

#[test]
fn test_234_concurrent_writers_never_exceed_the_cap() {
    let tmp = TempDir::new().unwrap();
    let store = std::sync::Arc::new(EvidenceStore::open(config(tmp.path(), 3)).unwrap());
    let date = "2026-09-14";
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .store_snapshot(1000, "fail", b"x", Some(date), None)
                    .is_ok()
            })
        })
        .collect();
    let ok = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|ok| *ok)
        .count();
    assert_eq!(ok, 3);
    assert_eq!(store.list_snapshots_for_date(date).unwrap().len(), 3);
    assert_eq!(store.daily_count(1000, date), 3);
}

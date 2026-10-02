//! Contract tests for GitHub #310 / #312 (review findings DMN-NEW-1, STO-NEW-10): the
//! exclusive base-directory `flock` taken by `rotate_retention` and
//! `migrate_legacy_snapshots` is waited for with a bound. A contended lock is polled until
//! the store's lock timeout and then reported as `EvidenceStoreError::LockTimeout`, never
//! waited for indefinitely, so a blocking-pool evidence write cannot be pinned by a running
//! migration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and expect"
)]

use nix::fcntl::{Flock, FlockArg};
use soos_evidence_store::{
    EvidenceConfig, EvidenceStore, EvidenceStoreError, MasterKey, EVIDENCE_LOCK_TIMEOUT,
};
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Lock timeout configured on the stores under test.
const TEST_LOCK_TIMEOUT: Duration = Duration::from_millis(200);

/// Upper bound after which a call is considered blocked on the lock.
const WATCHDOG: Duration = Duration::from_secs(5);

fn store(base: &Path, key_path: &Path) -> EvidenceStore {
    EvidenceStore::new(
        EvidenceConfig {
            enabled: true,
            base_dir: base.to_path_buf(),
            retention_days: 7,
            daily_cap_per_uid: 5,
            key_path: key_path.to_path_buf(),
        },
        MasterKey::generate().expect("key"),
    )
    .with_lock_timeout(TEST_LOCK_TIMEOUT)
}

fn hold_base_lock(base: &Path) -> Flock<std::fs::File> {
    let dir = std::fs::File::open(base).expect("open base dir");
    Flock::lock(dir, FlockArg::LockExclusiveNonblock).expect("hold evidence lock")
}

/// Runs `op` on a helper thread and returns its result, or `None` when it did not return
/// within [`WATCHDOG`] (blocked on the lock).
fn run_bounded<T: Send + 'static>(
    op: impl FnOnce() -> T + Send + 'static,
) -> Option<(T, Duration)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let started = Instant::now();
        let out = op();
        let _ = tx.send((out, started.elapsed()));
    });
    rx.recv_timeout(WATCHDOG).ok()
}

#[test]
fn test_310_default_evidence_lock_timeout_is_bounded() {
    assert!(EVIDENCE_LOCK_TIMEOUT > Duration::ZERO);
    assert!(EVIDENCE_LOCK_TIMEOUT <= Duration::from_secs(5));
}

#[test]
fn test_310_rotate_retention_times_out_on_a_held_lock() {
    let tmp = TempDir::new().expect("tempdir");
    let base = tmp.path().join("evidence");
    std::fs::create_dir(&base).expect("base dir");
    let ev = Arc::new(store(&base, &tmp.path().join("evidence.key")));
    let _held = hold_base_lock(&base);

    let worker = Arc::clone(&ev);
    let (res, elapsed) = run_bounded(move || worker.rotate_retention("2026-10-02"))
        .expect("rotate_retention must not wait indefinitely for the evidence lock");
    assert!(
        matches!(res, Err(EvidenceStoreError::LockTimeout(_))),
        "expected LockTimeout, got {res:?}"
    );
    assert!(
        elapsed >= TEST_LOCK_TIMEOUT,
        "gave up before the lock timeout"
    );
}

#[test]
fn test_312_migrate_legacy_snapshots_times_out_on_a_held_lock() {
    let tmp = TempDir::new().expect("tempdir");
    let base = tmp.path().join("evidence");
    std::fs::create_dir(&base).expect("base dir");
    let ev = Arc::new(store(&base, &tmp.path().join("evidence.key")));
    let _held = hold_base_lock(&base);

    for dry_run in [true, false] {
        let worker = Arc::clone(&ev);
        let (res, _) = run_bounded(move || worker.migrate_legacy_snapshots(dry_run))
            .expect("migrate_legacy_snapshots must not wait indefinitely for the evidence lock");
        assert!(
            matches!(res, Err(EvidenceStoreError::LockTimeout(_))),
            "dry_run={dry_run}: expected LockTimeout, got {res:?}"
        );
    }
}

#[test]
fn test_310_rotate_retention_succeeds_once_the_lock_is_released() {
    let tmp = TempDir::new().expect("tempdir");
    let base = tmp.path().join("evidence");
    std::fs::create_dir(&base).expect("base dir");
    std::fs::create_dir(base.join("2020-01-01")).expect("old partition");
    let ev = Arc::new(store(&base, &tmp.path().join("evidence.key")).with_lock_timeout(WATCHDOG));
    let held = hold_base_lock(&base);

    let worker = Arc::clone(&ev);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(worker.rotate_retention("2026-10-02"));
    });
    std::thread::sleep(Duration::from_millis(50));
    drop(held);
    let report = rx
        .recv_timeout(WATCHDOG)
        .expect("rotation returns after the lock is released")
        .expect("rotation succeeds");
    assert_eq!(report.pruned_dates, vec!["2020-01-01".to_string()]);
}

#[test]
fn test_310_rotate_retention_refuses_a_symlinked_base_directory() {
    let tmp = TempDir::new().expect("tempdir");
    let real = tmp.path().join("real");
    std::fs::create_dir(&real).expect("real dir");
    let link = tmp.path().join("evidence");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    let ev = store(&link, &tmp.path().join("evidence.key"));
    assert!(matches!(
        ev.rotate_retention("2026-10-02"),
        Err(EvidenceStoreError::InvalidPath(_))
    ));
}

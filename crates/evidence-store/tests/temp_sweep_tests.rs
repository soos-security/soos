//! GitHub #291 (SGU5): `EvidenceStore::sweep_orphaned_temp_files` removes the temporary files a
//! blocking evidence write abandoned at daemon shutdown left in a date partition, and nothing
//! else.
//!
//! Candidates are only the exact names the store creates inside a `YYYY-MM-DD` partition:
//! `.tmp.<uuid>.<pid>.<16 hex>` (snapshot), `.tmp.migrate.<uuid>.<pid>.<16 hex>` (migration)
//! and `.tmp.daily_count.<uid>.<pid>.<16 hex>` (daily counter). Only regular, single-link files
//! owned by root or the effective UID and older than `TEMP_SWEEP_MIN_AGE` are removed, at most
//! `MAX_TEMP_SWEEP_REMOVALS` per call. Every decoy survives.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and slicing"
)]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use nix::fcntl::{Flock, FlockArg};
use soos_evidence_store::store::{MAX_TEMP_SWEEP_REMOVALS, TEMP_SWEEP_MIN_AGE};
use soos_evidence_store::{
    EvidenceConfig, EvidenceStore, EvidenceStoreError, MasterKey, TempSweepReport,
};
use tempfile::TempDir;

const UUID: &str = "0f8c2a4e-1b3d-4c5e-8f60-718293a4b5c6";
const DATE: &str = "2026-09-30";

fn store(temp: &TempDir, enabled: bool) -> (EvidenceStore, PathBuf) {
    let base = temp.path().join("evidence");
    let mut config = EvidenceConfig::enabled_with_dir(base.clone(), temp.path().join("ev.key"));
    config.enabled = enabled;
    (
        EvidenceStore::new(config, MasterKey::generate().unwrap()),
        base,
    )
}

fn partition(base: &Path, date: &str) -> PathBuf {
    let dir = base.join(date);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn set_age(path: &Path, age: Duration) {
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - age)
        .unwrap();
}

fn old() -> Duration {
    TEMP_SWEEP_MIN_AGE + Duration::from_secs(60)
}

fn plant(dir: &Path, name: &str, age: Duration) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"orphaned ciphertext").unwrap();
    set_age(&path, age);
    path
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn test_sgu_evidence_sweep_removes_the_three_orphaned_temp_kinds() {
    let temp = TempDir::new().unwrap();
    let (store, base) = store(&temp, true);
    let day = partition(&base, DATE);
    plant(&day, &format!(".tmp.{UUID}.4242.00ff00ff00ff00ff"), old());
    plant(
        &day,
        &format!(".tmp.migrate.{UUID}.1.0123456789abcdef"),
        old(),
    );
    plant(&day, ".tmp.daily_count.1000.77.fedcba9876543210", old());
    let other = partition(&base, "2026-10-01");
    plant(&other, &format!(".tmp.{UUID}.4242.0000000000000000"), old());

    let report = store.sweep_orphaned_temp_files().unwrap();

    assert_eq!(
        report,
        TempSweepReport {
            removed: 4,
            ..TempSweepReport::default()
        }
    );
    assert!(names(&day).is_empty(), "left: {:?}", names(&day));
    assert!(names(&other).is_empty(), "left: {:?}", names(&other));
}

#[test]
fn test_sgu_evidence_sweep_keeps_every_decoy_snapshots_and_counters() {
    let temp = TempDir::new().unwrap();
    let (store, _base) = store(&temp, true);
    let written = store
        .store_snapshot(1000, "spoof", b"opaque", Some(DATE), Some(1_759_190_400))
        .unwrap();
    let day = written.path.parent().unwrap().to_path_buf();
    for name in names(&day) {
        set_age(&day.join(name), old());
    }
    let kept_before = names(&day);

    let decoys = [
        format!(".tmp.{UUID}.4242"),
        format!(".tmp.{UUID}.4242.00ff00ff00ff00ff.bak"),
        format!(".tmp.{UUID}.4242.00FF00FF00FF00FF"),
        format!(".tmp.{UUID}.4242.00ff00ff00ff00f"),
        format!(".tmp.{UUID}.04242.00ff00ff00ff00ff"),
        format!(".tmp.{UUID}.4294967296.00ff00ff00ff00ff"),
        format!(".tmp.{}.1.00ff00ff00ff00ff", UUID.to_uppercase()),
        ".tmp.not-a-uuid.1.00ff00ff00ff00ff".to_string(),
        format!(".tmp.{}.1.00ff00ff00ff00ff", UUID.replace('-', "")),
        format!("tmp.{UUID}.1.00ff00ff00ff00ff"),
        format!("{UUID}.tmp.1.00ff00ff00ff00ff"),
        format!(".tmp.migrate.{UUID}.1"),
        ".tmp.daily_count.2147483648.1.00ff00ff00ff00ff".to_string(),
        ".tmp.daily_count.01000.1.00ff00ff00ff00ff".to_string(),
        ".tmp.daily_count..1.00ff00ff00ff00ff".to_string(),
        ".daily_count.1000.tmp".to_string(),
        "1000.tmp.1.2".to_string(),
        "notes.txt".to_string(),
    ];
    for name in &decoys {
        plant(&day, name, old());
    }

    let report = store.sweep_orphaned_temp_files().unwrap();

    assert_eq!(report.removed, 0, "{report:?}");
    let mut expected = kept_before.clone();
    expected.extend(decoys.iter().cloned());
    expected.sort();
    assert_eq!(names(&day), expected);
    assert!(
        store.load_snapshot(&written.path).is_ok(),
        "snapshot intact"
    );
    assert_eq!(store.daily_count(1000, DATE), 1, "counter intact");
}

#[test]
fn test_sgu_evidence_sweep_only_looks_inside_date_partitions() {
    let temp = TempDir::new().unwrap();
    let (store, base) = store(&temp, true);
    std::fs::create_dir_all(&base).unwrap();
    let name = format!(".tmp.{UUID}.4242.00ff00ff00ff00ff");
    // At the base directory level and in a non-date directory: never candidates.
    plant(&base, &name, old());
    let not_a_date = partition(&base, "lost+found");
    plant(&not_a_date, &name, old());
    let bad_date = partition(&base, "2026-13-01");
    plant(&bad_date, &name, old());
    // A symlinked partition is never followed.
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    plant(&outside, &name, old());
    std::os::unix::fs::symlink(&outside, base.join("2026-09-29")).unwrap();

    let report = store.sweep_orphaned_temp_files().unwrap();

    assert_eq!(report.removed, 0, "{report:?}");
    let mut expected_base = vec![
        name.clone(),
        "2026-09-29".to_string(),
        "2026-13-01".to_string(),
        "lost+found".to_string(),
    ];
    expected_base.sort();
    assert_eq!(names(&base), expected_base);
    assert_eq!(names(&not_a_date), vec![name.clone()]);
    assert_eq!(names(&bad_date), vec![name.clone()]);
    assert_eq!(names(&outside), vec![name]);
}

#[test]
fn test_sgu_evidence_sweep_keeps_recent_non_regular_and_linked_entries() {
    let temp = TempDir::new().unwrap();
    let (store, base) = store(&temp, true);
    let day = partition(&base, DATE);
    plant(
        &day,
        &format!(".tmp.{UUID}.1.0000000000000001"),
        Duration::from_secs(1),
    );
    let outside = temp.path().join("outside-target");
    std::fs::write(&outside, b"precious").unwrap();
    set_age(&outside, old());
    std::os::unix::fs::symlink(
        &outside,
        day.join(format!(".tmp.{UUID}.1.0000000000000002")),
    )
    .unwrap();
    std::fs::create_dir(day.join(format!(".tmp.{UUID}.1.0000000000000003"))).unwrap();
    let linked = temp.path().join("linked-elsewhere");
    std::fs::write(&linked, b"other").unwrap();
    set_age(&linked, old());
    std::fs::hard_link(&linked, day.join(format!(".tmp.{UUID}.1.0000000000000004"))).unwrap();

    let report = store.sweep_orphaned_temp_files().unwrap();

    assert_eq!(report.removed, 0, "{report:?}");
    assert_eq!(report.kept_recent, 1, "{report:?}");
    assert_eq!(names(&day).len(), 4);
    assert_eq!(std::fs::read(&outside).unwrap(), b"precious");
    assert_eq!(std::fs::read(&linked).unwrap(), b"other");
}

#[test]
fn test_sgu_evidence_sweep_is_capped_per_call() {
    let temp = TempDir::new().unwrap();
    let (store, base) = store(&temp, true);
    let day = partition(&base, DATE);
    let extra = 3;
    for i in 0..(MAX_TEMP_SWEEP_REMOVALS + extra) {
        plant(
            &day,
            &format!(".tmp.daily_count.{i}.9.00000000000000aa"),
            old(),
        );
    }

    let first = store.sweep_orphaned_temp_files().unwrap();
    assert_eq!(first.removed, MAX_TEMP_SWEEP_REMOVALS, "{first:?}");
    assert!(first.limit_reached, "{first:?}");
    let second = store.sweep_orphaned_temp_files().unwrap();
    assert_eq!(second.removed, extra, "{second:?}");
    assert!(names(&day).is_empty());
}

#[test]
fn test_sgu_evidence_sweep_touches_nothing_while_the_store_is_locked() {
    let temp = TempDir::new().unwrap();
    let (store, base) = store(&temp, true);
    let day = partition(&base, DATE);
    let name = format!(".tmp.{UUID}.4242.00ff00ff00ff00ff");
    plant(&day, &name, old());

    // The exclusive base-directory lock of retention and migration.
    let lock = Flock::lock(File::open(&base).unwrap(), FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    let report = store.sweep_orphaned_temp_files().unwrap();
    assert!(report.lock_busy, "{report:?}");
    assert_eq!(names(&day), vec![name]);
    drop(lock);
    assert_eq!(store.sweep_orphaned_temp_files().unwrap().removed, 1);
}

#[test]
fn test_sgu_evidence_sweep_of_a_disabled_or_missing_store_touches_nothing() {
    let temp = TempDir::new().unwrap();
    let (disabled, base) = store(&temp, false);
    let day = partition(&base, DATE);
    let name = format!(".tmp.{UUID}.4242.00ff00ff00ff00ff");
    plant(&day, &name, old());
    assert_eq!(
        disabled.sweep_orphaned_temp_files().unwrap(),
        TempSweepReport::default()
    );
    assert_eq!(names(&day), vec![name]);

    let missing = TempDir::new().unwrap();
    let (enabled, missing_base) = store(&missing, true);
    assert_eq!(
        enabled.sweep_orphaned_temp_files().unwrap(),
        TempSweepReport::default()
    );
    assert!(!missing_base.exists(), "nothing is created");
}

#[test]
fn test_sgu_evidence_sweep_refuses_a_symlinked_base_directory() {
    let temp = TempDir::new().unwrap();
    let real = temp.path().join("real");
    let day = partition(&real, DATE);
    let name = format!(".tmp.{UUID}.4242.00ff00ff00ff00ff");
    plant(&day, &name, old());
    let (store, base) = store(&temp, true);
    std::os::unix::fs::symlink(&real, &base).unwrap();

    let result = store.sweep_orphaned_temp_files();
    assert!(
        matches!(result, Err(EvidenceStoreError::InvalidPath(_))),
        "{result:?}"
    );
    assert_eq!(names(&day), vec![name]);
}

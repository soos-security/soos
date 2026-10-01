//! GitHub #291 (SGU4): `BiometricStore::sweep_orphaned_temp_files` removes the temporary files
//! an interrupted template write left behind, and nothing else.
//!
//! Only names of the exact form the store creates (`<uid>.tmp.<pid>.<u64>`, canonical decimal
//! numbers) are candidates; only regular, single-link files owned by root or the effective UID
//! and older than `TEMP_SWEEP_MIN_AGE` are removed; at most `MAX_TEMP_SWEEP_REMOVALS` per call;
//! nothing is touched while another operation holds the store lock. Every decoy survives.

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
use soos_biometric_store::{
    BiometricStore, BiometricTemplate, MasterKey, TempSweepReport, MAX_TEMP_SWEEP_REMOVALS,
    TEMP_SWEEP_MIN_AGE,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

fn open(temp: &TempDir) -> (BiometricStore, PathBuf) {
    let dir = temp.path().join("bio");
    let store = BiometricStore::new(&dir, MasterKey::generate().unwrap()).unwrap();
    (store, dir)
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

/// A regular file at `dir/name` whose modification time is `age` in the past.
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
fn test_sgu_sweep_removes_old_orphaned_template_temp_files() {
    let temp = TempDir::new().unwrap();
    let (store, dir) = open(&temp);
    plant(&dir, "1000.tmp.4242.17", old());
    plant(&dir, "0.tmp.1.2", old());
    plant(
        &dir,
        "4294967295.tmp.4294967295.18446744073709551615",
        old(),
    );

    let report = store.sweep_orphaned_temp_files().unwrap();

    assert_eq!(
        report,
        TempSweepReport {
            removed: 3,
            ..TempSweepReport::default()
        }
    );
    assert!(names(&dir).is_empty(), "left: {:?}", names(&dir));
}

#[test]
fn test_sgu_sweep_keeps_every_decoy_and_the_templates() {
    let temp = TempDir::new().unwrap();
    let (store, dir) = open(&temp);
    store
        .enroll(
            &BiometricTemplate::new(
                1000,
                "arcface_w600k_mbf".to_string(),
                "2.0.0".to_string(),
                1,
                Zeroizing::new(vec![0.5; 512]),
            )
            .unwrap(),
        )
        .unwrap();
    set_age(&dir.join("1000.cbor.enc"), old());

    let decoys = [
        "1000.tmp.123",
        "1000.tmp.123.456.bak",
        "1000.tmp.123.456.",
        "abc.tmp.1.2",
        "1000.tmp.1.0x10",
        "01000.tmp.1.2",
        "1000.tmp.01.2",
        "1000.tmp.1.02",
        "+1000.tmp.1.2",
        "1000.TMP.1.2",
        ".1000.tmp.1.2",
        "1000.tmp.-1.2",
        "1000.tmp.1.18446744073709551616",
        "4294967296.tmp.1.2",
        "1000.tmp.4294967296.2",
        "master.key.tmp.1.2",
        ".tmp.00000000-0000-4000-8000-000000000000.1.0000000000000000",
        "1000.tmp..2",
        "notes.txt",
    ];
    for name in decoys {
        plant(&dir, name, old());
    }

    let report = store.sweep_orphaned_temp_files().unwrap();

    assert_eq!(report.removed, 0, "{report:?}");
    let mut expected: Vec<String> = decoys.iter().map(|s| (*s).to_string()).collect();
    expected.push("1000.cbor.enc".to_string());
    expected.sort();
    assert_eq!(names(&dir), expected);
    assert!(store.get(1000).unwrap().is_some(), "the template is intact");
}

#[test]
fn test_sgu_sweep_keeps_recent_and_future_dated_temp_files() {
    let temp = TempDir::new().unwrap();
    let (store, dir) = open(&temp);
    // A write that may still be in progress (younger than the minimum age).
    plant(&dir, "1000.tmp.10.1", Duration::from_secs(1));
    plant(
        &dir,
        "1000.tmp.10.2",
        TEMP_SWEEP_MIN_AGE.saturating_sub(Duration::from_secs(10)),
    );
    // A modification time in the future (clock step) is never treated as old.
    let future = dir.join("1000.tmp.10.3");
    std::fs::write(&future, b"x").unwrap();
    File::options()
        .write(true)
        .open(&future)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();

    let report = store.sweep_orphaned_temp_files().unwrap();

    assert_eq!(report.removed, 0, "{report:?}");
    assert_eq!(report.kept_recent, 3, "{report:?}");
    assert_eq!(names(&dir).len(), 3);
}

#[test]
fn test_sgu_sweep_never_follows_or_removes_non_regular_entries() {
    let temp = TempDir::new().unwrap();
    let (store, dir) = open(&temp);

    // A symlink with a matching name pointing outside the store: neither the link nor its
    // target is removed.
    let outside = temp.path().join("outside-target");
    std::fs::write(&outside, b"precious").unwrap();
    set_age(&outside, old());
    std::os::unix::fs::symlink(&outside, dir.join("1000.tmp.1.5")).unwrap();

    // A directory and a FIFO with matching names.
    std::fs::create_dir(dir.join("1000.tmp.1.6")).unwrap();
    std::fs::write(dir.join("1000.tmp.1.6").join("inner"), b"x").unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(dir.join("1000.tmp.1.7"))
        .status()
        .unwrap();
    assert!(status.success());

    // A hard link with a matching name: removing it would drop a name of another file.
    let linked = temp.path().join("linked-elsewhere");
    std::fs::write(&linked, b"other file").unwrap();
    set_age(&linked, old());
    std::fs::hard_link(&linked, dir.join("1000.tmp.1.8")).unwrap();

    let report = store.sweep_orphaned_temp_files().unwrap();

    assert_eq!(report.removed, 0, "{report:?}");
    assert_eq!(std::fs::read(&outside).unwrap(), b"precious");
    assert_eq!(
        names(&dir),
        vec![
            "1000.tmp.1.5".to_string(),
            "1000.tmp.1.6".to_string(),
            "1000.tmp.1.7".to_string(),
            "1000.tmp.1.8".to_string(),
        ]
    );
    assert!(dir.join("1000.tmp.1.6").join("inner").exists());
    assert_eq!(std::fs::read(&linked).unwrap(), b"other file");
}

#[test]
fn test_sgu_sweep_is_capped_per_call() {
    let temp = TempDir::new().unwrap();
    let (store, dir) = open(&temp);
    let extra = 5;
    for i in 0..(MAX_TEMP_SWEEP_REMOVALS + extra) {
        plant(&dir, &format!("1000.tmp.7.{i}"), old());
    }

    let first = store.sweep_orphaned_temp_files().unwrap();
    assert_eq!(first.removed, MAX_TEMP_SWEEP_REMOVALS, "{first:?}");
    assert!(first.limit_reached, "{first:?}");
    assert_eq!(names(&dir).len(), extra);

    let second = store.sweep_orphaned_temp_files().unwrap();
    assert_eq!(second.removed, extra, "{second:?}");
    assert!(!second.limit_reached, "{second:?}");
    assert!(names(&dir).is_empty());
}

#[test]
fn test_sgu_sweep_touches_nothing_while_the_store_lock_is_held() {
    let temp = TempDir::new().unwrap();
    let (store, dir) = open(&temp);
    plant(&dir, "1000.tmp.4242.17", old());

    let lock = Flock::lock(File::open(&dir).unwrap(), FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    let started = std::time::Instant::now();
    let report = store.sweep_orphaned_temp_files().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the sweep never waits for the lock"
    );
    assert!(report.lock_busy, "{report:?}");
    assert_eq!(report.removed, 0, "{report:?}");
    assert_eq!(names(&dir), vec!["1000.tmp.4242.17".to_string()]);

    drop(lock);
    assert_eq!(store.sweep_orphaned_temp_files().unwrap().removed, 1);
}

#[test]
fn test_sgu_sweep_minimum_age_and_cap_are_bounded() {
    assert!(TEMP_SWEEP_MIN_AGE >= Duration::from_secs(60));
    assert!((1..=1024).contains(&MAX_TEMP_SWEEP_REMOVALS));
}

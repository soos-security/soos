//! GitHub #291 (SGU3): `BiometricStore::enroll_if_absent` checks that no template exists and
//! writes the new one under the same exclusive store lock, so `soos-enroll import` without
//! `--yes` can never replace a template enrolled after its first duplicate check.
//!
//! The lock is held from the test through `nix::fcntl::Flock` on the store directory (the
//! documented on-disk protocol), so the "concurrent enroll lands first" interleaving is
//! deterministic.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and slicing"
)]

use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use nix::fcntl::{Flock, FlockArg};
use soos_biometric_store::{BiometricStore, BiometricStoreError, BiometricTemplate, MasterKey};
use tempfile::TempDir;
use zeroize::Zeroizing;

const UID: u32 = 1000;

fn template(value: f32, timestamp: u64) -> BiometricTemplate {
    BiometricTemplate::new(
        UID,
        "arcface_w600k_mbf".to_string(),
        "2.0.0".to_string(),
        timestamp,
        Zeroizing::new(vec![value; 512]),
    )
    .unwrap()
}

fn open(dir: &Path, key: &MasterKey) -> BiometricStore {
    BiometricStore::new(dir, key.clone()).unwrap()
}

fn hold_lock(dir: &Path) -> Flock<File> {
    Flock::lock(File::open(dir).unwrap(), FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .expect("the test takes the store lock first")
}

fn dir_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn test_sgu_enroll_if_absent_writes_a_missing_template() {
    let temp = TempDir::new().unwrap();
    let key = MasterKey::generate().unwrap();
    let dir = temp.path().join("bio");
    let store = open(&dir, &key);

    store.enroll_if_absent(&template(0.25, 1)).unwrap();

    let kept = store.get(UID).unwrap().unwrap();
    assert_eq!(kept.enrollment_timestamp, 1);
    assert_eq!(dir_entries(&dir), vec!["1000.cbor.enc".to_string()]);
}

#[test]
fn test_sgu_enroll_if_absent_refuses_an_enrolled_uid_and_keeps_it_byte_for_byte() {
    let temp = TempDir::new().unwrap();
    let key = MasterKey::generate().unwrap();
    let dir = temp.path().join("bio");
    let store = open(&dir, &key);
    store.enroll(&template(0.25, 1)).unwrap();
    let before = std::fs::read(store.template_path(UID).unwrap()).unwrap();

    let result = store.enroll_if_absent(&template(0.75, 2));

    assert!(
        matches!(result, Err(BiometricStoreError::AlreadyEnrolled(UID))),
        "expected AlreadyEnrolled({UID}), got {result:?}"
    );
    assert_eq!(
        std::fs::read(store.template_path(UID).unwrap()).unwrap(),
        before,
        "the enrolled template is never touched"
    );
    assert_eq!(dir_entries(&dir), vec!["1000.cbor.enc".to_string()]);
}

/// The check runs under the lock: a template that appears while `enroll_if_absent` waits for
/// the lock (an enrollment that won the lock first) is detected, never replaced.
#[test]
fn test_sgu_enroll_if_absent_checks_under_the_lock_against_a_concurrent_enroll() {
    let temp = TempDir::new().unwrap();
    let key = MasterKey::generate().unwrap();
    let dir = temp.path().join("bio");
    let store = open(&dir, &key);

    // The winning enrollment's ciphertext, produced by a second store with the same key.
    let staging = open(&temp.path().join("staging"), &key);
    staging.enroll(&template(0.75, 2)).unwrap();
    let winner = std::fs::read(staging.template_path(UID).unwrap()).unwrap();

    let lock = hold_lock(&dir);
    let importer = {
        let store = open(&dir, &key);
        std::thread::spawn(move || store.enroll_if_absent(&template(0.25, 1)))
    };
    // Let the importer reach the store and block on the lock, then land the winner.
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !store.exists(UID).unwrap(),
        "nothing is written while the lock is held"
    );
    std::fs::write(store.template_path(UID).unwrap(), &winner).unwrap();
    drop(lock);

    let result = importer.join().unwrap();
    assert!(
        matches!(result, Err(BiometricStoreError::AlreadyEnrolled(UID))),
        "expected AlreadyEnrolled({UID}), got {result:?}"
    );
    assert_eq!(
        std::fs::read(store.template_path(UID).unwrap()).unwrap(),
        winner,
        "the concurrent enrollment is kept byte for byte"
    );
    assert_eq!(dir_entries(&dir), vec!["1000.cbor.enc".to_string()]);
}

/// Concurrent `enroll_if_absent` calls for one UID: exactly one wins, every other call is
/// refused, and the stored template is the winner's.
#[test]
fn test_sgu_concurrent_enroll_if_absent_has_exactly_one_winner() {
    const RACERS: usize = 8;
    let temp = TempDir::new().unwrap();
    let key = MasterKey::generate().unwrap();
    let dir = temp.path().join("bio");
    let store = open(&dir, &key);
    let barrier = Arc::new(Barrier::new(RACERS));

    let handles: Vec<_> = (0..RACERS)
        .map(|i| {
            let racer = open(&dir, &key);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let timestamp = 10 + i as u64;
                (timestamp, racer.enroll_if_absent(&template(0.5, timestamp)))
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let winners: Vec<u64> = results
        .iter()
        .filter(|(_, r)| r.is_ok())
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(winners.len(), 1, "exactly one winner: {results:?}");
    for (_, result) in results.iter().filter(|(_, r)| r.is_err()) {
        assert!(
            matches!(result, Err(BiometricStoreError::AlreadyEnrolled(UID))),
            "every loser is refused with AlreadyEnrolled: {result:?}"
        );
    }
    assert_eq!(
        store.get(UID).unwrap().unwrap().enrollment_timestamp,
        winners[0]
    );
    assert_eq!(dir_entries(&dir), vec!["1000.cbor.enc".to_string()]);
}

/// The refusal names the UID only (never embedding values).
#[test]
fn test_sgu_already_enrolled_error_names_the_uid_only() {
    let message = BiometricStoreError::AlreadyEnrolled(UID).to_string();
    assert!(message.contains("1000"), "{message}");
    assert!(message.contains("already enrolled"), "{message}");
}

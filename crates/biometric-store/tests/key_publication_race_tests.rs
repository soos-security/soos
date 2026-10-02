//! Contract tests for GitHub #303 (review finding STO-NEW-1): concurrent first-boot creation
//! of the master key must never replace a key another caller has already published. Every
//! caller racing on a missing path returns the single key that ends up on disk, and no
//! temporary key file is left behind.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_biometric_store::MasterKey;
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

const RACERS: usize = 16;
const ROUNDS: usize = 25;

fn leftover_temp_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect()
}

#[test]
fn test_303_concurrent_load_or_create_returns_the_published_key() {
    for round in 0..ROUNDS {
        let tmp = TempDir::new().expect("tempdir");
        let path = Arc::new(tmp.path().join("master.key"));
        let barrier = Arc::new(Barrier::new(RACERS));
        let handles: Vec<_> = (0..RACERS)
            .map(|_| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    MasterKey::load_or_create(path.as_path())
                })
            })
            .collect();
        let keys: Vec<MasterKey> = handles
            .into_iter()
            .map(|h| h.join().expect("racer thread").expect("load_or_create"))
            .collect();

        let on_disk = std::fs::read(path.as_path()).expect("published key");
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(
                key.as_bytes().as_slice(),
                on_disk.as_slice(),
                "round {round}: racer {i} returned a key that is not the published key"
            );
        }
        let leftovers = leftover_temp_files(tmp.path());
        assert!(
            leftovers.is_empty(),
            "round {round}: temporary key files left behind: {leftovers:?}"
        );
    }
}

#[test]
fn test_303_creation_leaves_no_temporary_file() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("nested").join("master.key");
    let created = MasterKey::load_or_create(&path).expect("create");
    let reloaded = MasterKey::load_or_create(&path).expect("reload");
    assert_eq!(created, reloaded);
    assert!(leftover_temp_files(path.parent().unwrap()).is_empty());
}

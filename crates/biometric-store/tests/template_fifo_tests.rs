//! Contract tests for GitHub #312 (review finding STO-NEW-6): a template read never blocks on
//! a FIFO planted at the template path. The file is opened `O_NOFOLLOW | O_NONBLOCK` and must
//! be a regular file; anything else fails closed without reading.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and expect"
)]

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use nix::sys::stat::Mode;
use soos_biometric_store::{BiometricStore, MasterKey};
use tempfile::TempDir;

const UID: u32 = 4312;

#[test]
fn test_312_template_read_does_not_block_on_a_fifo() {
    let temp = TempDir::new().unwrap();
    let store = Arc::new(
        BiometricStore::new(temp.path().join("bio"), MasterKey::generate().unwrap()).unwrap(),
    );
    let path = store.template_path(UID).unwrap();
    nix::unistd::mkfifo(&path, Mode::from_bits_truncate(0o600)).expect("mkfifo");

    let (tx, rx) = mpsc::channel();
    let reader = Arc::clone(&store);
    std::thread::spawn(move || {
        let _ = tx.send(reader.get(UID).is_err());
    });
    let failed_closed = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a template read must not block on a FIFO");
    assert!(failed_closed, "a FIFO template must be refused");
}

#[test]
fn test_312_template_directory_at_template_path_is_refused() {
    let temp = TempDir::new().unwrap();
    let store =
        BiometricStore::new(temp.path().join("bio"), MasterKey::generate().unwrap()).unwrap();
    std::fs::create_dir(store.template_path(UID).unwrap()).unwrap();
    assert!(store.get(UID).is_err(), "a directory is not a template");
}

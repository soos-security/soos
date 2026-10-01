//! GitHub #291 (SGU3): `soos-enroll import` without `--yes` writes through
//! `BiometricStore::enroll_if_absent`, so a template enrolled after the early duplicate check
//! (which still runs before the input is read, GitHub #237) is never replaced.
//!
//! The store lock is held from the test through `nix::fcntl::Flock` on the store directory; the
//! reader signals once the import has passed its early check, and the concurrent enrollment
//! lands while the import waits for the lock.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::fs::File;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::Duration;

use nix::fcntl::{Flock, FlockArg};
use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use soos_enrollment_cli::args::ImportArgs;
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::{EnrollmentService, IMPORT_STDIN_PATH};
use tempfile::tempdir;
use zeroize::Zeroizing;

const UID: u32 = 1000;

fn stdin_args() -> ImportArgs {
    ImportArgs {
        uid: Some(UID),
        username: None,
        file: PathBuf::from(IMPORT_STDIN_PATH),
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    }
}

/// Reader that reports its first `read` call: the import passed its early check.
struct SignallingReader {
    inner: Cursor<Vec<u8>>,
    signal: Option<Sender<()>>,
}

impl Read for SignallingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(signal) = self.signal.take() {
            let _ = signal.send(());
        }
        self.inner.read(buf)
    }
}

#[test]
fn test_sgu_import_without_yes_never_replaces_a_concurrent_enrollment() {
    let tmp = tempdir().unwrap();
    let key = MasterKey::load_or_create(tmp.path().join("master.key")).unwrap();
    let dir = tmp.path().join("biometrics");
    let store = Arc::new(BiometricStore::new(&dir, key.clone()).unwrap());
    let service = EnrollmentService::new_store_only(Arc::clone(&store), false);

    // The concurrent enrollment's ciphertext, produced by a second store with the same key.
    let staging = BiometricStore::new(tmp.path().join("staging"), key).unwrap();
    staging
        .enroll(
            &BiometricTemplate::new(
                UID,
                "arcface_w600k_mbf".to_string(),
                "2.0.0".to_string(),
                42,
                Zeroizing::new(vec![0.75; 512]),
            )
            .unwrap(),
        )
        .unwrap();
    let winner = std::fs::read(staging.template_path(UID).unwrap()).unwrap();

    let lock = Flock::lock(File::open(&dir).unwrap(), FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = SignallingReader {
        inner: Cursor::new(serde_json::to_vec(&vec![0.25_f32; 512]).unwrap()),
        signal: Some(tx),
    };
    let importer = std::thread::spawn(move || {
        service.import_with_overwrite_from_reader(&stdin_args(), false, reader)
    });

    rx.recv_timeout(Duration::from_secs(5))
        .expect("the import passed its early duplicate check and read its input");
    std::thread::sleep(Duration::from_millis(150));
    std::fs::write(store.template_path(UID).unwrap(), &winner).unwrap();
    drop(lock);

    match importer.join().unwrap() {
        Err(EnrollmentCliError::AlreadyEnrolled(uid)) => assert_eq!(uid, UID),
        other => panic!("the import must be refused atomically, got {other:?}"),
    }
    assert_eq!(
        std::fs::read(store.template_path(UID).unwrap()).unwrap(),
        winner,
        "the concurrent enrollment is kept byte for byte"
    );
    assert_eq!(store.get(UID).unwrap().unwrap().enrollment_timestamp, 42);
}

/// With `--yes` the import still replaces an existing template and reports it.
#[test]
fn test_sgu_import_with_yes_still_replaces_and_reports_it() {
    let tmp = tempdir().unwrap();
    let key = MasterKey::load_or_create(tmp.path().join("master.key")).unwrap();
    let store = Arc::new(BiometricStore::new(tmp.path().join("biometrics"), key).unwrap());
    let service = EnrollmentService::new_store_only(Arc::clone(&store), false);
    let json = || Cursor::new(serde_json::to_vec(&vec![0.5_f32; 512]).unwrap());

    let first = service
        .import_with_overwrite_from_reader(&stdin_args(), false, json())
        .unwrap();
    assert!(!first.replaced_existing);
    let second = service
        .import_with_overwrite_from_reader(&stdin_args(), true, json())
        .unwrap();
    assert!(second.replaced_existing);
}

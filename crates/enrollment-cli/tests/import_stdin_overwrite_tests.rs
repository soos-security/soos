//! Contractual tests for `soos-enroll import --file -` overwrite protection (GitHub #237
//! STO-21, candid review finding 1).
//!
//! Contract:
//! - A stdin import onto an already enrolled UID without `--yes` fails with
//!   `AlreadyEnrolled` BEFORE standard input is read, and the stored template stays
//!   byte-identical.
//! - With `--yes` the same stdin import replaces the template and reports the replacement.
//! - `import_with_overwrite` applies the same refusal to `--file -` as to a file path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use soos_biometric_store::{BiometricStore, MasterKey};
use soos_enrollment_cli::args::ImportArgs;
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::{EnrollmentService, IMPORT_STDIN_PATH};
use tempfile::{tempdir, TempDir};

fn service(tmp: &TempDir) -> (EnrollmentService, Arc<BiometricStore>) {
    let key = MasterKey::load_or_create(tmp.path().join("master.key")).expect("master key");
    let store = Arc::new(BiometricStore::new(tmp.path().join("biometrics"), key).expect("store"));
    (
        EnrollmentService::new_store_only(Arc::clone(&store), false),
        store,
    )
}

fn stdin_args(uid: u32) -> ImportArgs {
    ImportArgs {
        uid: Some(uid),
        username: None,
        file: PathBuf::from(IMPORT_STDIN_PATH),
        model_id: soos_enrollment_cli::service::MODEL_ID_EMBEDDING.to_string(),
        model_version: "2.0.0".to_string(),
    }
}

fn embedding_json(value: f32) -> Vec<u8> {
    serde_json::to_string(&vec![value; soos_enrollment_cli::service::IMPORT_EMBEDDING_DIM])
        .unwrap()
        .into_bytes()
}

/// Reader that counts every `read` call, proving whether the input was consumed.
struct CountingReader {
    inner: Cursor<Vec<u8>>,
    reads: Arc<AtomicUsize>,
}

impl Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read(buf)
    }
}

fn enroll_first(service: &EnrollmentService, store: &BiometricStore) -> Vec<u8> {
    service
        .import_from_reader(&stdin_args(1000), Cursor::new(embedding_json(0.25)))
        .expect("first stdin import");
    std::fs::read(store.template_path(1000).unwrap()).expect("stored template bytes")
}

#[test]
fn test_stdin_import_onto_enrolled_uid_without_yes_is_refused_before_reading() {
    let tmp = tempdir().unwrap();
    let (service, store) = service(&tmp);
    let before = enroll_first(&service, &store);

    let reads = Arc::new(AtomicUsize::new(0));
    let reader = CountingReader {
        inner: Cursor::new(embedding_json(0.75)),
        reads: Arc::clone(&reads),
    };
    match service.import_with_overwrite_from_reader(&stdin_args(1000), false, reader) {
        Err(EnrollmentCliError::AlreadyEnrolled(uid)) => assert_eq!(uid, 1000),
        other => panic!("stdin import without --yes must be refused, got {other:?}"),
    }
    assert_eq!(
        reads.load(Ordering::SeqCst),
        0,
        "standard input must not be read when the import is refused"
    );

    let after = std::fs::read(store.template_path(1000).unwrap()).unwrap();
    assert_eq!(
        before, after,
        "the enrolled template must stay byte-identical"
    );
}

#[test]
fn test_stdin_import_onto_enrolled_uid_with_yes_replaces_and_reports_it() {
    let tmp = tempdir().unwrap();
    let (service, store) = service(&tmp);
    let before = enroll_first(&service, &store);

    let outcome = service
        .import_with_overwrite_from_reader(
            &stdin_args(1000),
            true,
            Cursor::new(embedding_json(0.75)),
        )
        .expect("stdin import with --yes must replace");
    assert!(outcome.replaced_existing);

    let after = std::fs::read(store.template_path(1000).unwrap()).unwrap();
    assert_ne!(before, after, "--yes must replace the stored template");
    let loaded = store.get(1000).unwrap().expect("template stored");
    assert!((loaded.embedding[0] - 0.75).abs() < 1e-5);
}

#[test]
fn test_stdin_import_onto_fresh_uid_without_yes_succeeds() {
    let tmp = tempdir().unwrap();
    let (service, _store) = service(&tmp);
    let outcome = service
        .import_with_overwrite_from_reader(
            &stdin_args(1001),
            false,
            Cursor::new(embedding_json(0.5)),
        )
        .expect("a first stdin import needs no --yes");
    assert!(!outcome.replaced_existing);
}

#[test]
fn test_cli_stdin_path_without_yes_refuses_without_touching_stdin() {
    let tmp = tempdir().unwrap();
    let (service, store) = service(&tmp);
    let before = enroll_first(&service, &store);

    // `import_with_overwrite` is the exact call made by `soos-enroll import`; the refusal
    // must happen before the process standard input is read.
    match service.import_with_overwrite(&stdin_args(1000), false) {
        Err(EnrollmentCliError::AlreadyEnrolled(uid)) => assert_eq!(uid, 1000),
        other => panic!("`--file -` without --yes must be refused, got {other:?}"),
    }
    let after = std::fs::read(store.template_path(1000).unwrap()).unwrap();
    assert_eq!(
        before, after,
        "the enrolled template must stay byte-identical"
    );
}

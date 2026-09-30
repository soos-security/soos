//! Contractual tests for the bounded `soos-enroll import` input path (review findings CAM-08 /
//! STO-12, GitHub #156).
//!
//! Contract:
//! - `--file -` (`IMPORT_STDIN_PATH`) reads the embedding from standard input, so the GUI never
//!   writes the plaintext template to disk; `import_from_reader` is the testable entry point.
//! - Every input (stdin or file) is capped at `MAX_IMPORT_INPUT_BYTES` (64 KiB) while reading.
//! - The embedding must be a 512-value array of finite floats; anything else is refused and
//!   nothing is stored.
//! - A file input is opened without following symlinks, must be a regular file within the cap
//!   and, under `pkexec`, must be owned by `PKEXEC_UID`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use clap::Parser;
use soos_biometric_store::{BiometricStore, MasterKey};
use soos_enrollment_cli::args::{Cli, Commands, ImportArgs};
use soos_enrollment_cli::service::{
    parse_pkexec_uid, read_import_file, EnrollmentService, IMPORT_STDIN_PATH,
    MAX_IMPORT_INPUT_BYTES,
};
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
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    }
}

#[test]
fn test_import_stdin_marker_is_accepted_by_the_cli() {
    assert_eq!(IMPORT_STDIN_PATH, "-");
    let cli = Cli::try_parse_from(["soos-enroll", "import", "--uid", "1000", "--file", "-"])
        .expect("`--file -` must parse");
    match cli.command {
        Commands::Import(args) => assert_eq!(args.file, PathBuf::from("-")),
        other => panic!("unexpected command {other:?}"),
    }
}

#[test]
fn test_import_reads_embedding_from_stdin() {
    let tmp = tempdir().unwrap();
    let (service, store) = service(&tmp);
    let json = serde_json::to_string(&vec![0.042f32; 512]).unwrap();

    let outcome = service
        .import_from_reader(&stdin_args(1000), Cursor::new(json.into_bytes()))
        .expect("stdin import must succeed");
    assert_eq!(outcome.uid, 1000);
    assert_eq!(outcome.embedding_dim, 512);

    let loaded = store.get(1000).unwrap().expect("template stored");
    assert_eq!(loaded.embedding.len(), 512);
    assert!((loaded.embedding[0] - 0.042).abs() < 1e-5);
}

/// Endless reader counting how many bytes were pulled from it.
struct Endless(Arc<AtomicUsize>);

impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        buf.fill(b' ');
        self.0.fetch_add(buf.len(), Ordering::SeqCst);
        Ok(buf.len())
    }
}

#[test]
fn test_import_stdin_rejects_oversized_input_while_reading() {
    assert_eq!(MAX_IMPORT_INPUT_BYTES, 64 * 1024);
    let tmp = tempdir().unwrap();
    let (service, store) = service(&tmp);

    // A valid array followed by whitespace padding is valid JSON: only the cap rejects it.
    let mut padded = serde_json::to_string(&vec![0.042f32; 512])
        .unwrap()
        .into_bytes();
    padded.resize(MAX_IMPORT_INPUT_BYTES + 1, b' ');
    assert!(service
        .import_from_reader(&stdin_args(1000), Cursor::new(padded))
        .is_err());

    let consumed = Arc::new(AtomicUsize::new(0));
    assert!(service
        .import_from_reader(&stdin_args(1000), Endless(Arc::clone(&consumed)))
        .is_err());
    assert!(
        consumed.load(Ordering::SeqCst) <= MAX_IMPORT_INPUT_BYTES + 8192,
        "the bound must be enforced while reading, consumed {}",
        consumed.load(Ordering::SeqCst)
    );
    assert!(store.get(1000).unwrap().is_none(), "nothing may be stored");
}

#[test]
fn test_import_rejects_malformed_or_non_finite_embeddings() {
    let tmp = tempdir().unwrap();
    let (service, store) = service(&tmp);

    let mut overflowing: Vec<String> = vec!["0.1".to_string(); 512];
    overflowing[7] = "1e39".to_string(); // overflows f32 to +inf
    let payloads: Vec<Vec<u8>> = vec![
        format!("[{}]", overflowing.join(",")).into_bytes(),
        b"[0.1, 0.2".to_vec(),
        b"{\"embedding\": []}".to_vec(),
        b"".to_vec(),
        serde_json::to_string(&vec![0.1f32; 511])
            .unwrap()
            .into_bytes(),
    ];
    for payload in payloads {
        assert!(
            service
                .import_from_reader(&stdin_args(1000), Cursor::new(payload))
                .is_err(),
            "invalid payload must be refused"
        );
    }
    assert!(store.get(1000).unwrap().is_none(), "nothing may be stored");
}

#[test]
fn test_import_file_rejects_oversized_file() {
    let tmp = tempdir().unwrap();
    let (service, store) = service(&tmp);
    let path = tmp.path().join("big.json");
    let mut padded = serde_json::to_string(&vec![0.042f32; 512])
        .unwrap()
        .into_bytes();
    padded.resize(MAX_IMPORT_INPUT_BYTES + 1, b' ');
    std::fs::write(&path, padded).unwrap();

    let args = ImportArgs {
        file: path.clone(),
        ..stdin_args(1000)
    };
    assert!(
        service.import(&args).is_err(),
        "oversized file must be refused"
    );
    assert!(read_import_file(&path, None).is_err());
    assert!(store.get(1000).unwrap().is_none());
}

#[test]
fn test_import_file_owner_must_match_pkexec_uid() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("embedding.json");
    std::fs::write(&path, serde_json::to_string(&vec![0.042f32; 512]).unwrap()).unwrap();
    let me = nix::unistd::getuid().as_raw();

    assert!(read_import_file(&path, None).is_ok());
    assert!(read_import_file(&path, Some(me)).is_ok());
    assert!(
        read_import_file(&path, Some(me.wrapping_add(1))).is_err(),
        "a file not owned by the pkexec caller must be refused"
    );
}

#[test]
fn test_import_file_refuses_symlinks_and_non_regular_files() {
    let tmp = tempdir().unwrap();
    let target = tmp.path().join("embedding.json");
    std::fs::write(
        &target,
        serde_json::to_string(&vec![0.042f32; 512]).unwrap(),
    )
    .unwrap();
    let link = tmp.path().join("link.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    assert!(
        read_import_file(&link, None).is_err(),
        "symlink must be refused"
    );
    assert!(
        read_import_file(tmp.path(), None).is_err(),
        "directory must be refused"
    );
}

#[test]
fn test_parse_pkexec_uid() {
    assert_eq!(parse_pkexec_uid(None).unwrap(), None);
    assert_eq!(parse_pkexec_uid(Some("1000")).unwrap(), Some(1000));
    assert!(parse_pkexec_uid(Some("")).is_err());
    assert!(parse_pkexec_uid(Some("abc")).is_err());
    assert!(parse_pkexec_uid(Some("-1")).is_err());
}

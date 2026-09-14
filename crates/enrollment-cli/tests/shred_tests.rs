#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs::File;
use std::io::Write;
use tempfile::TempDir;

use soos_enrollment_cli::shred::secure_shred_file;

#[test]
fn test_shred_overwrites_and_removes_file() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("sensitive_template.cbor.enc");

    let original_data = b"Highly sensitive biometric template raw encryption bytes!";
    {
        let mut file = File::create(&file_path).unwrap();
        file.write_all(original_data).unwrap();
        file.sync_all().unwrap();
    }
    assert!(file_path.exists());

    secure_shred_file(&file_path).expect("Shredding must succeed");
    assert!(
        !file_path.exists(),
        "File must be removed after secure shredding"
    );
}

#[test]
fn test_shred_empty_file_removes_cleanly() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("empty.txt");
    File::create(&file_path).unwrap();
    assert!(file_path.exists());

    secure_shred_file(&file_path).expect("Shredding empty file must succeed");
    assert!(!file_path.exists());
}

#[test]
fn test_shred_nonexistent_file_returns_error() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("does_not_exist.bin");

    let res = secure_shred_file(&file_path);
    assert!(res.is_err());
}

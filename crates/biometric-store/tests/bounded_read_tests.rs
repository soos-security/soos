//! GitHub #235 (STO-19): template reads are size-bounded and metadata can be listed without
//! materialising the embedding.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and slicing"
)]

use std::io::Write;

use soos_biometric_store::{
    BiometricStore, BiometricStoreError, BiometricTemplate, MasterKey, TemplateMetadata,
    MAGIC_HEADER, MAX_TEMPLATE_FILE_BYTES, TEMPLATE_EXTENSION,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

fn store(temp: &TempDir) -> BiometricStore {
    BiometricStore::new(temp.path().join("bio"), MasterKey::generate().unwrap()).unwrap()
}

fn template(uid: u32, dim: usize) -> BiometricTemplate {
    let embedding: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.01).sin()).collect();
    BiometricTemplate::new(
        uid,
        "arcface_w600k_mbf".to_string(),
        "2.0.0".to_string(),
        1_700_000_000,
        Zeroizing::new(embedding),
    )
    .unwrap()
}

/// Writes a template-named file made of the real magic header followed by `len` junk bytes.
fn write_oversized(temp: &TempDir, uid: u32, len: usize) {
    let path = temp
        .path()
        .join("bio")
        .join(format!("{uid}{TEMPLATE_EXTENSION}"));
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(MAGIC_HEADER).unwrap();
    f.write_all(&vec![0xA5u8; len]).unwrap();
    f.sync_all().unwrap();
}

#[test]
fn test_max_template_file_bytes_fits_a_512d_template() {
    assert_eq!(MAX_TEMPLATE_FILE_BYTES, 64 * 1024);
    let temp = TempDir::new().unwrap();
    let s = store(&temp);
    s.enroll(&template(1000, 512)).unwrap();
    let len = std::fs::metadata(s.template_path(1000).unwrap())
        .unwrap()
        .len();
    assert!(
        len < MAX_TEMPLATE_FILE_BYTES / 4,
        "512D template is {len} bytes"
    );
}

#[test]
fn test_get_rejects_oversized_template_file_as_corrupt() {
    let temp = TempDir::new().unwrap();
    let s = store(&temp);
    write_oversized(&temp, 1000, 10 * 1024 * 1024);

    let res = s.get(1000);
    assert!(
        matches!(res, Err(BiometricStoreError::CorruptFile(_))),
        "a 10 MiB template file must be refused before decryption, got {res:?}"
    );
}

#[test]
fn test_get_metadata_rejects_oversized_template_file_as_corrupt() {
    let temp = TempDir::new().unwrap();
    let s = store(&temp);
    write_oversized(&temp, 1001, 10 * 1024 * 1024);

    let res = s.get_metadata(1001);
    assert!(
        matches!(res, Err(BiometricStoreError::CorruptFile(_))),
        "got {res:?}"
    );
}

#[test]
fn test_get_accepts_file_at_exact_bound_boundary_only_below() {
    // A file of exactly MAX_TEMPLATE_FILE_BYTES + 1 bytes is refused as oversized even though
    // its content would otherwise only fail authentication.
    let temp = TempDir::new().unwrap();
    let s = store(&temp);
    let over = usize::try_from(MAX_TEMPLATE_FILE_BYTES).unwrap() + 1 - MAGIC_HEADER.len();
    write_oversized(&temp, 1002, over);
    assert!(matches!(
        s.get(1002),
        Err(BiometricStoreError::CorruptFile(_))
    ));

    let at = usize::try_from(MAX_TEMPLATE_FILE_BYTES).unwrap() - MAGIC_HEADER.len();
    write_oversized(&temp, 1003, at);
    assert!(
        matches!(s.get(1003), Err(BiometricStoreError::Crypto(_))),
        "a file at the bound is read and fails authentication, not the size check"
    );
}

#[test]
fn test_enroll_refuses_template_exceeding_max_file_size() {
    let temp = TempDir::new().unwrap();
    let s = store(&temp);
    let res = s.enroll(&template(1004, 20_000));
    assert!(
        matches!(res, Err(BiometricStoreError::InvalidMetadata(_))),
        "a template that could never be read back must not be written, got {res:?}"
    );
    assert!(!s.exists(1004).unwrap());
    assert!(s.list_enrolled().unwrap().is_empty());
}

#[test]
fn test_get_metadata_returns_template_fields() {
    let temp = TempDir::new().unwrap();
    let s = store(&temp);
    s.enroll(&template(1005, 512)).unwrap();

    let meta: TemplateMetadata = s.get_metadata(1005).unwrap().expect("enrolled");
    assert_eq!(
        meta,
        TemplateMetadata {
            uid: 1005,
            model_id: "arcface_w600k_mbf".to_string(),
            model_version: "2.0.0".to_string(),
            enrollment_timestamp: 1_700_000_000,
            embedding_dim: 512,
        }
    );
}

#[test]
fn test_get_metadata_missing_template_is_none() {
    let temp = TempDir::new().unwrap();
    let s = store(&temp);
    assert!(s.get_metadata(4242).unwrap().is_none());
}

#[test]
fn test_get_metadata_refuses_uid_mismatch() {
    let temp = TempDir::new().unwrap();
    let s = store(&temp);
    s.enroll(&template(1006, 512)).unwrap();
    std::fs::rename(
        s.template_path(1006).unwrap(),
        s.template_path(1007).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        s.get_metadata(1007),
        Err(BiometricStoreError::CorruptFile(_))
    ));
}

#[test]
fn test_template_metadata_from_cbor_validates_dimension() {
    let t = template(1008, 16);
    let cbor = t.to_cbor().unwrap();
    let meta = TemplateMetadata::from_cbor(&cbor).unwrap();
    assert_eq!(meta.embedding_dim, 16);
    assert_eq!(meta.uid, 1008);

    // Truncated CBOR (embedding array cut short) is refused like `from_cbor`.
    let truncated = &cbor[..cbor.len() - 5];
    assert!(TemplateMetadata::from_cbor(truncated).is_err());
    assert!(BiometricTemplate::from_cbor(truncated).is_err());
}

#[test]
fn test_template_metadata_debug_matches_fields_only() {
    let meta = TemplateMetadata {
        uid: 1,
        model_id: "m".to_string(),
        model_version: "v".to_string(),
        enrollment_timestamp: 2,
        embedding_dim: 3,
    };
    let dbg = format!("{meta:?}");
    assert!(dbg.contains("embedding_dim: 3"));
}

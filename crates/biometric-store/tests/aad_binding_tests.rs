//! GitHub #266 (STO-22): template ciphertexts are bound to their UID, file role and format
//! version through AES-GCM associated data, while templates written before the change (legacy
//! unbound envelope) stay readable and are upgraded on the next write.
//!
//! These tests use only the pre-existing public API so that they fail at runtime (not at
//! compile time) on the unbound implementation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and slicing"
)]

use soos_biometric_store::{
    decrypt_payload, encrypt_payload, BiometricStore, BiometricStoreError, BiometricTemplate,
    MasterKey,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

/// Format marker expected right after the 8-byte magic of an AAD-bound (v2) template.
const EXPECTED_BOUND_MARKER: &[u8; 4] = b"AAD\x02";

fn template(uid: u32, value: f32) -> BiometricTemplate {
    BiometricTemplate::new(
        uid,
        "arcface_w600k_mbf".to_string(),
        "2.0.0".to_string(),
        1_700_000_000,
        Zeroizing::new(vec![value; 512]),
    )
    .unwrap()
}

fn open(temp: &TempDir) -> (BiometricStore, MasterKey) {
    let key = MasterKey::generate().unwrap();
    let store = BiometricStore::new(temp.path().join("bio"), key.clone()).unwrap();
    (store, key)
}

/// Writes `template` with the legacy unbound envelope, exactly as releases before #266 did.
fn write_legacy(store: &BiometricStore, key: &MasterKey, template: &BiometricTemplate) {
    let cbor = template.to_cbor().unwrap();
    let legacy = encrypt_payload(key, &cbor).unwrap();
    std::fs::write(store.template_path(template.uid).unwrap(), legacy).unwrap();
}

#[test]
fn test_sad_enrolled_template_declares_bound_format_and_uid() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open(&temp);
    store.enroll(&template(1000, 0.25)).unwrap();

    let bytes = std::fs::read(store.template_path(1000).unwrap()).unwrap();
    assert_eq!(&bytes[..8], b"SOOSBIO1", "the envelope magic is unchanged");
    assert_eq!(
        &bytes[8..12],
        EXPECTED_BOUND_MARKER,
        "a new template must carry the AAD-bound format marker"
    );
    assert_eq!(&bytes[12..16], &1000u32.to_be_bytes(), "clear bound UID");
}

#[test]
fn test_sad_enrolled_template_is_not_readable_as_unbound_payload() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    store.enroll(&template(1000, 0.25)).unwrap();

    let bytes = std::fs::read(store.template_path(1000).unwrap()).unwrap();
    assert!(
        decrypt_payload(&key, &bytes).is_err(),
        "a new template must not authenticate without its associated data"
    );
}

#[test]
fn test_sad_legacy_unbound_template_is_still_readable() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    write_legacy(&store, &key, &template(1000, 0.5));

    let loaded = store.get(1000).unwrap().expect("legacy template must load");
    assert_eq!(loaded.uid, 1000);
    assert!(loaded
        .embedding
        .iter()
        .all(|v| (*v - 0.5).abs() < f32::EPSILON));
    let meta = store.get_metadata(1000).unwrap().expect("legacy metadata");
    assert_eq!(meta.embedding_dim, 512);
}

#[test]
fn test_sad_reenroll_upgrades_legacy_template_to_bound_format() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    write_legacy(&store, &key, &template(1000, 0.5));

    store.enroll(&template(1000, 0.75)).unwrap();

    let bytes = std::fs::read(store.template_path(1000).unwrap()).unwrap();
    assert_eq!(&bytes[8..12], EXPECTED_BOUND_MARKER);
    assert!(decrypt_payload(&key, &bytes).is_err());
    let loaded = store.get(1000).unwrap().expect("upgraded template");
    assert!(loaded
        .embedding
        .iter()
        .all(|v| (*v - 0.75).abs() < f32::EPSILON));
}

#[test]
fn test_sad_legacy_template_with_foreign_uid_is_still_refused() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    // A legacy file of UID 1000 planted under the name of UID 1001.
    let cbor = template(1000, 0.5).to_cbor().unwrap();
    std::fs::write(
        store.template_path(1001).unwrap(),
        encrypt_payload(&key, &cbor).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        store.get(1001),
        Err(BiometricStoreError::CorruptFile(_))
    ));
}

#[test]
fn test_sad_bound_template_copied_to_other_uid_is_refused() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open(&temp);
    store.enroll(&template(1000, 0.25)).unwrap();
    std::fs::copy(
        store.template_path(1000).unwrap(),
        store.template_path(1001).unwrap(),
    )
    .unwrap();

    assert!(matches!(
        store.get(1001),
        Err(BiometricStoreError::CorruptFile(_))
    ));
    assert!(matches!(
        store.get_metadata(1001),
        Err(BiometricStoreError::CorruptFile(_))
    ));
    assert!(store.get(1000).unwrap().is_some(), "the original is intact");
}

#[test]
fn test_sad_forged_clear_uid_fails_authentication() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open(&temp);
    store.enroll(&template(1000, 0.25)).unwrap();

    let mut bytes = std::fs::read(store.template_path(1000).unwrap()).unwrap();
    bytes[12..16].copy_from_slice(&1001u32.to_be_bytes());
    std::fs::write(store.template_path(1001).unwrap(), &bytes).unwrap();

    assert!(
        matches!(store.get(1001), Err(BiometricStoreError::Crypto(_))),
        "rewriting the clear UID must break authentication"
    );
}

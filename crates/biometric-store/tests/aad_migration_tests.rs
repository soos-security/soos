//! GitHub #266 (STO-22): AAD-bound template codec, format reporting and explicit migration
//! of legacy unbound templates.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and slicing"
)]

use soos_biometric_store::{
    decrypt_template_payload, encrypt_payload, encrypt_template_payload, template_aad,
    BiometricStore, BiometricStoreError, BiometricTemplate, MasterKey, PayloadFormat,
    BOUND_FORMAT_MARKER, MAGIC_HEADER, PAYLOAD_FORMAT_VERSION,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

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

fn write_legacy(store: &BiometricStore, key: &MasterKey, template: &BiometricTemplate) {
    let cbor = template.to_cbor().unwrap();
    std::fs::write(
        store.template_path(template.uid).unwrap(),
        encrypt_payload(key, &cbor).unwrap(),
    )
    .unwrap();
}

#[test]
fn test_sad_format_constants() {
    assert_eq!(PAYLOAD_FORMAT_VERSION, 2);
    assert_eq!(&BOUND_FORMAT_MARKER, b"AAD\x02");
    assert_eq!(MAGIC_HEADER, b"SOOSBIO1");
}

#[test]
fn test_sad_template_aad_binds_domain_version_and_uid() {
    let a = template_aad(1000);
    let b = template_aad(1001);
    assert_ne!(a, b, "the AAD must differ per UID");
    assert!(a.starts_with(b"soos/biometric-template"));
    assert!(a.ends_with(&1000u32.to_be_bytes()));
    assert!(a.windows(4).any(|w| w == BOUND_FORMAT_MARKER));
}

#[test]
fn test_sad_decrypt_with_other_uid_fails_before_cbor_parsing() {
    let key = MasterKey::generate().unwrap();
    // Not CBOR at all: a parse attempt would report `Serialization`.
    let sealed = encrypt_template_payload(&key, 1000, b"not a cbor document").unwrap();

    let (plain, format) = decrypt_template_payload(&key, 1000, &sealed).unwrap();
    assert_eq!(plain.as_slice(), b"not a cbor document");
    assert_eq!(format, PayloadFormat::BoundV2);

    let res = decrypt_template_payload(&key, 1001, &sealed);
    assert!(
        matches!(res, Err(BiometricStoreError::CorruptFile(_))),
        "a template bound to another UID is refused by the codec, got {res:?}"
    );
}

#[test]
fn test_sad_bound_payload_rejects_tampering_and_wrong_key() {
    let key = MasterKey::generate().unwrap();
    let sealed = encrypt_template_payload(&key, 42, b"payload").unwrap();

    let other = MasterKey::generate().unwrap();
    assert!(decrypt_template_payload(&other, 42, &sealed).is_err());

    for index in [8usize, 11, 16, 27, sealed.len() - 1] {
        let mut tampered = sealed.clone();
        tampered[index] ^= 0x01;
        assert!(
            decrypt_template_payload(&key, 42, &tampered).is_err(),
            "flipping byte {index} must be detected"
        );
    }
    assert!(decrypt_template_payload(&key, 42, &sealed[..sealed.len() - 1]).is_err());
}

#[test]
fn test_sad_decrypt_template_payload_reports_legacy_format() {
    let key = MasterKey::generate().unwrap();
    let legacy = encrypt_payload(&key, b"legacy body").unwrap();
    let (plain, format) = decrypt_template_payload(&key, 7, &legacy).unwrap();
    assert_eq!(plain.as_slice(), b"legacy body");
    assert_eq!(format, PayloadFormat::LegacyV1);
}

#[test]
fn test_sad_template_format_reports_legacy_bound_and_missing() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    assert_eq!(store.template_format(1000).unwrap(), None);

    write_legacy(&store, &key, &template(1000, 0.5));
    assert_eq!(
        store.template_format(1000).unwrap(),
        Some(PayloadFormat::LegacyV1)
    );

    store.enroll(&template(1001, 0.5)).unwrap();
    assert_eq!(
        store.template_format(1001).unwrap(),
        Some(PayloadFormat::BoundV2)
    );
}

#[test]
fn test_sad_migrate_legacy_template_reencrypts_once_and_preserves_content() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    write_legacy(&store, &key, &template(1000, 0.5));
    let before = store.get(1000).unwrap().unwrap();

    assert!(store.migrate_legacy_template(1000).unwrap());
    assert_eq!(
        store.template_format(1000).unwrap(),
        Some(PayloadFormat::BoundV2)
    );
    let after = store.get(1000).unwrap().unwrap();
    assert_eq!(after.uid, before.uid);
    assert_eq!(after.model_id, before.model_id);
    assert_eq!(after.enrollment_timestamp, before.enrollment_timestamp);
    assert_eq!(*after.embedding, *before.embedding);

    assert!(
        !store.migrate_legacy_template(1000).unwrap(),
        "an already bound template is left untouched"
    );
    assert!(!store.migrate_legacy_template(4242).unwrap(), "missing");
}

#[test]
fn test_sad_migrate_refuses_legacy_template_with_foreign_uid() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open(&temp);
    let cbor = template(1000, 0.5).to_cbor().unwrap();
    let planted = encrypt_payload(&key, &cbor).unwrap();
    std::fs::write(store.template_path(1001).unwrap(), &planted).unwrap();

    assert!(matches!(
        store.migrate_legacy_template(1001),
        Err(BiometricStoreError::CorruptFile(_))
    ));
    assert_eq!(
        std::fs::read(store.template_path(1001).unwrap()).unwrap(),
        planted,
        "a refused file is never rewritten"
    );
}

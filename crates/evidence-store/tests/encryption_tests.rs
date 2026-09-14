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

use soos_evidence_store::crypto::{
    decrypt_payload, encrypt_payload, MasterKey, MAGIC_HEADER, NONCE_LEN,
};
use soos_evidence_store::{EvidenceConfig, EvidenceStore};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_encryption_roundtrip_and_structure() {
    let key = MasterKey::generate().unwrap();
    let plaintext = b"CONFIDENTIAL_INTRUDER_SNAPSHOT_PAYLOAD_1234567890";

    let encrypted = encrypt_payload(&key, plaintext).unwrap();

    // 1. Verify magic header
    assert_eq!(&encrypted[..8], MAGIC_HEADER);

    // 2. Verify length >= 8 (magic) + 12 (nonce) + plaintext.len() + 16 (tag)
    let expected_min_len = 8 + NONCE_LEN + plaintext.len() + 16;
    assert_eq!(encrypted.len(), expected_min_len);

    // 3. Verify plaintext is not visible in ciphertext
    let marker = b"CONFIDENTIAL_INTRUDER";
    let found = encrypted
        .windows(marker.len())
        .any(|window| window == marker);
    assert!(!found, "Plaintext string detected in ciphertext payload!");

    // 4. Verify decryption recovers exact plaintext
    let decrypted = decrypt_payload(&key, &encrypted).unwrap();
    assert_eq!(decrypted.as_slice(), plaintext);
}

#[test]
fn test_unique_nonces_for_identical_payloads() {
    let key = MasterKey::generate().unwrap();
    let plaintext = b"IDENTICAL_FRAME_DATA";

    let c1 = encrypt_payload(&key, plaintext).unwrap();
    let c2 = encrypt_payload(&key, plaintext).unwrap();

    assert_ne!(
        c1, c2,
        "Ciphertexts for identical plaintext must be distinct due to CSPRNG nonces"
    );

    // Extract nonces
    let nonce1 = &c1[8..8 + NONCE_LEN];
    let nonce2 = &c2[8..8 + NONCE_LEN];
    assert_ne!(nonce1, nonce2, "Nonces must be unique across encryptions");
}

#[test]
fn test_tamper_detection() {
    let key = MasterKey::generate().unwrap();
    let plaintext = b"UNMODIFIED_EVIDENCE_FRAME";
    let mut encrypted = encrypt_payload(&key, plaintext).unwrap();

    // Tamper with a byte in the ciphertext body
    let last_idx = encrypted.len() - 1;
    encrypted[last_idx] ^= 0xFF;

    let res = decrypt_payload(&key, &encrypted);
    assert!(
        res.is_err(),
        "Tampered ciphertext MUST fail decryption authenticated tag verification"
    );
}

#[test]
fn test_store_roundtrip_and_record_integrity() {
    let temp = TempDir::new().unwrap();
    let config = EvidenceConfig {
        enabled: true,
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: 10,
    };

    let store = EvidenceStore::open(config).unwrap();
    let frame = b"MOCK_WEBP_IMAGE_DATA_BYTES";

    let stored = store
        .store_snapshot(
            1005,
            "failed_pam_auth",
            frame,
            Some("2026-09-14"),
            Some(1726315000),
        )
        .unwrap();

    // Verify raw file on disk is encrypted
    let on_disk_bytes = fs::read(&stored.path).unwrap();
    assert_eq!(&on_disk_bytes[..8], MAGIC_HEADER);

    // Verify loading and decrypting through store
    let record = store.load_snapshot(&stored.path).unwrap();
    assert_eq!(record.snapshot_id, stored.snapshot_id);
    assert_eq!(record.uid, 1005);
    assert_eq!(record.reason, "failed_pam_auth");
    assert_eq!(record.timestamp, 1726315000);
    assert_eq!(record.image_data, frame);
}

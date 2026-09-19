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

use soos_biometric_store::{
    decrypt_payload, encrypt_payload, BiometricStore, BiometricTemplate, MasterKey,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

#[test]
fn test_b1_encryption_at_rest_and_tamper_detection() {
    let key = MasterKey::generate().expect("key generation failed");
    let plaintext = b"biometric_embedding_vector_secret_payload_512d";

    let ciphertext = encrypt_payload(&key, plaintext).expect("encryption failed");

    // Criterion B1: encrypted data at rest does not reveal plaintext
    assert!(!ciphertext.windows(plaintext.len()).any(|w| w == plaintext));

    // Decrypt succeeds with valid key
    let decrypted = decrypt_payload(&key, &ciphertext).expect("decryption failed");
    assert_eq!(&*decrypted, plaintext);

    // Decrypt fails with wrong key
    let wrong_key = MasterKey::generate().expect("wrong key generation");
    let err = decrypt_payload(&wrong_key, &ciphertext);
    assert!(err.is_err(), "decryption with wrong key must fail");

    // Tamper detection: flip a single bit in ciphertext body
    let mut tampered = ciphertext.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    let err_tampered = decrypt_payload(&key, &tampered);
    assert!(
        err_tampered.is_err(),
        "decryption of tampered ciphertext must fail"
    );
}

#[test]
fn test_b1_unique_nonce_per_write() {
    let key = MasterKey::generate().expect("key generation failed");
    let plaintext = b"fixed_embedding_vector_test";

    let c1 = encrypt_payload(&key, plaintext).expect("encrypt 1");
    let c2 = encrypt_payload(&key, plaintext).expect("encrypt 2");

    // Two encryptions of same plaintext must produce different ciphertexts due to fresh nonces
    assert_ne!(c1, c2, "ciphertexts must differ due to unique nonces");
}

#[test]
fn test_b1_file_on_disk_is_encrypted() {
    let tmp = TempDir::new().expect("tempdir");
    let key = MasterKey::generate().expect("key");
    let store = BiometricStore::new(tmp.path(), key).expect("store init");

    let floats = vec![0.12345_f32, 0.67890_f32, -0.45678_f32];
    let template = BiometricTemplate::new(
        1000,
        "facenet_v1".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(floats.clone()),
    )
    .expect("template");

    store.enroll(&template).expect("enroll");

    let template_file = store.template_path(1000).expect("template path");
    assert!(template_file.is_file());

    let raw_bytes = std::fs::read(&template_file).expect("read file");

    // Check that raw bytes do not contain unencrypted float bytes
    for f in &floats {
        let f_bytes = f.to_le_bytes();
        assert!(
            !raw_bytes.windows(4).any(|w| w == f_bytes),
            "raw float bytes must not appear plaintext in file"
        );
    }
}

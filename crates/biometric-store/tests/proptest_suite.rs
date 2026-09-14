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

use proptest::prelude::*;
use soos_biometric_store::{decrypt_payload, encrypt_payload, BiometricTemplate, MasterKey};
use zeroize::Zeroizing;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn prop_encrypt_decrypt_roundtrip(
        payload in proptest::collection::vec(any::<u8>(), 0..=2048),
        key_bytes in any::<[u8; 32]>(),
    ) {
        let key = MasterKey::from_bytes(key_bytes);
        let encrypted = encrypt_payload(&key, &payload).expect("encryption");
        let decrypted = decrypt_payload(&key, &encrypted).expect("decryption");
        prop_assert_eq!(&*decrypted, &payload);
    }

    #[test]
    fn prop_tampered_payload_always_fails_decryption(
        payload in proptest::collection::vec(any::<u8>(), 1..=512),
        key_bytes in any::<[u8; 32]>(),
        tamper_idx in any::<usize>(),
    ) {
        let key = MasterKey::from_bytes(key_bytes);
        let mut encrypted = encrypt_payload(&key, &payload).expect("encryption");

        // Mutate one byte in the encrypted stream
        let idx = tamper_idx % encrypted.len();
        encrypted[idx] ^= 0x5a;

        // Decryption must fail on tampered data
        prop_assert!(decrypt_payload(&key, &encrypted).is_err());
    }

    #[test]
    fn prop_template_cbor_serialization_roundtrip(
        uid in any::<u32>(),
        model_id in "[a-zA-Z0-9_-]{1,32}",
        model_ver in "[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}",
        timestamp in any::<u64>(),
        floats in proptest::collection::vec(-1.0_f32..=1.0_f32, 1..=128),
    ) {
        let template = BiometricTemplate::new(
            uid,
            model_id.clone(),
            model_ver.clone(),
            timestamp,
            Zeroizing::new(floats.clone()),
        ).expect("valid template");

        let cbor_bytes = template.to_cbor().expect("to_cbor");
        let restored = BiometricTemplate::from_cbor(&cbor_bytes).expect("from_cbor");

        prop_assert_eq!(restored.uid, uid);
        prop_assert_eq!(restored.model_id, model_id);
        prop_assert_eq!(restored.model_version, model_ver);
        prop_assert_eq!(restored.enrollment_timestamp, timestamp);
        prop_assert_eq!(restored.embedding_dim, floats.len());
        prop_assert_eq!(&*restored.embedding, &floats);
    }
}

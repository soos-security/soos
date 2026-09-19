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
use soos_evidence_store::crypto::{decrypt_payload, encrypt_payload, MasterKey};
use soos_evidence_store::{EvidenceConfig, EvidenceStore};
use tempfile::TempDir;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn test_payload_encryption_roundtrip(
        payload in proptest::collection::vec(any::<u8>(), 0..4096)
    ) {
        let key = MasterKey::generate().unwrap();
        let ciphertext = encrypt_payload(&key, &payload).unwrap();
        let decrypted = decrypt_payload(&key, &ciphertext).unwrap();
        prop_assert_eq!(decrypted.as_slice(), payload.as_slice());
    }

    #[test]
    fn test_store_and_load_roundtrip_proptest(
        uid in any::<u32>(),
        reason in "[a-zA-Z0-9_]{1,32}",
        frame in proptest::collection::vec(any::<u8>(), 1..2048),
    ) {
        let temp = TempDir::new().unwrap();
        let config = EvidenceConfig {
            enabled: true,
            base_dir: temp.path().join("evidence"),
            key_path: temp.path().join("evidence.key"),
            retention_days: 7,
            daily_cap_per_uid: 1000,
        };

        let store = EvidenceStore::open(config).unwrap();
        let res = match store.store_snapshot(
            uid,
            &reason,
            &frame,
            Some("2026-09-14"),
            Some(1726300000),
        ) {
            Ok(r) => r,
            Err(soos_evidence_store::EvidenceStoreError::InvalidUid(bad_uid)) => {
                prop_assert!(bad_uid > soos_evidence_store::MAX_VALID_UID);
                return Ok(());
            }
            Err(e) => panic!("Unexpected error during store_snapshot: {e:?}"),
        };

        let loaded = store.load_snapshot(&res.path).unwrap();
        prop_assert_eq!(loaded.snapshot_id, res.snapshot_id);
        prop_assert_eq!(loaded.uid, uid);
        prop_assert_eq!(loaded.reason, reason);
        prop_assert_eq!(loaded.image_data, frame);
    }
}

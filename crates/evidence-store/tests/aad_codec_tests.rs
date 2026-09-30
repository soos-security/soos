//! GitHub #266 (STO-22): AAD-bound evidence snapshot codec.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions"
)]

use soos_evidence_store::crypto::{
    decrypt_snapshot_payload, encrypt_payload, encrypt_snapshot_payload, snapshot_aad,
    PayloadFormat, BOUND_FORMAT_MARKER, MAGIC_HEADER, PAYLOAD_FORMAT_VERSION,
};
use soos_evidence_store::MasterKey;

const DATE: &str = "2026-09-14";
const ID: &str = "4b5f8e32-0000-4000-8000-9f8c12a45b67";

#[test]
fn test_sad_evidence_format_constants() {
    assert_eq!(PAYLOAD_FORMAT_VERSION, 2);
    assert_eq!(&BOUND_FORMAT_MARKER, b"AAD\x02");
    assert_eq!(MAGIC_HEADER, b"SOOSEVD1");
}

#[test]
fn test_sad_snapshot_aad_binds_domain_date_and_id() {
    let a = snapshot_aad(DATE, ID);
    assert!(a.starts_with(b"soos/evidence-snapshot"));
    assert!(a.windows(4).any(|w| w == BOUND_FORMAT_MARKER));
    assert_ne!(a, snapshot_aad("2026-09-13", ID));
    assert_ne!(
        a,
        snapshot_aad(DATE, "00000000-0000-4000-8000-000000000000")
    );
    // Component boundaries are unambiguous: moving characters between fields changes the AAD.
    assert_ne!(snapshot_aad("ab", "c"), snapshot_aad("a", "bc"));
}

#[test]
fn test_sad_snapshot_payload_roundtrip_and_binding() {
    let key = MasterKey::generate().unwrap();
    let sealed = encrypt_snapshot_payload(&key, DATE, ID, b"record").unwrap();
    assert_eq!(&sealed[..8], MAGIC_HEADER);
    assert_eq!(&sealed[8..12], &BOUND_FORMAT_MARKER);

    let (plain, format) = decrypt_snapshot_payload(&key, DATE, ID, &sealed).unwrap();
    assert_eq!(plain.as_slice(), b"record");
    assert_eq!(format, PayloadFormat::BoundV2);

    assert!(decrypt_snapshot_payload(&key, "2026-09-13", ID, &sealed).is_err());
    assert!(decrypt_snapshot_payload(&key, DATE, "other", &sealed).is_err());
    let other = MasterKey::generate().unwrap();
    assert!(decrypt_snapshot_payload(&other, DATE, ID, &sealed).is_err());
    for index in [8usize, 11, 12, sealed.len() - 1] {
        let mut tampered = sealed.clone();
        tampered[index] ^= 0x01;
        assert!(
            decrypt_snapshot_payload(&key, DATE, ID, &tampered).is_err(),
            "flipping byte {index} must be detected"
        );
    }
}

#[test]
fn test_sad_snapshot_codec_reads_legacy_unbound_payload() {
    let key = MasterKey::generate().unwrap();
    let legacy = encrypt_payload(&key, b"old record").unwrap();
    let (plain, format) = decrypt_snapshot_payload(&key, DATE, ID, &legacy).unwrap();
    assert_eq!(plain.as_slice(), b"old record");
    assert_eq!(format, PayloadFormat::LegacyV1);
}

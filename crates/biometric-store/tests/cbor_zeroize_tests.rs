#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

//! Contract test for GitHub #179 (review finding STO-06): the CBOR plaintext of a template
//! (the embedding in serialized form) is returned in a zeroizing buffer.

use soos_biometric_store::BiometricTemplate;
use zeroize::Zeroizing;

fn template(uid: u32, value: f32) -> BiometricTemplate {
    BiometricTemplate::new(
        uid,
        "arcface_w600k_mbf".to_string(),
        "1.0.0".to_string(),
        1_700_000_000,
        Zeroizing::new(vec![value; 512]),
    )
    .expect("template")
}

#[test]
fn test_to_cbor_returns_zeroizing_buffer() {
    let t = template(1002, 0.1);
    let bytes: Zeroizing<Vec<u8>> = t.to_cbor().expect("to_cbor");
    let restored = BiometricTemplate::from_cbor(&bytes).expect("from_cbor");
    assert_eq!(restored, t);
}

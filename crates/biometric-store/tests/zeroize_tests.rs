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

use soos_biometric_store::{BiometricTemplate, MasterKey};
use zeroize::Zeroizing;

#[test]
fn test_zeroize_master_key() {
    let key = MasterKey::generate().expect("key");
    assert_ne!(key.as_bytes(), &[0u8; 32]);

    // Test that debug does not reveal key bytes
    let debug_repr = format!("{:?}", key);
    assert!(!debug_repr.contains(&format!("{:02x}", key.as_bytes()[0])));
    assert!(debug_repr.contains("[REDACTED]"));
}

#[test]
fn test_zeroize_template_embedding() {
    let template = BiometricTemplate::new(
        1000,
        "facenet".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(vec![0.42_f32; 128]),
    )
    .expect("template");

    // Template Debug implementation must NOT print the raw float embedding
    let debug_repr = format!("{:?}", template);
    assert!(!debug_repr.contains("0.42"));
    assert!(debug_repr.contains("dim: 128") || debug_repr.contains("128"));
}

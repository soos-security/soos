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

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use tempfile::TempDir;
use zeroize::Zeroizing;

#[test]
fn test_b3_metadata_tracking_and_model_migration_support() {
    let tmp = TempDir::new().expect("tempdir");
    let key = MasterKey::generate().expect("key");
    let store = BiometricStore::new(tmp.path(), key).expect("store");

    let model_id = "glintr100".to_string();
    let model_version = "2.1.0".to_string();
    let enrollment_time = 1726000000_u64;
    let embedding = vec![0.01_f32; 512];

    let template = BiometricTemplate::new(
        1002,
        model_id.clone(),
        model_version.clone(),
        enrollment_time,
        Zeroizing::new(embedding.clone()),
    )
    .expect("valid template");

    store.enroll(&template).expect("enroll");

    let retrieved = store.get(1002).expect("get").expect("must exist");

    // Criterion B3: model_id, model_version, enrollment_timestamp, and embedding_dim are preserved
    assert_eq!(retrieved.uid, 1002);
    assert_eq!(retrieved.model_id, model_id);
    assert_eq!(retrieved.model_version, model_version);
    assert_eq!(retrieved.enrollment_timestamp, enrollment_time);
    assert_eq!(retrieved.embedding_dim, 512);
    assert_eq!(&*retrieved.embedding, &embedding);
}

#[test]
fn test_b3_template_validation_rejects_invalid_metadata() {
    // Empty model id must be rejected
    let err_empty_id = BiometricTemplate::new(
        1003,
        "".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(vec![0.1_f32; 128]),
    );
    assert!(err_empty_id.is_err(), "empty model_id must fail validation");

    // Empty model version must be rejected
    let err_empty_ver = BiometricTemplate::new(
        1003,
        "facenet".to_string(),
        "".to_string(),
        1700000000,
        Zeroizing::new(vec![0.1_f32; 128]),
    );
    assert!(
        err_empty_ver.is_err(),
        "empty model_version must fail validation"
    );

    // Zero-length embedding must be rejected
    let err_empty_vec = BiometricTemplate::new(
        1003,
        "facenet".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(Vec::new()),
    );
    assert!(
        err_empty_vec.is_err(),
        "empty embedding must fail validation"
    );

    // Non-finite float values must be rejected
    let err_nan = BiometricTemplate::new(
        1003,
        "facenet".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(vec![f32::NAN; 128]),
    );
    assert!(err_nan.is_err(), "NaN float must fail validation");
}

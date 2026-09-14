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
fn test_b4_full_crud_lifecycle() {
    let tmp = TempDir::new().expect("tempdir");
    let key = MasterKey::generate().expect("key");
    let store = BiometricStore::new(tmp.path(), key).expect("store");

    // Initially empty
    assert_eq!(store.list_enrolled().expect("list"), Vec::<u32>::new());
    assert!(!store.exists(1001).expect("exists"));
    assert_eq!(store.get(1001).expect("get"), None);

    // 1. Create (Enroll) UID 1001
    let t1 = BiometricTemplate::new(
        1001,
        "facenet_v1".to_string(),
        "1.0.0".to_string(),
        1700000001,
        Zeroizing::new(vec![0.1_f32; 128]),
    )
    .expect("t1");
    store.enroll(&t1).expect("enroll t1");

    assert!(store.exists(1001).expect("exists"));
    let loaded1 = store.get(1001).expect("get").expect("must exist");
    assert_eq!(loaded1.uid, 1001);
    assert_eq!(&*loaded1.embedding, &*t1.embedding);

    // 2. Enroll UID 1002 and UID 1005
    let t2 = BiometricTemplate::new(
        1002,
        "facenet_v1".to_string(),
        "1.0.0".to_string(),
        1700000002,
        Zeroizing::new(vec![0.2_f32; 128]),
    )
    .expect("t2");
    store.enroll(&t2).expect("enroll t2");

    let t5 = BiometricTemplate::new(
        1005,
        "facenet_v1".to_string(),
        "1.0.0".to_string(),
        1700000005,
        Zeroizing::new(vec![0.5_f32; 128]),
    )
    .expect("t5");
    store.enroll(&t5).expect("enroll t5");

    // Verify list_enrolled returns sorted UIDs
    let enrolled = store.list_enrolled().expect("list");
    assert_eq!(enrolled, vec![1001, 1002, 1005]);

    // 3. Update (Re-enroll) UID 1001 with new embedding
    let t1_updated = BiometricTemplate::new(
        1001,
        "glintr100".to_string(),
        "2.0.0".to_string(),
        1700000010,
        Zeroizing::new(vec![0.9_f32; 512]),
    )
    .expect("t1_updated");
    store.enroll(&t1_updated).expect("update t1");

    let loaded1_updated = store.get(1001).expect("get").expect("must exist");
    assert_eq!(loaded1_updated.model_id, "glintr100");
    assert_eq!(loaded1_updated.embedding_dim, 512);
    assert_eq!(&*loaded1_updated.embedding, &*t1_updated.embedding);

    // 4. Delete UID 1002
    assert!(store.delete(1002).expect("delete 1002"));
    assert!(!store.exists(1002).expect("exists"));
    assert_eq!(store.get(1002).expect("get"), None);

    // Deleting again returns false
    assert!(!store.delete(1002).expect("delete again"));

    // Check list after deletion
    let enrolled_after = store.list_enrolled().expect("list after delete");
    assert_eq!(enrolled_after, vec![1001, 1005]);
}

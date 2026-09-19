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

#[test]
fn test_delete_securely_overwrites_before_unlink() {
    let tmp = TempDir::new().expect("tempdir");
    let key = MasterKey::generate().expect("key");
    let store = BiometricStore::new(tmp.path(), key).expect("store");

    let t = BiometricTemplate::new(
        2001,
        "facenet_v1".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(vec![0.42_f32; 128]),
    )
    .expect("template");
    store.enroll(&t).expect("enroll");

    let template_path = store.template_path(2001).expect("template path");
    assert!(template_path.exists());

    // Create a hard link to the template file. Hard links share the exact same inode and data blocks.
    let hard_link = tmp.path().join("hardlink_probe.bin");
    std::fs::hard_link(&template_path, &hard_link).expect("create hard link probe");

    let original_bytes = std::fs::read(&hard_link).expect("read original bytes via probe");
    assert!(!original_bytes.is_empty());
    assert!(original_bytes.starts_with(b"SOOSBIO1"));

    // Delete the template
    let deleted = store.delete(2001).expect("delete must succeed");
    assert!(deleted, "delete must return true for existing template");

    // Template path must be unlinked
    assert!(!template_path.exists(), "template path must be unlinked");
    assert!(!store.exists(2001).expect("exists query"));

    // The hard link probe still points to the same sectors: verify secure overwrite
    let overwritten_bytes = std::fs::read(&hard_link).expect("read overwritten probe bytes");
    assert_eq!(
        overwritten_bytes.len(),
        original_bytes.len(),
        "file sectors must be overwritten in-place before unlinking"
    );
    assert_ne!(
        overwritten_bytes, original_bytes,
        "file content on disk must be completely overwritten before unlinking"
    );
    assert!(
        !overwritten_bytes.starts_with(b"SOOSBIO1"),
        "overwritten file must no longer contain the SOOSBIO1 magic header"
    );
}

#[test]
fn test_biometric_store_rejects_symlink_template_path() {
    let tmp = TempDir::new().expect("tempdir");
    let key = MasterKey::generate().expect("key");
    let store = BiometricStore::new(tmp.path(), key).expect("store");

    // 1. Target file simulation (/etc/shadow or decoy)
    let decoy = tmp.path().join("decoy_secret.txt");
    std::fs::write(&decoy, b"super_secret_payload").expect("write decoy");

    // 2. Create symlink pointing to decoy at template path for UID 2002
    let symlink_path = tmp.path().join("2002.cbor.enc");
    std::os::unix::fs::symlink(&decoy, &symlink_path).expect("create symlink");

    // template_path(2002) must reject the symlink
    let res = store.template_path(2002);
    assert!(
        res.is_err(),
        "template_path must return Err when path is a symlink"
    );

    // All CRUD operations for UID 2002 must fail closed
    assert!(
        store.exists(2002).is_err(),
        "exists must fail on symlink template path"
    );
    assert!(
        store.get(2002).is_err(),
        "get must fail on symlink template path"
    );
    assert!(
        store.delete(2002).is_err(),
        "delete must fail on symlink template path"
    );

    let template = BiometricTemplate::new(
        2002,
        "facenet_v1".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(vec![0.7_f32; 128]),
    )
    .expect("template");
    assert!(
        store.enroll(&template).is_err(),
        "enroll must fail on symlink template path"
    );

    // Decoy file must NOT have been modified or deleted
    let decoy_content = std::fs::read_to_string(&decoy).expect("read decoy");
    assert_eq!(decoy_content, "super_secret_payload");

    // 3. Broken symlink (target does not exist) must also be rejected
    let broken_symlink = tmp.path().join("2003.cbor.enc");
    std::os::unix::fs::symlink("/tmp/nonexistent_target_soos_12345", &broken_symlink)
        .expect("create broken symlink");
    assert!(
        store.template_path(2003).is_err(),
        "template_path must reject broken symlinks"
    );
}

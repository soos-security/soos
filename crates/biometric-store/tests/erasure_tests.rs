#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

//! Contract tests for GitHub #179 (review finding STO-06): re-enrollment must not leave the
//! previous template's bytes intact on the replaced inode, and the in-memory CBOR plaintext of
//! a template must be a zeroizing buffer.

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey, MAGIC_HEADER};
use std::os::unix::fs::MetadataExt;
use tempfile::TempDir;
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
fn test_reenroll_overwrites_previous_template_inode_before_release() {
    let tmp = TempDir::new().expect("tempdir");
    let store_dir = tmp.path().join("biometrics");
    let store =
        BiometricStore::new(&store_dir, MasterKey::generate().expect("key")).expect("store");

    store.enroll(&template(1000, 0.25)).expect("first enroll");
    let path = store.template_path(1000).expect("path");

    // A hard link keeps the first inode (and its data blocks) observable after the replace.
    let probe = tmp.path().join("previous_inode_probe");
    std::fs::hard_link(&path, &probe).expect("hard link probe");
    let original = std::fs::read(&probe).expect("read original");
    assert!(original.starts_with(MAGIC_HEADER));
    let old_ino = std::fs::metadata(&probe).expect("probe meta").ino();

    store.enroll(&template(1000, 0.75)).expect("re-enroll");

    // The new template is committed atomically on a new inode and is readable.
    let new_ino = std::fs::metadata(&path).expect("new meta").ino();
    assert_ne!(
        new_ino, old_ino,
        "re-enroll must atomically replace the file"
    );
    let loaded = store.get(1000).expect("get").expect("present");
    assert!(loaded
        .embedding
        .iter()
        .all(|v| (*v - 0.75).abs() < f32::EPSILON));

    // The previous inode's content must have been overwritten in place.
    let after = std::fs::read(&probe).expect("read probe after re-enroll");
    assert_eq!(
        after.len(),
        original.len(),
        "overwrite must cover the whole file"
    );
    assert_ne!(
        after, original,
        "previous ciphertext must not survive re-enroll"
    );
    assert!(
        !after.starts_with(MAGIC_HEADER),
        "previous inode must no longer carry the SOOSBIO1 header"
    );

    // No temporary files are left behind.
    let names: Vec<String> = std::fs::read_dir(&store_dir)
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["1000.cbor.enc".to_string()]);
}

#[test]
fn test_first_enroll_without_previous_template_succeeds() {
    let tmp = TempDir::new().expect("tempdir");
    let store = BiometricStore::new(tmp.path().join("b"), MasterKey::generate().expect("key"))
        .expect("store");

    store.enroll(&template(1001, 0.5)).expect("first enroll");
    assert!(store.exists(1001).expect("exists"));
}

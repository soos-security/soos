//! Contract tests of GitHub #325 item 5 (matrix PFU6): `BiometricStore::has_enrolled_template`,
//! the cheap, bounded probe the presence worker runs every tick before any D-Bus traffic.
//!
//! Contract: `true` at the first entry named `<canonical u32>.cbor.enc` whose type (not
//! followed) is a regular file; no content is read or decrypted; at most
//! `MAX_ENROLLMENT_PROBE_ENTRIES` entries are examined (more entries without a match is an
//! error); an unlistable (e.g. missing) directory is an error.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::os::unix::fs::symlink;

use soos_biometric_store::store::MAX_ENROLLMENT_PROBE_ENTRIES;
use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey, TEMPLATE_EXTENSION};
use tempfile::TempDir;
use zeroize::Zeroizing;

fn store() -> (TempDir, BiometricStore) {
    let tmp = TempDir::new().unwrap();
    let store = BiometricStore::new(
        tmp.path().join("biometrics"),
        MasterKey::generate().unwrap(),
    )
    .unwrap();
    (tmp, store)
}

fn template(uid: u32) -> BiometricTemplate {
    BiometricTemplate::new(
        uid,
        "sface_2021dec".to_string(),
        "2.0.0".to_string(),
        1,
        Zeroizing::new(vec![0.1_f32; 128]),
    )
    .unwrap()
}

#[test]
fn test_pfu_probe_bound_is_4096_entries() {
    assert_eq!(MAX_ENROLLMENT_PROBE_ENTRIES, 4096);
}

/// PFU6: an empty store holds no template.
#[test]
fn test_pfu_probe_is_false_for_an_empty_store() {
    let (_tmp, store) = store();
    assert!(!store.has_enrolled_template().unwrap());
}

/// PFU6: one enrolled template is enough; deleting it makes the store empty again.
#[test]
fn test_pfu_probe_is_true_with_one_template_and_false_after_delete() {
    let (_tmp, store) = store();
    store.enroll(&template(1000)).unwrap();
    assert!(store.has_enrolled_template().unwrap());
    assert!(store.delete(1000).unwrap());
    assert!(!store.has_enrolled_template().unwrap());
}

/// PFU6: only `<canonical u32>.cbor.enc` regular files count: temporary files, other names,
/// non-canonical or overflowing UIDs, directories and symlinks (even to a real template) do not.
#[test]
fn test_pfu_probe_ignores_entries_that_are_not_template_files() {
    let (tmp, store) = store();
    let dir = tmp.path().join("biometrics");
    let real = tmp.path().join("outside.cbor.enc");
    fs::write(&real, b"x").unwrap();
    for name in [
        "1000.tmp.42.7".to_string(),
        "notes.txt".to_string(),
        format!("0123{TEMPLATE_EXTENSION}"),
        format!("+5{TEMPLATE_EXTENSION}"),
        format!("-1{TEMPLATE_EXTENSION}"),
        TEMPLATE_EXTENSION.to_string(),
        format!("4294967296{TEMPLATE_EXTENSION}"),
        format!("1000{TEMPLATE_EXTENSION}.bak"),
        "1000.cbor".to_string(),
    ] {
        fs::write(dir.join(&name), b"x").unwrap();
    }
    fs::create_dir(dir.join(format!("1001{TEMPLATE_EXTENSION}"))).unwrap();
    symlink(&real, dir.join(format!("1002{TEMPLATE_EXTENSION}"))).unwrap();
    symlink(
        tmp.path().join("dangling"),
        dir.join(format!("1003{TEMPLATE_EXTENSION}")),
    )
    .unwrap();
    assert!(
        !store.has_enrolled_template().unwrap(),
        "none of these entries is a template file"
    );
    fs::write(dir.join(format!("0{TEMPLATE_EXTENSION}")), b"x").unwrap();
    assert!(
        store.has_enrolled_template().unwrap(),
        "`0` is a canonical UID"
    );
}

/// PFU6: the probe never reads or decrypts: an undecryptable file with a template name counts
/// (the per-UID `get` of the worker reports it as a store error later).
#[test]
fn test_pfu_probe_does_not_decrypt() {
    let (tmp, store) = store();
    let dir = tmp.path().join("biometrics");
    fs::write(dir.join(format!("1000{TEMPLATE_EXTENSION}")), b"garbage").unwrap();
    assert!(store.has_enrolled_template().unwrap());
    assert!(
        store.get(1000).is_err(),
        "the garbage file is not a valid template"
    );
}

/// PFU6: exactly `MAX_ENROLLMENT_PROBE_ENTRIES` non-template entries are examined (no
/// template ⇒ `false`); one more entry without a template is an error, never `false`.
#[test]
fn test_pfu_probe_is_bounded_by_max_entries() {
    let (tmp, store) = store();
    let dir = tmp.path().join("biometrics");
    for i in 0..MAX_ENROLLMENT_PROBE_ENTRIES {
        fs::write(dir.join(format!("junk-{i}")), b"").unwrap();
    }
    assert!(!store.has_enrolled_template().unwrap(), "exactly the bound");
    fs::write(dir.join("junk-extra"), b"").unwrap();
    assert!(
        store.has_enrolled_template().is_err(),
        "more entries than the bound without a template is an error"
    );
}

/// PFU6: a store directory that cannot be listed (here: removed) is an error, never `false`.
#[test]
fn test_pfu_probe_missing_directory_is_an_error() {
    let (tmp, store) = store();
    fs::remove_dir_all(tmp.path().join("biometrics")).unwrap();
    assert!(store.has_enrolled_template().is_err());
}

/// PFU6: the probe changes nothing in the store directory.
#[test]
fn test_pfu_probe_is_read_only() {
    let (tmp, store) = store();
    store.enroll(&template(1000)).unwrap();
    let dir = tmp.path().join("biometrics");
    let list = |dir: &std::path::Path| {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    let before = list(&dir);
    for _ in 0..3 {
        assert!(store.has_enrolled_template().unwrap());
    }
    assert_eq!(list(&dir), before);
    assert_eq!(store.get(1000).unwrap().unwrap().uid, 1000);
}

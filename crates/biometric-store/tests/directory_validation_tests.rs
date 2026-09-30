#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

//! Contract tests for GitHub #178 (review finding STO-04): `BiometricStore::new` must never
//! change the permissions of an existing, administrator-chosen directory. An existing
//! directory is validated (real directory, not a symlink, owned by root or by the effective
//! UID, not group- or world-writable) and refused with `InvalidPath` otherwise. Only a
//! directory created by the call itself receives mode `0700`.

use soos_biometric_store::{BiometricStore, BiometricStoreError, MasterKey};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use tempfile::TempDir;

fn mode_of(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o7777
}

fn make_dir_with_mode(parent: &Path, name: &str, mode: u32) -> std::path::PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).expect("chmod");
    assert_eq!(mode_of(&dir), mode, "fixture mode must be applied");
    dir
}

fn assert_invalid_path(res: Result<BiometricStore, BiometricStoreError>, what: &str) {
    match res {
        Err(BiometricStoreError::InvalidPath(_)) => {}
        other => panic!("{what}: expected Err(InvalidPath), got {other:?}"),
    }
}

#[test]
fn test_new_refuses_world_writable_sticky_dir_and_leaves_mode_unchanged() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = make_dir_with_mode(tmp.path(), "shared_tmp", 0o1777);

    let res = BiometricStore::new(&dir, MasterKey::generate().expect("key"));

    assert_invalid_path(res, "a 1777 directory (like /tmp) must be refused");
    assert_eq!(
        mode_of(&dir),
        0o1777,
        "an existing directory's mode (including the sticky bit) must never be rewritten"
    );
}

#[test]
fn test_new_refuses_group_writable_dir_and_leaves_mode_unchanged() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = make_dir_with_mode(tmp.path(), "group_writable", 0o770);

    let res = BiometricStore::new(&dir, MasterKey::generate().expect("key"));

    assert_invalid_path(res, "a group-writable directory must be refused");
    assert_eq!(mode_of(&dir), 0o770, "mode must be left unchanged");
}

#[test]
fn test_new_refuses_world_writable_dir_without_sticky_bit() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = make_dir_with_mode(tmp.path(), "world_writable", 0o707);

    let res = BiometricStore::new(&dir, MasterKey::generate().expect("key"));

    assert_invalid_path(res, "a world-writable directory must be refused");
    assert_eq!(mode_of(&dir), 0o707, "mode must be left unchanged");
}

#[test]
fn test_new_accepts_safe_existing_dir_without_touching_its_mode() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = make_dir_with_mode(tmp.path(), "admin_chosen", 0o755);

    BiometricStore::new(&dir, MasterKey::generate().expect("key"))
        .expect("an owner-only-writable directory must be accepted");

    assert_eq!(
        mode_of(&dir),
        0o755,
        "an accepted existing directory must keep the mode the administrator chose"
    );
}

#[test]
fn test_new_preserves_setgid_bit_on_safe_existing_dir() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = make_dir_with_mode(tmp.path(), "setgid_dir", 0o2750);

    BiometricStore::new(&dir, MasterKey::generate().expect("key"))
        .expect("a 2750 directory is not group/world writable and must be accepted");

    assert_eq!(
        mode_of(&dir),
        0o2750,
        "the setgid bit of an existing directory must never be dropped"
    );
}

#[test]
fn test_new_refuses_existing_non_directory() {
    let tmp = TempDir::new().expect("tempdir");
    let file = tmp.path().join("not_a_dir");
    std::fs::write(&file, b"x").expect("write file");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("chmod");

    let res = BiometricStore::new(&file, MasterKey::generate().expect("key"));

    assert_invalid_path(res, "a regular file must be refused as a store directory");
    assert_eq!(
        mode_of(&file),
        0o644,
        "the file mode must be left unchanged"
    );
}

#[test]
fn test_new_refuses_symlinked_store_dir() {
    let tmp = TempDir::new().expect("tempdir");
    let target = make_dir_with_mode(tmp.path(), "real_dir", 0o700);
    let link = tmp.path().join("link_dir");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");

    let res = BiometricStore::new(&link, MasterKey::generate().expect("key"));

    assert_invalid_path(res, "a symlinked store directory must be refused");
}

#[test]
fn test_new_creates_missing_dir_with_0700() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path().join("nested").join("biometrics");

    BiometricStore::new(&dir, MasterKey::generate().expect("key"))
        .expect("a missing directory must be created");

    assert_eq!(
        mode_of(&dir),
        0o700,
        "a created store directory must be 0700"
    );
}

#[test]
fn test_new_ownership_rule_against_effective_uid() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = make_dir_with_mode(tmp.path(), "owned", 0o700);
    let euid = std::fs::metadata(tmp.path()).expect("metadata").uid();

    // A directory owned by the effective UID is always acceptable.
    BiometricStore::new(&dir, MasterKey::generate().expect("key"))
        .expect("a directory owned by the effective UID must be accepted");

    if euid == 0 {
        // As root, a directory owned by an unprivileged account must be refused.
        let foreign = make_dir_with_mode(tmp.path(), "foreign", 0o700);
        std::os::unix::fs::chown(&foreign, Some(65534), None).expect("chown to nobody");
        let res = BiometricStore::new(&foreign, MasterKey::generate().expect("key"));
        assert_invalid_path(res, "a directory owned by a foreign UID must be refused");
        assert_eq!(mode_of(&foreign), 0o700, "mode must be left unchanged");
    }
}

//! Contract tests for GitHub #230 (review finding STO-14): an existing evidence key is only
//! accepted when it is a regular file owned by root or by the effective UID, has no group or
//! world permission bit, and is exactly `MASTER_KEY_LEN` bytes (read with a bounded read).
//! Creating a key never forces `0700` on the parent directory: missing parents are created
//! `0755` (the `/var/lib/soos` contract) and existing parents are never modified.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and expect"
)]

use soos_evidence_store::crypto::MASTER_KEY_LEN;
use soos_evidence_store::{EvidenceStoreError, MasterKey};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;
use tempfile::TempDir;

fn write_key(path: &Path, bytes: &[u8], mode: u32) {
    std::fs::write(path, bytes).expect("write key");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod key");
}

fn mode_of(path: &Path) -> u32 {
    std::fs::symlink_metadata(path).expect("metadata").mode() & 0o7777
}

fn is_root() -> bool {
    nix::unistd::geteuid().is_root()
}

#[test]
fn test_230_world_readable_key_is_refused_with_key_error() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("master.key");
    write_key(&path, &[7u8; MASTER_KEY_LEN], 0o644);

    let res = MasterKey::load_or_create(&path);
    assert!(
        matches!(res, Err(EvidenceStoreError::KeyError(_))),
        "a 0644 master key must be refused, got {res:?}"
    );
    assert_eq!(
        mode_of(&path),
        0o644,
        "the refused key must not be modified"
    );
}

#[test]
fn test_230_group_accessible_keys_are_refused() {
    for mode in [0o640, 0o620, 0o610, 0o604, 0o602, 0o601] {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("master.key");
        write_key(&path, &[7u8; MASTER_KEY_LEN], mode);
        let res = MasterKey::load_or_create(&path);
        assert!(
            matches!(res, Err(EvidenceStoreError::KeyError(_))),
            "mode {mode:o} must be refused, got {res:?}"
        );
    }
}

#[test]
fn test_230_owner_only_key_of_exact_length_is_accepted() {
    for mode in [0o600, 0o400] {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("master.key");
        write_key(&path, &[9u8; MASTER_KEY_LEN], mode);
        let key = MasterKey::load_or_create(&path).expect("0600/0400 key accepted");
        assert_eq!(key.as_bytes(), &[9u8; MASTER_KEY_LEN]);
    }
}

#[test]
fn test_230_wrong_length_keys_are_refused() {
    for len in [0, 31, 33, 4096] {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("master.key");
        write_key(&path, &vec![1u8; len], 0o600);
        let res = MasterKey::load_or_create(&path);
        assert!(
            matches!(res, Err(EvidenceStoreError::KeyError(_))),
            "a {len}-byte key must be refused with KeyError, got {res:?}"
        );
    }
}

#[test]
fn test_230_non_regular_key_paths_are_refused_without_blocking() {
    let tmp = TempDir::new().expect("tempdir");

    let dir_path = tmp.path().join("dir.key");
    std::fs::create_dir(&dir_path).expect("mkdir");
    assert!(MasterKey::load_or_create(&dir_path).is_err());

    let fifo_path = tmp.path().join("fifo.key");
    nix::unistd::mkfifo(&fifo_path, nix::sys::stat::Mode::from_bits_truncate(0o600))
        .expect("mkfifo");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(MasterKey::load_or_create(&fifo_path).is_err());
    });
    let refused = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("loading a FIFO key path must not block");
    assert!(refused, "a FIFO key path must be refused");
}

#[test]
fn test_230_foreign_owned_key_is_refused() {
    if !is_root() {
        // Changing a file owner needs root; unprivileged runs skip this case.
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("master.key");
    write_key(&path, &[7u8; MASTER_KEY_LEN], 0o600);
    std::os::unix::fs::chown(&path, Some(65534), None).expect("chown to nobody");
    let res = MasterKey::load_or_create(&path);
    assert!(
        matches!(res, Err(EvidenceStoreError::KeyError(_))),
        "a key owned by another user must be refused, got {res:?}"
    );
}

#[test]
fn test_230_missing_parent_is_created_0755_not_0700() {
    let tmp = TempDir::new().expect("tempdir");
    let parent = tmp.path().join("var_lib_soos");
    let path = parent.join("master.key");

    MasterKey::load_or_create(&path).expect("create key");
    assert_eq!(
        mode_of(&parent),
        0o755,
        "a created key parent must follow the /var/lib/soos 0755 contract"
    );
    assert_eq!(mode_of(&path), 0o600);
}

#[test]
fn test_230_missing_nested_parents_are_all_created_0755() {
    let tmp = TempDir::new().expect("tempdir");
    let outer = tmp.path().join("a");
    let inner = outer.join("b");
    MasterKey::load_or_create(inner.join("master.key")).expect("create key");
    assert_eq!(mode_of(&outer), 0o755);
    assert_eq!(mode_of(&inner), 0o755);
}

#[test]
fn test_230_existing_parent_mode_is_never_modified() {
    for mode in [0o755, 0o711, 0o750] {
        let tmp = TempDir::new().expect("tempdir");
        let parent = tmp.path().join("existing");
        std::fs::create_dir(&parent).expect("mkdir");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(mode)).expect("chmod");

        MasterKey::load_or_create(parent.join("master.key")).expect("create key");
        assert_eq!(mode_of(&parent), mode, "existing parent mode changed");
    }
}

#[test]
fn test_230_symlinked_evidence_key_is_refused_and_target_untouched() {
    let tmp = TempDir::new().expect("tempdir");
    let target = tmp.path().join("decoy");
    write_key(&target, &[3u8; MASTER_KEY_LEN], 0o600);
    let link = tmp.path().join("evidence.key");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");

    let res = MasterKey::load_or_create(&link);
    assert!(res.is_err(), "a symlinked evidence key must be refused");
    assert_eq!(
        std::fs::read(&target).expect("read decoy"),
        vec![3u8; MASTER_KEY_LEN]
    );
    assert!(std::fs::symlink_metadata(&link)
        .expect("link metadata")
        .file_type()
        .is_symlink());
}

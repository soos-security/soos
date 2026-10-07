//! Contract tests of GitHub #339 for the socket directory and listener preparation (spec
//! §2.7, D2). Everything lives in a `tempfile::TempDir`; the foreign-owner cases pass a
//! different `uid` instead of needing root.

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

use std::fs;
use std::os::unix::fs::{symlink, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::Path;

use soos_remote::socket::{bind_listener, prepare_socket_dir, SocketError};

fn own_uid(dir: &Path) -> u32 {
    fs::metadata(dir).unwrap().uid()
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

/// §2.7: an absent directory is created 0700 and owned by the caller.
#[test]
fn test_rmc_prepare_socket_dir_creates_0700_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let uid = own_uid(tmp.path());
    let parent = tmp.path().join("soos-remote");
    prepare_socket_dir(&parent, uid).unwrap();
    let meta = fs::symlink_metadata(&parent).unwrap();
    assert!(meta.is_dir());
    assert_eq!(meta.permissions().mode() & 0o7777, 0o700);
    assert_eq!(meta.uid(), uid);
    // Idempotent.
    prepare_socket_dir(&parent, uid).unwrap();
    assert_eq!(mode(&parent), 0o700);
}

/// §2.7: only one missing level is created (the runtime dir itself must exist).
#[test]
fn test_rmc_prepare_socket_dir_refuses_two_missing_levels() {
    let tmp = tempfile::tempdir().unwrap();
    let uid = own_uid(tmp.path());
    let parent = tmp.path().join("missing-runtime").join("soos-remote");
    let err = prepare_socket_dir(&parent, uid).unwrap_err();
    assert!(matches!(err, SocketError::Io(_)), "{err:?}");
    assert!(!tmp.path().join("missing-runtime").exists());
}

/// §2.7: an existing directory with a loose mode is tightened to 0700.
#[test]
fn test_rmc_prepare_socket_dir_tightens_an_existing_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let uid = own_uid(tmp.path());
    let parent = tmp.path().join("soos-remote");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(mode(&parent), 0o755);
    prepare_socket_dir(&parent, uid).unwrap();
    assert_eq!(mode(&parent), 0o700);
}

/// §2.7: a symlink (even to a directory) or a regular file at the parent path is refused
/// and left untouched.
#[test]
fn test_rmc_prepare_socket_dir_refuses_symlink_and_regular_file() {
    let tmp = tempfile::tempdir().unwrap();
    let uid = own_uid(tmp.path());
    let target = tmp.path().join("real-dir");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    let link = tmp.path().join("soos-remote");
    symlink(&target, &link).unwrap();
    let err = prepare_socket_dir(&link, uid).unwrap_err();
    assert!(matches!(err, SocketError::NotADirectory), "{err:?}");
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(mode(&target), 0o755, "the symlink target is never modified");

    let file = tmp.path().join("file-not-dir");
    fs::write(&file, b"x").unwrap();
    let err = prepare_socket_dir(&file, uid).unwrap_err();
    assert!(matches!(err, SocketError::NotADirectory), "{err:?}");
    assert_eq!(fs::read(&file).unwrap(), b"x");
}

/// §2.7: a directory owned by another uid is refused and its mode is not changed.
#[test]
fn test_rmc_prepare_socket_dir_refuses_a_foreign_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let uid = own_uid(tmp.path());
    let parent = tmp.path().join("soos-remote");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    let err = prepare_socket_dir(&parent, uid.wrapping_add(1)).unwrap_err();
    assert!(matches!(err, SocketError::WrongOwner), "{err:?}");
    assert_eq!(mode(&parent), 0o755, "no chmod on a foreign directory");
}

/// §2.7 / D2: the listener is bound 0600 and a stale own socket is replaced.
#[tokio::test]
async fn test_rmc_bind_listener_binds_0600_and_replaces_a_stale_socket() {
    let tmp = tempfile::tempdir().unwrap();
    let uid = own_uid(tmp.path());
    let parent = tmp.path().join("soos-remote");
    prepare_socket_dir(&parent, uid).unwrap();
    let path = parent.join("remote.sock");
    let listener = bind_listener(&path, uid).unwrap();
    let meta = fs::symlink_metadata(&path).unwrap();
    assert!(meta.file_type().is_socket());
    assert_eq!(meta.permissions().mode() & 0o7777, 0o600);
    assert_eq!(meta.uid(), uid);
    // A client can connect while the listener lives.
    let client = tokio::net::UnixStream::connect(&path).await;
    assert!(client.is_ok());
    drop(client);
    drop(listener);
    assert!(path.exists(), "the socket file is stale now");
    let again = bind_listener(&path, uid).unwrap();
    assert_eq!(mode(&path), 0o600);
    drop(again);
}

/// §2.7: a regular file, a symlink or a socket of another uid at the path is never
/// unlinked.
#[tokio::test]
async fn test_rmc_bind_listener_refuses_non_socket_paths_without_unlinking() {
    let tmp = tempfile::tempdir().unwrap();
    let uid = own_uid(tmp.path());
    let parent = tmp.path().join("soos-remote");
    prepare_socket_dir(&parent, uid).unwrap();

    let file = parent.join("remote.sock");
    fs::write(&file, b"not a socket").unwrap();
    let err = bind_listener(&file, uid).unwrap_err();
    assert!(matches!(err, SocketError::NotASocket), "{err:?}");
    assert_eq!(fs::read(&file).unwrap(), b"not a socket");
    fs::remove_file(&file).unwrap();

    let real = parent.join("real.sock");
    let listener = bind_listener(&real, uid).unwrap();
    let link = parent.join("remote.sock");
    symlink(&real, &link).unwrap();
    let err = bind_listener(&link, uid).unwrap_err();
    assert!(matches!(err, SocketError::NotASocket), "{err:?}");
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(&real).unwrap().file_type().is_socket());
    drop(listener);
    fs::remove_file(&link).unwrap();

    // A stale socket "of another uid": the ownership check uses the uid handed in.
    let stale = bind_listener(&link, uid).unwrap();
    drop(stale);
    let err = bind_listener(&link, uid.wrapping_add(1)).unwrap_err();
    assert!(matches!(err, SocketError::NotASocket), "{err:?}");
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_socket());

    let dir_path = parent.join("dir.sock");
    fs::create_dir(&dir_path).unwrap();
    let err = bind_listener(&dir_path, uid).unwrap_err();
    assert!(matches!(err, SocketError::NotASocket), "{err:?}");
    assert!(dir_path.is_dir());
}

/// §2.7 / §4: a missing parent is an I/O failure (`EXIT_RUNTIME`, restarted).
#[tokio::test]
async fn test_rmc_bind_listener_in_a_missing_directory_is_io() {
    let tmp = tempfile::tempdir().unwrap();
    let uid = own_uid(tmp.path());
    let err = bind_listener(&tmp.path().join("absent").join("remote.sock"), uid).unwrap_err();
    assert!(
        matches!(err, SocketError::Io(std::io::ErrorKind::NotFound)),
        "{err:?}"
    );
}

/// §4: error messages are fixed text (the `Io` kind is the only payload).
#[test]
fn test_rmc_socket_error_messages() {
    assert_eq!(
        SocketError::NotADirectory.to_string(),
        "socket directory is not a directory or is a symlink"
    );
    assert_eq!(
        SocketError::WrongOwner.to_string(),
        "socket directory is not owned by the service user"
    );
    assert_eq!(
        SocketError::NotASocket.to_string(),
        "socket path exists and is not a socket"
    );
    assert!(SocketError::Io(std::io::ErrorKind::NotFound)
        .to_string()
        .starts_with("socket setup failed: "));
}

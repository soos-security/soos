//! Contractual tests for socket binding hardening (GitHub #199 DMN-08, GitHub #202 DMN-13).
//!
//! - A world-accessible `socket_mode` is refused before anything is bound.
//! - The permission fallback never uses a path-based `chmod` that follows symlinks: it
//!   pins the socket node with `O_PATH | O_NOFOLLOW`, checks it is a socket, and changes
//!   the mode through `/proc/self/fd/<n>`.
//! - The daemon never spawns `groupadd` (or any subprocess): a missing group is an error.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tempfile::tempdir;

use soos_daemon::config::SocketConfig;
use soos_daemon::error::DaemonError;
use soos_daemon::socket::{
    bind_socket, chmod_socket_node_nofollow, open_and_validate_directory, resolve_socket_group,
};

fn private_dir() -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o750))
        .expect("chmod tempdir");
    dir
}

fn daemon_source(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[tokio::test]
async fn test_bind_socket_refuses_world_writable_mode_before_bind() {
    let dir = private_dir();
    let socket_path = dir.path().join("daemon.sock");
    for mode in [0o666u32, 0o777, 0o664, 0o4660] {
        let config = SocketConfig {
            socket_path: socket_path.clone(),
            socket_dir: dir.path().to_path_buf(),
            socket_mode: mode,
            enforce_root_owner: false,
            socket_group: None,
        };
        match bind_socket(&config).await {
            Err(DaemonError::Config(msg)) => {
                assert!(
                    msg.contains("socket_mode"),
                    "error must name socket_mode: {msg}"
                );
            }
            Err(other) => panic!("mode {mode:o}: expected DaemonError::Config, got {other:?}"),
            Ok(_) => panic!("mode {mode:o} must be refused before bind"),
        }
        assert!(
            std::fs::symlink_metadata(&socket_path).is_err(),
            "mode {mode:o}: no socket node may exist after a refused bind"
        );
    }
}

#[tokio::test]
async fn test_bind_socket_accepts_0600_mode() {
    let dir = private_dir();
    let socket_path = dir.path().join("daemon.sock");
    let config = SocketConfig {
        socket_path: socket_path.clone(),
        socket_dir: dir.path().to_path_buf(),
        socket_mode: 0o600,
        enforce_root_owner: false,
        socket_group: None,
    };
    let (_listener, guard) = bind_socket(&config).await.expect("0600 must bind");
    let mode = std::fs::symlink_metadata(&socket_path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(mode, 0o600);
    drop(guard);
}

#[test]
fn test_chmod_fallback_sets_mode_on_socket_node() {
    let dir = private_dir();
    let socket_path = dir.path().join("daemon.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket_path).expect("bind");
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o644))
        .expect("pre-chmod");

    let dir_file = open_and_validate_directory(dir.path(), false).expect("open dir");
    chmod_socket_node_nofollow(&dir_file, Path::new("daemon.sock"), 0o660)
        .expect("fallback chmod on a socket node must succeed");

    let mode = std::fs::symlink_metadata(&socket_path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(mode, 0o660);
}

#[test]
fn test_chmod_fallback_never_follows_symlink() {
    let dir = private_dir();
    let victim = dir.path().join("victim");
    std::fs::write(&victim, b"x").expect("victim");
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    std::os::unix::fs::symlink(&victim, dir.path().join("daemon.sock")).expect("symlink");

    let dir_file = open_and_validate_directory(dir.path(), false).expect("open dir");
    let res = chmod_socket_node_nofollow(&dir_file, Path::new("daemon.sock"), 0o660);
    assert!(
        matches!(res, Err(DaemonError::SocketDirValidation(_))),
        "a symlink in place of the socket must be refused, got {res:?}"
    );
    let mode = std::fs::metadata(&victim)
        .expect("meta")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(mode, 0o600, "the symlink target must keep its mode");
}

#[test]
fn test_chmod_fallback_refuses_non_socket_node() {
    let dir = private_dir();
    let file = dir.path().join("daemon.sock");
    std::fs::write(&file, b"x").expect("file");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");

    let dir_file = open_and_validate_directory(dir.path(), false).expect("open dir");
    let res = chmod_socket_node_nofollow(&dir_file, Path::new("daemon.sock"), 0o660);
    assert!(
        matches!(res, Err(DaemonError::SocketDirValidation(_))),
        "a regular file in place of the socket must be refused, got {res:?}"
    );
    let mode = std::fs::metadata(&file).expect("meta").permissions().mode() & 0o7777;
    assert_eq!(mode, 0o600);
}

#[test]
fn test_chmod_fallback_refuses_path_with_directory_component() {
    let dir = private_dir();
    let dir_file = open_and_validate_directory(dir.path(), false).expect("open dir");
    for name in ["../daemon.sock", "sub/daemon.sock", "", ".", ".."] {
        let res = chmod_socket_node_nofollow(&dir_file, Path::new(name), 0o660);
        assert!(
            matches!(res, Err(DaemonError::SocketDirValidation(_))),
            "`{name}` must be refused as a socket node name, got {res:?}"
        );
    }
}

#[test]
fn test_missing_group_is_an_error() {
    let res = resolve_socket_group("soos-dmn13-group-that-does-not-exist");
    match res {
        Err(DaemonError::SocketDirValidation(msg)) => assert!(msg.contains("not found")),
        other => panic!("a missing group must be an error, got {other:?}"),
    }
}

#[test]
fn test_socket_module_never_spawns_subprocess_or_path_chmod() {
    let src = daemon_source("socket.rs");
    let code: String = src
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("//")
        })
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "std::process",
        "Command::new",
        "\"groupadd\"",
        "set_permissions(&config.socket_path",
    ] {
        assert!(
            !code.contains(forbidden),
            "socket.rs must not contain `{forbidden}` (DMN-13)"
        );
    }
}

#[test]
fn test_daemon_crate_never_spawns_groupadd() {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&src_dir).expect("read src") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let content = std::fs::read_to_string(&path).expect("read");
        let code: String = content
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("\"groupadd\"") && !code.contains("\"addgroup\""),
            "{} must not spawn group management tools (DMN-13)",
            path.display()
        );
    }
}

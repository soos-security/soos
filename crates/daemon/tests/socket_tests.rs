//! Contractual integration tests for socket lifecycle and directory validation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

use soos_daemon::config::SocketConfig;
use soos_daemon::error::DaemonError;
use soos_daemon::socket::{bind_socket, validate_directory};

#[tokio::test]
async fn test_socket_created_with_0660_permissions() {
    let dir = tempdir().expect("Failed to create tempdir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o750))
        .expect("Failed to set test dir permissions");
    let socket_path = dir.path().join("daemon.sock");

    let config = SocketConfig {
        socket_path: socket_path.clone(),
        socket_dir: dir.path().to_path_buf(),
        socket_mode: 0o660,
        enforce_root_owner: false, // In user-space test, non-root user runs test
    };

    let (_listener, guard) = bind_socket(&config).await.expect("Failed to bind socket");

    let meta = std::fs::symlink_metadata(&socket_path).expect("Failed to get socket metadata");
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o660,
        "Acceptance D1: Socket must be created with 0660 permissions, got {:o}",
        mode
    );

    drop(guard);
    assert!(
        !socket_path.exists(),
        "Socket file must be removed when SocketGuard is dropped"
    );
}

#[tokio::test]
async fn test_socket_directory_validation_rejects_symlink() {
    let dir = tempdir().expect("Failed to create tempdir");
    let real_dir = dir.path().join("real_dir");
    std::fs::create_dir(&real_dir).expect("Failed to create real dir");

    let symlink_dir = dir.path().join("symlink_dir");
    std::os::unix::fs::symlink(&real_dir, &symlink_dir).expect("Failed to create symlink");

    let err = validate_directory(&symlink_dir, false).expect_err("Symlink dir must be rejected");
    match err {
        DaemonError::SocketDirValidation(msg) => {
            assert!(
                msg.contains("symlink"),
                "Expected error mentioning symlink, got: {}",
                msg
            );
        }
        other => panic!("Expected SocketDirValidation error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_socket_directory_validation_rejects_world_writable() {
    let dir = tempdir().expect("Failed to create tempdir");
    let test_dir = dir.path().join("world_writable");
    std::fs::create_dir(&test_dir).expect("Failed to create dir");
    std::fs::set_permissions(&test_dir, std::fs::Permissions::from_mode(0o777))
        .expect("Failed to set 0777");

    let err =
        validate_directory(&test_dir, false).expect_err("World-writable dir must be rejected");
    match err {
        DaemonError::SocketDirValidation(msg) => {
            assert!(
                msg.contains("world-writable"),
                "Expected error mentioning world-writable, got: {}",
                msg
            );
        }
        other => panic!("Expected SocketDirValidation error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_socket_recreation_cleans_up_stale_socket() {
    let dir = tempdir().expect("Failed to create tempdir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o750))
        .expect("Failed to set test dir permissions");
    let socket_path = dir.path().join("daemon.sock");

    let config = SocketConfig {
        socket_path: socket_path.clone(),
        socket_dir: dir.path().to_path_buf(),
        socket_mode: 0o660,
        enforce_root_owner: false,
    };

    // First bind
    let (_listener1, guard1) = bind_socket(&config).await.expect("First bind failed");
    // Manually leak socket file via ManuallyDrop to simulate stale socket
    let _ = std::mem::ManuallyDrop::new(guard1);
    assert!(socket_path.exists());

    // Second bind on same path must clean up stale socket without error
    let (_listener2, guard2) = bind_socket(&config)
        .await
        .expect("Second bind on stale socket must succeed");
    assert!(socket_path.exists());

    drop(guard2);
    assert!(!socket_path.exists());
}

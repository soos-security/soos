//! Contractual integration tests for socket lifecycle and directory validation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
        socket_group: Some("soos".to_string()),
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
        socket_group: Some("soos".to_string()),
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

#[tokio::test]
async fn test_socket_binding_resists_symlink_race() {
    let dir = tempdir().expect("Failed to create tempdir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o750))
        .expect("Failed to set test dir permissions");

    let victim_file = dir.path().join("victim.txt");
    std::fs::write(&victim_file, b"sensitive_data").expect("Failed to create victim file");
    std::fs::set_permissions(&victim_file, std::fs::Permissions::from_mode(0o600))
        .expect("Failed to set victim permissions");

    let socket_path = dir.path().join("daemon.sock");
    // Create symlink pointing to victim file
    std::os::unix::fs::symlink(&victim_file, &socket_path).expect("Failed to create symlink");

    let config = SocketConfig {
        socket_path: socket_path.clone(),
        socket_dir: dir.path().to_path_buf(),
        socket_mode: 0o660,
        enforce_root_owner: false,
        socket_group: Some("soos".to_string()),
    };

    // Attempting to bind must fail closed because stale socket node is a symlink
    let err = bind_socket(&config)
        .await
        .expect_err("bind_socket must reject when socket path is a symlink");
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

    // Invariant: victim file MUST NOT be modified, truncated, deleted, or permissions changed
    assert!(victim_file.exists(), "Victim file must not be deleted");
    let content = std::fs::read(&victim_file).expect("Failed to read victim file");
    assert_eq!(
        content, b"sensitive_data",
        "Victim file content must remain unchanged"
    );
    let victim_meta = std::fs::metadata(&victim_file).expect("Failed to read victim file metadata");
    let victim_mode = victim_meta.permissions().mode() & 0o777;
    assert_eq!(
        victim_mode, 0o600,
        "Victim file permissions must remain untouched (0600)"
    );

    // Broken symlink test: symlink pointing to nonexistent target
    let broken_sock_path = dir.path().join("broken.sock");
    std::os::unix::fs::symlink(dir.path().join("does_not_exist"), &broken_sock_path)
        .expect("Failed to create broken symlink");

    let config_broken = SocketConfig {
        socket_path: broken_sock_path.clone(),
        socket_dir: dir.path().to_path_buf(),
        socket_mode: 0o660,
        enforce_root_owner: false,
        socket_group: Some("soos".to_string()),
    };

    let err_broken = bind_socket(&config_broken)
        .await
        .expect_err("bind_socket must reject broken symlinks");
    match err_broken {
        DaemonError::SocketDirValidation(msg) => {
            assert!(
                msg.contains("symlink"),
                "Expected error mentioning symlink for broken symlink, got: {}",
                msg
            );
        }
        other => panic!("Expected SocketDirValidation error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_socket_ownership_root_soos() {
    let dir = tempdir().expect("Failed to create tempdir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o750))
        .expect("Failed to set test dir permissions");
    let socket_path = dir.path().join("daemon.sock");

    let is_root = nix::unistd::geteuid().is_root();

    if is_root {
        // When running as root, verify exact root:soos ownership
        let config = SocketConfig {
            socket_path: socket_path.clone(),
            socket_dir: dir.path().to_path_buf(),
            socket_mode: 0o660,
            enforce_root_owner: true,
            socket_group: Some("soos".to_string()),
        };

        let (_listener, guard) = bind_socket(&config)
            .await
            .expect("bind_socket must succeed as root");

        let meta = std::fs::symlink_metadata(&socket_path).expect("metadata");
        assert_eq!(
            meta.uid(),
            0,
            "Acceptance 20.2: Socket must be owned by root (uid 0)"
        );

        if let Ok(Some(soos_grp)) = nix::unistd::Group::from_name("soos") {
            assert_eq!(
                meta.gid(),
                soos_grp.gid.as_raw(),
                "Acceptance 20.2: Socket GID must match soos group GID"
            );
        }

        drop(guard);
    } else {
        // When running in unprivileged test environment:
        // 1. Verify default SocketConfig specifies "soos" group
        let default_config = SocketConfig::default();
        assert_eq!(
            default_config.socket_group.as_deref(),
            Some("soos"),
            "Default SocketConfig must specify 'soos' group"
        );

        // 2. Resolve current user's primary group name to test group application
        let current_gid = nix::unistd::getgid();
        let current_grp = nix::unistd::Group::from_gid(current_gid)
            .expect("Lookup current group")
            .expect("Group must exist");

        let config = SocketConfig {
            socket_path: socket_path.clone(),
            socket_dir: dir.path().to_path_buf(),
            socket_mode: 0o660,
            enforce_root_owner: false,
            socket_group: Some(current_grp.name.clone()),
        };

        let (_listener, guard) = bind_socket(&config)
            .await
            .expect("bind_socket must succeed in unprivileged test mode");

        let meta = std::fs::symlink_metadata(&socket_path).expect("metadata");
        assert_eq!(
            meta.gid(),
            current_gid.as_raw(),
            "Acceptance 20.2: Socket GID must be set to specified target group"
        );
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o660, "Socket mode must remain 0660");

        drop(guard);
    }
}

//! Contractual integration tests for SO_PEERCRED extraction and verification.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use tempfile::tempdir;
use tokio::net::{UnixListener, UnixStream};

use soos_daemon::error::DaemonError;
use soos_daemon::peercred::{get_peer_credentials, verify_peer_credentials, PeerCredentials};

#[tokio::test]
async fn test_peercred_extraction_nominal() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("test_cred.sock");

    let listener = UnixListener::bind(&sock_path).expect("Failed to bind test listener");

    let client_task = tokio::spawn({
        let path = sock_path.clone();
        async move {
            UnixStream::connect(path)
                .await
                .expect("Client connect failed")
        }
    });

    let (server_stream, _addr) = listener.accept().await.expect("Accept failed");
    let _client_stream = client_task.await.expect("Join failed");

    let creds = get_peer_credentials(&server_stream).expect("Failed to extract peer credentials");

    let current_uid = nix::unistd::getuid().as_raw();
    let current_gid = nix::unistd::getgid().as_raw();
    let current_pid = i32::try_from(std::process::id()).unwrap_or(0);

    assert_eq!(
        creds.uid, current_uid,
        "Extracted peer UID must match caller UID"
    );
    assert_eq!(
        creds.gid, current_gid,
        "Extracted peer GID must match caller GID"
    );
    assert_eq!(
        creds.pid,
        Some(current_pid),
        "Extracted peer PID must match caller PID"
    );
}

#[test]
fn test_peercred_verification_allows_matching_uid() {
    let creds = PeerCredentials {
        uid: 1000,
        gid: 1000,
        pid: Some(1234),
    };

    let result = verify_peer_credentials(&creds, 1000);
    assert!(result.is_ok(), "Matching peer UID must be permitted");
}

#[test]
fn test_peercred_verification_allows_root_caller() {
    // Root caller (e.g. display manager or sudo) authenticating on behalf of target user
    let creds = PeerCredentials {
        uid: 0,
        gid: 0,
        pid: Some(1),
    };

    let result = verify_peer_credentials(&creds, 1000);
    assert!(
        result.is_ok(),
        "Root caller (UID 0) must be permitted for any target UID"
    );
}

#[test]
fn test_peercred_verification_rejects_mismatched_uid() {
    // Acceptance D2: Spoofed UID test rejects mismatched peer
    let creds = PeerCredentials {
        uid: 1001,
        gid: 1001,
        pid: Some(5678),
    };

    let err = verify_peer_credentials(&creds, 1000).expect_err("Mismatched UID must be rejected");
    match err {
        DaemonError::UidMismatch {
            peer_uid,
            requested_uid,
        } => {
            assert_eq!(peer_uid, 1001);
            assert_eq!(requested_uid, 1000);
        }
        other => panic!("Expected UidMismatch error, got: {:?}", other),
    }
}

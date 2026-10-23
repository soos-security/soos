//! Contract of the system push sender socket path (ADR "[2026-10-23] Push Sender as a
//! Sandboxed System Service"; spec `AI/architect_spec_push_sender_system_service.md` §3;
//! matrix PSS3).
//!
//! - `PUSH_SYSTEM_SOCKET_ROOT` is `/run/soos-push`, the directory of
//!   `packaging/soos-push-sender@.socket`.
//! - `system_push_socket_path(uid)` is `<root>/<decimal uid>/push.sock`, the file name being
//!   `PUSH_SOCKET_FILE_NAME`.
//! - `PUSH_SYSTEM_INSTANCE_ARG` is the argument of the unit's `ExecStart=`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::{Path, PathBuf};

use soos_push_protocol::{
    system_push_socket_path, PUSH_SOCKET_FILE_NAME, PUSH_SYSTEM_INSTANCE_ARG,
    PUSH_SYSTEM_SOCKET_ROOT,
};

/// PSS3: the constants and the per-uid path.
#[test]
fn test_pss3_system_socket_path() {
    assert_eq!(PUSH_SYSTEM_SOCKET_ROOT, "/run/soos-push");
    assert_eq!(PUSH_SYSTEM_INSTANCE_ARG, "--system-instance");
    assert_eq!(
        system_push_socket_path(1000),
        PathBuf::from("/run/soos-push/1000/push.sock")
    );
    assert_eq!(
        system_push_socket_path(1),
        PathBuf::from("/run/soos-push/1/push.sock")
    );
    assert_eq!(
        system_push_socket_path(4_294_967_294),
        PathBuf::from("/run/soos-push/4294967294/push.sock")
    );
    let path = system_push_socket_path(60_001);
    assert!(path.is_absolute());
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        Some(PUSH_SOCKET_FILE_NAME)
    );
    assert_eq!(
        path.parent().and_then(Path::parent),
        Some(Path::new(PUSH_SYSTEM_SOCKET_ROOT))
    );
    // Distinct uids never share a socket.
    assert_ne!(system_push_socket_path(1000), system_push_socket_path(1001));
}

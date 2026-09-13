//! Kernel-enforced peer credential extraction and verification via `SO_PEERCRED`.

use nix::sys::socket::{getsockopt, sockopt::PeerCredentials as NixPeerCreds};
use tokio::net::UnixStream;
use tracing::debug;

use crate::error::DaemonError;

/// Extracted kernel credentials for a connecting Unix socket peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCredentials {
    /// Effective user ID of the connecting process.
    pub uid: u32,
    /// Effective group ID of the connecting process.
    pub gid: u32,
    /// Process ID of the connecting process (if available from kernel).
    pub pid: Option<i32>,
}

/// Queries `getsockopt(SO_PEERCRED)` on the incoming Unix domain stream.
pub fn get_peer_credentials(stream: &UnixStream) -> Result<PeerCredentials, DaemonError> {
    let ucred = getsockopt(stream, NixPeerCreds).map_err(|e| {
        DaemonError::PeerCredExtraction(format!("getsockopt(SO_PEERCRED) failed: {}", e))
    })?;

    let uid = ucred.uid();
    let gid = ucred.gid();
    let pid = ucred.pid();

    debug!(
        peer_uid = uid,
        peer_gid = gid,
        peer_pid = pid,
        "Retrieved peer info via SO_PEERCRED"
    );

    Ok(PeerCredentials {
        uid,
        gid,
        pid: Some(pid),
    })
}

/// Verifies that peer credentials permit authenticating as `requested_uid`.
///
/// Security rules (ARCHITECTURE.md §4):
/// - A peer is permitted if `peer.uid == requested_uid`.
/// - A root caller (`peer.uid == 0`, e.g. display manager `gdm` or `sudo`) is
///   permitted to authenticate on behalf of any requested UID.
/// - Any other mismatched peer is rejected with `DaemonError::UidMismatch`.
pub fn verify_peer_credentials(
    peer: &PeerCredentials,
    requested_uid: u32,
) -> Result<(), DaemonError> {
    if peer.uid == 0 || peer.uid == requested_uid {
        Ok(())
    } else {
        Err(DaemonError::UidMismatch {
            peer_uid: peer.uid,
            requested_uid,
        })
    }
}

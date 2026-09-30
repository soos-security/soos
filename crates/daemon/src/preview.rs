//! Authorization policy for `RequestKind::PreviewFrame` (GUI diagnostic camera preview).
//!
//! Camera frames are biometric data (ARCHITECTURE.md §1). They may leave the daemon only
//! towards a peer that is either root or explicitly listed by the administrator in the
//! `[preview]` section of `daemon.toml`, which is disabled by default (fail-closed).
//! The dispatcher additionally requires an active logind session for unprivileged peers
//! and applies a per-UID rate limit to every preview request, root included.

use soos_policy::RateLimitConfig;
use thiserror::Error;

use crate::error::DaemonError;

pub use crate::preview_image::{
    preview_image_for_frame, wire_format_code, PreviewImage, MAX_PREVIEW_PIXEL_BYTES,
    MAX_PREVIEW_WIDTH, PREVIEW_FORMAT_EMPTY, PREVIEW_HEADER_RESERVE,
};

/// Maximum number of entries accepted in `[preview] allowed_uids` (bounded configuration).
pub const MAX_PREVIEW_ALLOWED_UIDS: usize = 64;

/// Default preview request quota per peer UID and per second.
///
/// The GUI polls at roughly 30 requests per second; 40 leaves headroom for jitter while a
/// misbehaving client is still throttled well below the daemon's capacity.
pub const DEFAULT_PREVIEW_MAX_REQUESTS_PER_SEC: u32 = 40;

/// Sliding window used by the preview rate limiter (one second).
pub const PREVIEW_RATE_WINDOW_NS: u64 = 1_000_000_000;

/// Maximum number of distinct peer UIDs tracked by the preview rate limiter.
pub const PREVIEW_RATE_MAX_TRACKED_UIDS: usize = 64;

/// Configuration of the `[preview]` section of `daemon.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewConfig {
    /// Whether unprivileged peers listed in `allowed_uids` may request preview frames.
    /// `false` (default): only a root peer (UID 0) is served.
    pub enabled: bool,
    /// Explicit allow-list of unprivileged peer UIDs. Empty by default; bounded by
    /// [`MAX_PREVIEW_ALLOWED_UIDS`].
    pub allowed_uids: Vec<u32>,
    /// Preview requests permitted per peer UID within [`PREVIEW_RATE_WINDOW_NS`].
    /// `0` denies every preview request (root included).
    pub max_requests_per_sec: u32,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_uids: Vec::new(),
            max_requests_per_sec: DEFAULT_PREVIEW_MAX_REQUESTS_PER_SEC,
        }
    }
}

impl PreviewConfig {
    /// Validates configuration bounds.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::Config`] when `allowed_uids` exceeds [`MAX_PREVIEW_ALLOWED_UIDS`].
    pub fn validate(&self) -> Result<(), DaemonError> {
        if self.allowed_uids.len() > MAX_PREVIEW_ALLOWED_UIDS {
            return Err(DaemonError::Config(format!(
                "[preview] allowed_uids has {} entries (maximum {})",
                self.allowed_uids.len(),
                MAX_PREVIEW_ALLOWED_UIDS
            )));
        }
        Ok(())
    }

    /// Rate limiter parameters derived from this configuration.
    #[must_use]
    pub const fn rate_limit_config(&self) -> RateLimitConfig {
        RateLimitConfig::new_with_capacity(
            self.max_requests_per_sec,
            PREVIEW_RATE_WINDOW_NS,
            PREVIEW_RATE_MAX_TRACKED_UIDS,
        )
    }
}

/// Reason why a preview request was refused (internal observability only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PreviewDenied {
    /// The peer asserted a `uid_hint` different from its kernel-verified UID.
    #[error("peer UID {peer_uid} does not match requested UID {requested_uid}")]
    UidMismatch {
        /// Kernel `SO_PEERCRED` UID of the peer.
        peer_uid: u32,
        /// `uid_hint` declared in the request payload.
        requested_uid: u32,
    },
    /// Preview is disabled for unprivileged peers (`[preview] enabled = false`).
    #[error("preview is disabled for unprivileged peer UID {peer_uid}")]
    Disabled {
        /// Kernel `SO_PEERCRED` UID of the peer.
        peer_uid: u32,
    },
    /// The peer UID is not present in `[preview] allowed_uids`.
    #[error("peer UID {peer_uid} is not in the preview allow-list")]
    NotAllowed {
        /// Kernel `SO_PEERCRED` UID of the peer.
        peer_uid: u32,
    },
}

/// Decides whether `peer_uid` (kernel verified) may receive preview frames.
///
/// Rules:
/// - A root peer (`peer_uid == 0`) is always authorized, whatever `requested_uid` says.
/// - Any other peer must declare its own UID (`peer_uid == requested_uid`), preview must be
///   `enabled`, and the peer UID must be listed in `allowed_uids`.
///
/// # Errors
///
/// Returns the first failing rule as a [`PreviewDenied`] variant.
pub fn authorize_preview(
    config: &PreviewConfig,
    peer_uid: u32,
    requested_uid: u32,
) -> Result<(), PreviewDenied> {
    if peer_uid == 0 {
        return Ok(());
    }
    if peer_uid != requested_uid {
        return Err(PreviewDenied::UidMismatch {
            peer_uid,
            requested_uid,
        });
    }
    if !config.enabled {
        return Err(PreviewDenied::Disabled { peer_uid });
    }
    if !config.allowed_uids.contains(&peer_uid) {
        return Err(PreviewDenied::NotAllowed { peer_uid });
    }
    Ok(())
}

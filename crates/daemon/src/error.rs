//! Daemon error definitions.

use thiserror::Error;

/// Comprehensive error type for daemon operations.
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Underlying I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Socket directory validation error (e.g. symlink, world-writable, non-root).
    #[error("Socket directory validation failed: {0}")]
    SocketDirValidation(String),

    /// Peer credential extraction error.
    #[error("Failed to extract peer credentials: {0}")]
    PeerCredExtraction(String),

    /// Peer UID does not match requested target UID.
    #[error("UID mismatch: peer UID {peer_uid} does not match requested UID {requested_uid}")]
    UidMismatch {
        /// Real UID of peer connection obtained via `SO_PEERCRED`.
        peer_uid: u32,
        /// Target UID asserted in request payload.
        requested_uid: u32,
    },

    /// Wire codec error.
    #[error("Codec error: {0}")]
    Codec(#[from] soos_protocol::codec::CodecError),

    /// Protocol framing or version error.
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Connection operation timed out.
    #[error("Connection timed out")]
    Timeout,

    /// Maximum concurrent connections reached.
    #[error("Max concurrent connections reached")]
    ConcurrencyLimitReached,

    /// Declared payload size exceeds maximum boundary.
    #[error("Payload size {size} exceeds maximum {max} bytes")]
    OversizedPayload {
        /// Declared size in frame.
        size: usize,
        /// Maximum allowable size.
        max: usize,
    },

    /// Biometric store error.
    #[error("Biometric store error: {0}")]
    BiometricStore(#[from] soos_biometric_store::BiometricStoreError),

    /// Evidence store error.
    #[error("Evidence store error: {0}")]
    EvidenceStore(#[from] soos_evidence_store::EvidenceStoreError),

    /// Vision pipeline error.
    #[error("Vision pipeline error: {0}")]
    Vision(#[from] soos_vision::VisionError),

    /// Camera capture error.
    #[error("Camera error: {0}")]
    Camera(#[from] soos_camera_v4l::CameraError),

    /// Monotonic clock error.
    #[error("Clock error: {0}")]
    Clock(String),
}

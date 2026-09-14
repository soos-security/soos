//! Error types for the evidence-store crate.

use thiserror::Error;

/// Error variants encountered during evidence persistence, encryption, or rotation.
#[derive(Debug, Error)]
pub enum EvidenceStoreError {
    /// Evidence storage is disabled in configuration.
    #[error("Evidence storage is disabled in configuration")]
    Disabled,

    /// Daily capture limit exceeded for a specific UID.
    #[error("Daily capture cap exceeded for UID {uid} on {date}: reached limit of {cap}")]
    DailyCapExceeded {
        /// User identifier.
        uid: u32,
        /// Configured daily cap.
        cap: u32,
        /// Date string in `YYYY-MM-DD` format.
        date: String,
    },

    /// Cryptographic encryption or decryption failure.
    #[error("Cryptographic operation failed: {0}")]
    Crypto(String),

    /// Key management or loading error.
    #[error("Master key error: {0}")]
    KeyError(String),

    /// Corrupted ciphertext, invalid magic header, or truncated payload.
    #[error("Corrupt or invalid evidence payload: {0}")]
    CorruptPayload(String),

    /// Filesystem or I/O failure.
    #[error("Filesystem I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization or deserialization failure.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Invalid date format string.
    #[error("Invalid date format: {0}")]
    InvalidDate(String),
}

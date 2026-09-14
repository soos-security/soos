//! Error definitions for the biometric storage subsystem.

use thiserror::Error;

/// Enumeration of errors that can occur during biometric store operations.
#[derive(Debug, Error)]
pub enum BiometricStoreError {
    /// Standard I/O error occurred during file or directory operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Cryptographic operation failed (encryption, decryption, or authentication tag verification).
    #[error("Cryptographic operation failed: {0}")]
    Crypto(String),

    /// CBOR serialization or deserialization failed.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Template metadata validation failed (e.g., empty model name, NaN floats).
    #[error("Invalid template metadata: {0}")]
    InvalidMetadata(String),

    /// Storage path or file name is invalid.
    #[error("Invalid store path: {0}")]
    InvalidPath(String),

    /// Template file on disk is corrupt or has invalid header/size.
    #[error("Corrupted template file: {0}")]
    CorruptFile(String),

    /// Master key generation, derivation, or loading error.
    #[error("Key error: {0}")]
    KeyError(String),
}

//! Typed errors for the `soos-inference-ort` crate.

use std::path::PathBuf;
use thiserror::Error;

/// Domain errors encountered during model manifest verification, loading, or inference.
#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("I/O error reading manifest at {path:?}: {source}")]
    ManifestIo {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to parse manifest TOML: {0}")]
    ManifestParse(#[from] toml::de::Error),

    #[error("Model '{id}' not found at path {path:?}")]
    ModelNotFound { id: String, path: PathBuf },

    #[error("I/O error accessing model '{id}' at {path:?}: {source}")]
    ModelIo {
        id: String,
        path: PathBuf,
        source: std::io::Error,
    },

    #[error(
        "SHA-256 checksum mismatch for model '{id}': expected {expected}, calculated {actual}"
    )]
    ChecksumMismatch {
        id: String,
        expected: String,
        actual: String,
    },

    #[error("Model '{id}' is corrupted or unreadable: {message}")]
    ModelCorrupted { id: String, message: String },

    #[error("ONNX Runtime error: {0}")]
    Ort(String),

    #[error("Invalid image input dimensions: expected {expected:?}, got {actual:?}")]
    InvalidDimensions {
        expected: (u32, u32),
        actual: (u32, u32),
    },

    #[error("Invalid raw pixel buffer size: expected {expected} bytes, got {actual} bytes")]
    InvalidBufferSize { expected: usize, actual: usize },

    #[error("Invalid input data: {0}")]
    InvalidInput(String),

    #[error("Face detection failed: {0}")]
    DetectionFailed(String),

    #[error("Landmark extraction failed: {0}")]
    LandmarkFailed(String),

    #[error("Embedding extraction failed: {0}")]
    EmbeddingFailed(String),

    #[error("Vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("Tensor shape or layout error: {0}")]
    TensorError(String),
}

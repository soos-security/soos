//! Error types for the vision preprocessing and matching pipeline.

use thiserror::Error;

use soos_camera_v4l::PixelFormat;
use soos_inference_ort::InferenceError;

/// Errors that can occur during image preprocessing, alignment, and facial matching.
#[derive(Debug, Error)]
pub enum VisionError {
    /// The input camera pixel format is not supported for color conversion.
    #[error("Unsupported camera pixel format: {0:?}")]
    UnsupportedPixelFormat(PixelFormat),

    /// Color conversion failed due to malformed or corrupt buffer data.
    #[error("Color conversion failed: {0}")]
    ColorConversionFailed(String),

    /// Image dimensions are zero or would cause an arithmetic overflow.
    #[error("Invalid image dimensions: width={width}, height={height}")]
    InvalidDimensions { width: u32, height: u32 },

    /// Buffer size does not match the expected byte count for dimensions and format.
    #[error("Invalid buffer size: expected {expected} bytes, got {actual} bytes")]
    InvalidBufferSize { expected: usize, actual: usize },

    /// Face detector found zero faces in the input frame.
    #[error("No face detected in frame")]
    NoFaceDetected,

    /// Face detector found multiple faces in the input frame (security invariant violation).
    #[error("Multiple faces detected in frame ({count} faces)")]
    MultipleFacesDetected { count: usize },

    /// Detected face confidence score is below the required threshold.
    #[error("Face detection confidence {confidence} is below required threshold {min_confidence}")]
    FaceBelowConfidence {
        confidence: f32,
        min_confidence: f32,
    },

    /// Face detection is missing required 5-point facial landmarks.
    #[error("Face detection is missing required 5-point landmarks")]
    MissingLandmarks,

    /// 5-point facial landmark alignment transformation failed.
    #[error("Landmark alignment failed: {0}")]
    AlignmentFailed(String),

    /// Presentation attack detected (spoof attempt: printed photo, screen replay, or mask).
    #[error(
        "Presentation attack detected: liveness score {score:.3} is below threshold {threshold:.3}"
    )]
    PadFailed { score: f32, threshold: f32 },

    /// Embedding dimension mismatch during cosine similarity matching.
    #[error("Embedding dimension mismatch: expected {expected}, actual {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    /// Degenerate embedding vector with near-zero Euclidean norm.
    #[error("Degenerate embedding vector: {0}")]
    DegenerateEmbedding(String),

    /// Error propagated from underlying ONNX Runtime neural inference.
    #[error("Inference failure: {0}")]
    Inference(#[from] InferenceError),
}

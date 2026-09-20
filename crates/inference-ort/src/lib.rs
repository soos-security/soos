//! `soos-inference-ort` — Isolated ONNX Runtime CPU inference engine for Linux Biometric PAM.
//!
//! Provides:
//! - Cryptographic manifest attestation and model SHA-256 verification (`manifest`, `registry`)
//! - UltraFace Slim 320 face detection and deterministic Rust NMS (`detector`)
//! - 5-point facial landmark estimation (`landmarks`)
//! - MobileFaceNet ArcFace feature extraction with L2-normalized embeddings (`embedding`)
//! - Hardware-free deterministic simulation mocks (`mock`)

#![forbid(unsafe_code)]

pub mod detector;
pub mod embedding;
pub mod error;
pub mod landmarks;
pub mod manifest;
pub mod mock;
pub mod pad;
pub mod registry;

pub use detector::{
    letterbox_pad, nms, unproject, BoundingBox, FaceDetection, FaceDetector, OrtFaceDetector,
    OrtScrfdDetector,
};
pub use embedding::{BiometricEmbedding, EmbeddingExtractor, OrtEmbeddingExtractor};
pub use error::InferenceError;
pub use landmarks::{FaceLandmarks, LandmarkDetector, Point2f};
pub use manifest::{ManifestHeader, ModelManifest, ModelMetadata};
pub use mock::{MockEmbeddingExtractor, MockFaceDetector, MockLandmarkDetector, MockPadDetector};
pub use pad::{AttackType, OrtPadDetector, PadDetector, PadResult};
pub use registry::{ModelRegistry, RegistryConfig};

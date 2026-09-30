//! `soos-inference-ort` — Isolated ONNX Runtime CPU inference engine for Linux Biometric PAM.
//!
//! Provides:
//! - Cryptographic manifest attestation and model SHA-256 verification (`manifest`, `registry`)
//! - Manifest I/O shape attestation of every loaded session (`manifest`, `registry`)
//! - SCRFD 500M KPS face detection with 5-point landmarks and deterministic Rust NMS (`detector`)
//! - 5-point facial landmark types (`landmarks`)
//! - Single bilinear letterbox implementation with integer offsets (`letterbox`)
//! - ArcFace ResNet34 (NHWC, tf2onnx) 512D feature extraction with L2-normalized embeddings
//!   (`embedding`; manifest id `arcface_w600k_mbf` is a historical name)
//! - MiniFASNetV2 presentation attack detection (`pad`)
//! - Hardware-free deterministic simulation mocks (`mock`)
//! - In-place wiping of ORT-owned output tensors (`outputs`)

#![forbid(unsafe_code)]

pub mod detector;
pub mod embedding;
pub mod error;
pub mod landmarks;
pub mod letterbox;
pub mod manifest;
pub mod mock;
pub mod outputs;
pub mod pad;
pub mod registry;

pub use detector::{
    letterbox_pad, letterbox_pad_into, nms, unproject, BoundingBox, FaceDetection, FaceDetector,
    OrtScrfdDetector, ScoreActivation,
};
pub use embedding::{BiometricEmbedding, EmbeddingExtractor, OrtEmbeddingExtractor};
pub use error::InferenceError;
pub use landmarks::{FaceLandmarks, Point2f};
pub use letterbox::{letterbox_bilinear, letterbox_geometry, LetterboxGeometry};
pub use manifest::{ManifestHeader, ModelManifest, ModelMetadata, TensorLayout};
pub use mock::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
pub use outputs::ZeroizingOutputs;
pub use pad::{AttackType, OrtPadDetector, PadDetector, PadResult, PadSelfTestReport};
pub use registry::{
    default_intra_threads, ModelRegistry, RegistryConfig, SharedSession, DEFAULT_MAX_INTRA_THREADS,
    MAX_INTRA_THREADS,
};

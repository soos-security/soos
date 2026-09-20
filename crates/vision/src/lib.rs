//! `soos-vision` — Local facial verification preprocessing, affine alignment,
//! and cosine matching pipeline for Linux Biometric PAM.
//!
//! Provides:
//! - Pure Rust color conversions for V4L2 frames (`YUYV 4:2:2`, `Grey`, `RGB24`, `MJPEG`)
//! - Standard 5-point facial landmark affine alignment to 112x112 crops (ArcFace / InsightFace)
//! - Cosine similarity matcher with zero-norm and dimension protections
//! - `VisionPipeline` orchestrator enforcing the single-face security invariant

#![forbid(unsafe_code)]

pub mod align;
pub mod color;
pub mod crop;
pub mod error;
pub mod letterbox;
pub mod matcher;
pub mod pipeline;

pub use align::{align_face_112, TARGET_LANDMARKS_112};
pub use color::convert_to_rgb;
pub use crop::{crop_and_resize, expand_bbox_for_pad};
pub use error::VisionError;
pub use letterbox::{letterbox_params, letterbox_resize, LetterboxParams};
pub use matcher::{cosine_similarity, match_embeddings, MatchResult};
pub use pipeline::{PipelineOutput, VerificationOutcome, VisionPipeline, VisionPipelineConfig};

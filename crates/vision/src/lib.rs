//! `soos-vision` — Local facial verification preprocessing, affine alignment,
//! and cosine matching pipeline for Linux Biometric PAM.
//!
//! Provides:
//! - Pure Rust color conversions for V4L2 frames (`YUYV 4:2:2`, `Grey`, `RGB24`, `MJPEG`)
//! - Standard 5-point facial landmark affine alignment to 112x112 crops (ArcFace / InsightFace)
//! - Cosine similarity matcher with zero-norm and dimension protections
//! - `VisionPipeline` orchestrator enforcing the single-face security invariant
//! - Format-aware PAD policy: fail-closed IR gate and stricter threshold for `Grey` frames
//! - Pre-PAD face quality gate: minimum face size and optional PAD crop sharpness floor

#![forbid(unsafe_code)]

pub mod align;
pub mod color;
pub mod crop;
pub mod error;
pub mod ir_liveness;
pub mod letterbox;
pub mod matcher;
pub mod pad_fusion;
pub mod pipeline;
pub mod pose;
pub mod quality;

pub use align::{align_face_112, TARGET_LANDMARKS_112};
pub use color::{convert_to_rgb, convert_to_rgb_cow};
pub use crop::{
    crop_and_resize, crop_pad_context, expand_bbox_for_pad, pad_crop_window, PadCropWindow,
};
pub use error::VisionError;
pub use ir_liveness::{
    evaluate_ir_gate, ir_crop_statistics, IrCropStatistics, IrGateRejection, PadInputModality,
    DEFAULT_IR_PAD_THRESHOLD, IR_MAX_MEAN_LUMA, IR_MIN_LUMA_STDDEV, IR_MIN_MEAN_LUMA,
    IR_MIN_TEXTURE_ENERGY,
};
pub use letterbox::{letterbox_params, letterbox_resize, LetterboxParams};
pub use matcher::{cosine_similarity, match_embeddings, MatchResult};
pub use pad_fusion::fuse_pad_results;
pub use pipeline::{
    PipelineOutput, VerificationOutcome, VisionAnalysis, VisionPipeline, VisionPipelineConfig,
    DEFAULT_MATCH_THRESHOLD, DEFAULT_MIN_FACE_CONFIDENCE, DEFAULT_NMS_IOU_THRESHOLD,
    DEFAULT_PAD_THRESHOLD, MAX_PAD_ENSEMBLE_MODELS,
};
pub use pose::{compute_face_geometry, estimate_head_pose, FaceGeometry, HeadPose};
pub use quality::{
    laplacian_variance, FaceQualityRejection, DEFAULT_MIN_FACE_WIDTH_PX,
    DEFAULT_MIN_PAD_CROP_SHARPNESS,
};

//! Shared application state and telemetry data structures for the SOOS GUI.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "Telemetry calculation and GUI coordinate conversions"
)]

use soos_enrollment_cli::guided_enrollment::{EnrollmentStepFeedback, GuidedEnrollmentSession};
use soos_enrollment_cli::service::EnrolledUserSummary;
use soos_inference_ort::{FaceDetection, PadResult};
use soos_vision::pose::HeadPose;

/// Captured frame telemetry and neural model outputs ready for UI rendering.
#[derive(Debug, Clone)]
pub struct LatestFrameData {
    /// RGB24 raw pixel buffer.
    pub rgb: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Raw detections from SCRFD face detector.
    pub detections: Vec<FaceDetection>,
    /// Presentation attack detection result.
    pub pad_result: Option<PadResult>,
    /// Estimated 3D head rotation pose.
    pub pose: Option<HeadPose>,
    /// 112x112 aligned face crop for preview.
    pub aligned_crop: Option<Vec<u8>>,
    /// Instantaneous pipeline latency in milliseconds.
    pub pipeline_latency_ms: f64,
    /// Detector inference latency in milliseconds.
    pub det_latency_ms: f64,
    /// PAD inference latency in milliseconds.
    pub pad_latency_ms: f64,
    /// Current video frame rate.
    pub fps: f32,
    /// Monotonic frame sequence counter.
    pub sequence: u64,
}

/// GUI state for guided multi-step face enrollment.
#[derive(Debug, Clone)]
pub struct EnrollmentGuiState {
    /// Active guided enrollment session tracking steps and samples.
    pub session: Option<GuidedEnrollmentSession>,
    /// Whether enrollment is currently actively sampling.
    pub is_active: bool,
    /// Target username to enroll.
    pub target_username: String,
    /// Target numeric UID.
    pub target_uid: u32,
    /// Last feedback received from state machine.
    pub last_feedback: Option<EnrollmentStepFeedback>,
    /// Generated composite embedding ready for encryption and storage.
    pub composite_embedding: Option<Vec<f32>>,
    /// Status or error banner message.
    pub status_message: Option<(String, bool)>, // (message, is_error)
}

impl Default for EnrollmentGuiState {
    fn default() -> Self {
        let current_uid = nix::unistd::getuid().as_raw();
        let username = match nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(current_uid)) {
            Ok(Some(u)) => u.name,
            _ => "user".to_string(),
        };

        Self {
            session: None,
            is_active: false,
            target_username: username,
            target_uid: current_uid,
            last_feedback: None,
            composite_embedding: None,
            status_message: None,
        }
    }
}

/// GUI state for biometric profiles and templates management.
#[derive(Debug, Clone, Default)]
pub struct ProfilesGuiState {
    /// Enrolled user template summaries.
    pub profiles: Vec<EnrolledUserSummary>,
    /// Currently selected UID for inspection or deletion.
    pub selected_uid: Option<u32>,
    /// Real-time live match comparison score against selected profile.
    pub live_match_score: Option<f32>,
    /// Status or confirmation message.
    pub status_message: Option<(String, bool)>,
    /// Confirmation dialog state for deletion.
    pub confirm_delete_uid: Option<u32>,
}

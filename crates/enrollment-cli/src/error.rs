//! Typed error enumerations for `soos-enrollment-cli`.

use thiserror::Error;

/// Error types encountered during enrollment, verification, deletion, or listing.
#[derive(Debug, Error)]
pub enum EnrollmentCliError {
    #[error("Root privileges (EUID 0) are required for this operation")]
    RootRequired,

    #[error("User '{0}' not found in system user database")]
    UserNotFound(String),

    #[error(
        "No target user given and no non-root invoking user found (SUDO_UID/PKEXEC_UID); \
         pass --username <NAME> or --uid <UID> (use --uid 0 to enroll root explicitly)"
    )]
    TargetUserRequired,

    #[error("Malformed {0} environment variable; pass --username <NAME> or --uid <UID>")]
    InvalidInvokerUid(String),

    #[error("User ID {0} is already enrolled; use --yes or confirm to overwrite")]
    AlreadyEnrolled(u32),

    #[error("No biometric template enrolled for user ID {0}")]
    NotEnrolled(u32),

    #[error("Operation cancelled by user")]
    Cancelled,

    #[error("No face detected in camera frame")]
    NoFaceDetected,

    #[error("Multiple faces detected ({count} faces); expected exactly 1")]
    MultipleFacesDetected { count: usize },

    #[error(
        "Face detection confidence {confidence:.2} is below required threshold {min_confidence:.2}"
    )]
    FaceBelowConfidence {
        confidence: f32,
        min_confidence: f32,
    },

    #[error("All {evaluated} captured frames failed quality criteria ({valid} valid candidates)")]
    LowQualityFrames { evaluated: usize, valid: usize },

    #[error("Camera error: {0}")]
    Camera(#[from] soos_camera_v4l::CameraError),

    /// Another process (normally `soos-daemon`) holds the camera device (GitHub #337).
    #[error(
        "Camera is busy: another process holds the camera device. soos-daemon owns the camera \
         while it runs; enroll from soos-gui, or stop the daemon (sudo systemctl stop soos-daemon), \
         enroll, then start it again (sudo systemctl start soos-daemon)"
    )]
    CameraBusy,

    #[error("Camera is not initialized; operation requires full service")]
    CameraNotInitialized,

    #[error("Vision pipeline is not initialized; operation requires full service")]
    PipelineNotInitialized,

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Vision pipeline error: {0}")]
    Vision(#[from] soos_vision::VisionError),

    #[error("Inference error: {0}")]
    Inference(#[from] soos_inference_ort::InferenceError),

    #[error("Biometric store error: {0}")]
    BiometricStore(#[from] soos_biometric_store::BiometricStoreError),

    #[error("Evidence store error: {0}")]
    EvidenceStore(#[from] soos_evidence_store::EvidenceStoreError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Nix system error: {0}")]
    Nix(#[from] nix::Error),

    /// Rejected `import` input (size bound, ownership, format or non-finite values).
    #[error("Invalid import input: {0}")]
    InvalidImport(String),

    /// The stored template belongs to another embedding model or has another vector length
    /// than the loaded model (for example the retired ArcFace): re-enrollment is required
    /// (GitHub #278). Carries metadata only, never vector values.
    #[error(
        "template of UID {uid} was enrolled with model '{template_model_id}' \
         ({template_dimension}-D), not the loaded embedding model: re-enroll with \
         `soos-enroll enroll`"
    )]
    TemplateModelMismatch {
        /// Target UID.
        uid: u32,
        /// Model id recorded in the template.
        template_model_id: String,
        /// Length of the stored vector.
        template_dimension: usize,
    },

    #[error("Invalid path or path traversal attempt: {0}")]
    InvalidPath(String),

    #[error("Refusing to write debug report to '{}': {reason}", path.display())]
    DebugReportRefused {
        path: std::path::PathBuf,
        reason: String,
    },
}

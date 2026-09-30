//! `soos-enrollment-cli` — Root enrollment and diagnostic tool for Linux Biometric PAM.
//!
//! Provides administrative commands for:
//! - Multi-frame enrollment with quality selection and interactive confirmation
//! - Diagnostic one-shot verification with latency and PAD reporting
//! - Deletion with anti-forensic secure erasure
//! - Enumeration of enrolled biometric templates

#![forbid(unsafe_code)]

pub mod args;
pub mod error;
pub mod guided_enrollment;
pub mod html_report;
pub mod quality;
pub mod service;
pub mod shred;

pub use args::{
    sanitize_path, validate_camera_device_path, validate_fhs_path, Cli, Commands, DebugVisionArgs,
    DeleteArgs, EnrollArgs, ImportArgs, ListArgs, OutputFormat, VerifyArgs, ALLOWED_FHS_PREFIXES,
};
pub use error::EnrollmentCliError;
pub use guided_enrollment::{EnrollmentStep, EnrollmentStepFeedback, GuidedEnrollmentSession};
pub use quality::{select_best_frame, BestCandidate, CandidateEvaluation};
pub use service::{
    build_full_service, build_service, build_store_only, check_privileges, ensure_debug_report_dir,
    resolve_camera_device, resolve_camera_device_from_config,
    resolve_camera_device_from_config_with, resolve_debug_report_path, write_debug_report,
    DiagnosticVerificationReport, EnrolledUserSummary, EnrollmentOutcome, EnrollmentService,
    EnrollmentSummary, LatencyBreakdown, DEBUG_REPORT_DIR_MODE, DEBUG_REPORT_FILE_MODE,
    DEFAULT_CAMERA_DEVICE, DEFAULT_DEBUG_REPORT_DIR, DEFAULT_KEY_PATH, DEFAULT_MODELS_DIR,
    MODEL_ID_EMBEDDING, MODEL_ID_FACE_DETECTOR, MODEL_ID_PAD, REQUIRED_MODEL_IDS,
};
pub use shred::secure_shred_file;

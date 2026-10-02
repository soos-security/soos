//! `soos-enrollment-cli` — Root enrollment and diagnostic tool for Linux Biometric PAM.
//!
//! Provides administrative commands for:
//! - Multi-frame enrollment with quality selection and interactive confirmation
//! - Diagnostic one-shot verification with latency and PAD reporting
//! - Deletion of a template: best-effort in-place overwrite before unlinking; the
//!   guarantee against recovery is encryption at rest plus master-key destruction
//!   (ADR 2026-09-30 "Biometric Template Erasure Model")
//! - Enumeration of enrolled biometric templates
//! - Operator-run migration of legacy (v1) templates and evidence snapshots to the
//!   AAD-bound v2 envelope (`migrate`, GitHub #287)

#![forbid(unsafe_code)]

pub mod args;
pub mod error;
pub mod guided_enrollment;
pub mod html_report;
pub mod quality;
pub mod service;

pub use args::{
    resolve_default_target_uid, sanitize_path, validate_camera_device_path, validate_fhs_path, Cli,
    Commands, DebugVisionArgs, DeleteArgs, EnrollArgs, ImportArgs, ImportCommand, ListArgs,
    MigrateArgs, OutputFormat, VerifyArgs, ALLOWED_FHS_PREFIXES,
};
pub use error::EnrollmentCliError;
pub use guided_enrollment::{EnrollmentStep, EnrollmentStepFeedback, GuidedEnrollmentSession};
pub use quality::{select_best_frame, BestCandidate, CandidateEvaluation};
pub use service::{
    build_evidence_for_migration, build_full_service, build_full_service_with_notes,
    build_store_only, build_templates_for_migration, camera_config_notes, check_privileges,
    enrollment_camera_config, ensure_debug_report_dir, format_enrolled_json, format_migration_json,
    open_evidence_store_for_migration, open_template_store_for_migration, resolve_camera_device,
    resolve_camera_device_from_config, resolve_camera_device_from_config_reported,
    resolve_camera_device_from_config_with, resolve_debug_report_path, run_migration,
    run_migration_with_reasons, write_debug_report, CameraDeviceChoice,
    DiagnosticVerificationReport, EnrolledUserSummary, EnrollmentOutcome, EnrollmentService,
    EnrollmentSummary, LatencyBreakdown, MigrationFailureSummary, MigrationSummary,
    StoreMigrationSummary, DEBUG_REPORT_DIR_MODE, DEBUG_REPORT_FILE_MODE, DEFAULT_CAMERA_DEVICE,
    DEFAULT_DEBUG_REPORT_DIR, DEFAULT_KEY_PATH, DEFAULT_MODELS_DIR, EMBEDDING_MODEL_VERSION,
    MODEL_ID_EMBEDDING, MODEL_ID_FACE_DETECTOR, MODEL_ID_PAD, REQUIRED_MODEL_IDS,
};

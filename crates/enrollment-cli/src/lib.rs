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
pub mod quality;
pub mod service;
pub mod shred;

pub use args::{Cli, Commands, DeleteArgs, EnrollArgs, ListArgs, OutputFormat, VerifyArgs};
pub use error::EnrollmentCliError;
pub use quality::{select_best_frame, BestCandidate, CandidateEvaluation};
pub use service::{
    check_privileges, DiagnosticVerificationReport, EnrolledUserSummary, EnrollmentOutcome,
    EnrollmentService, EnrollmentSummary, LatencyBreakdown,
};
pub use shred::secure_shred_file;

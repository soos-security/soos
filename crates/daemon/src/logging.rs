//! Structured logging configuration and initialization.
//!
//! # Security Invariant (Acceptance D5)
//!
//! Structured logging MUST never log:
//! - Raw camera frames or pixel buffers
//! - Biometric embeddings or feature vectors
//! - Passwords, hashes, or encryption keys
//! - Full raw request or response payloads
//!
//! Permitted log events: connection metadata (`peer_uid`, `peer_pid`),
//! anonymized `request_id`, sanitised `service`, rendered `verdict`,
//! `reason_class`, and processing duration in milliseconds.

use tracing_subscriber::{fmt, EnvFilter};

/// Initializes the global tracing subscriber with environment or configured level.
pub fn init_logging(default_level: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .compact()
        .try_init()?;

    Ok(())
}

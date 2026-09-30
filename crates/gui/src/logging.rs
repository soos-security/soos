//! Diagnostic logging for the GUI process (review finding CAM-07).
//!
//! Installs a `tracing` subscriber writing to stderr and honoring `RUST_LOG`, so camera
//! supervisor warnings (`EBUSY`, `EACCES`, missing device) and IPC failures reach the user and
//! support instead of being silently dropped. Logged events carry error kinds and device paths
//! only; frames, embeddings and credentials are never logged.

#![forbid(unsafe_code)]

use tracing_subscriber::EnvFilter;

/// Filter directive used when `RUST_LOG` is unset, empty or invalid.
pub const DEFAULT_LOG_DIRECTIVE: &str = "info";

/// Builds the log filter from an optional `RUST_LOG`-style directive, falling back to
/// [`DEFAULT_LOG_DIRECTIVE`] when it is absent, empty or unparsable.
pub fn build_env_filter(directive: Option<&str>) -> EnvFilter {
    directive
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .and_then(|d| EnvFilter::try_new(d).ok())
        .unwrap_or_else(|| EnvFilter::new(DEFAULT_LOG_DIRECTIVE))
}

/// Installs the global stderr subscriber. Returns `true` when this call installed it and
/// `false` when a subscriber was already installed (idempotent, never panics).
pub fn init() -> bool {
    let directive = std::env::var(EnvFilter::DEFAULT_ENV).ok();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(build_env_filter(directive.as_deref()))
        .try_init()
        .is_ok()
}

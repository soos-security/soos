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
//!
//! The request nonce is anonymized with [`short_request_id`] and logged only as
//! `request_id = %short_request_id(&req.request_id)` (GitHub #257; enforced by
//! `crates/daemon/tests/request_id_logging_tests.rs`).

use sha2::{Digest, Sha256};
use soos_protocol::types::RequestId;
use tracing_subscriber::{fmt, EnvFilter};

/// Domain-separation prefix of the log correlation digest.
const SHORT_REQUEST_ID_DOMAIN: &[u8] = b"soos.request-id.log.v1";

/// Number of digest bytes kept for log correlation.
const SHORT_REQUEST_ID_BYTES: usize = 4;

/// Length of the rendered [`ShortRequestId`] (lowercase hexadecimal digits).
pub const SHORT_REQUEST_ID_HEX_LEN: usize = SHORT_REQUEST_ID_BYTES * 2;

/// Anonymized, log-safe correlation tag of a request nonce (GitHub #257).
///
/// It holds the first 4 bytes of `SHA-256("soos.request-id.log.v1" || request_id)`:
/// enough to correlate the log lines of one request, while revealing no bit of the
/// 256-bit single-use nonce. `Display` and `Debug` both render 8 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ShortRequestId([u8; SHORT_REQUEST_ID_BYTES]);

impl std::fmt::Display for ShortRequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for ShortRequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

/// Returns the anonymized log tag of `request_id` (see [`ShortRequestId`]).
#[must_use]
pub fn short_request_id(request_id: &RequestId) -> ShortRequestId {
    let mut hasher = Sha256::new();
    hasher.update(SHORT_REQUEST_ID_DOMAIN);
    hasher.update(request_id);
    let digest = hasher.finalize();
    let mut tag = [0u8; SHORT_REQUEST_ID_BYTES];
    for (dst, src) in tag.iter_mut().zip(digest.iter()) {
        *dst = *src;
    }
    ShortRequestId(tag)
}

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

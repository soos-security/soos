//! Explicit error types for policy evaluation and configuration.

use core::fmt;

/// Errors arising during threshold validation or rate limit enforcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// Provided threshold configuration value is invalid (e.g. NaN, out of range 0.0..=1.0).
    InvalidThreshold {
        /// Name of the threshold setting.
        name: &'static str,
        /// Invalid numeric value encountered, stored as bits to ensure `Eq`.
        value_bits: u32,
        /// Detailed description of the invariant failure.
        reason: &'static str,
    },
    /// Rate limit has been exceeded for the target user ID.
    RateLimitExceeded {
        /// User ID subject to the rate limit.
        uid: u32,
        /// Duration in nanoseconds until the next attempt is allowed.
        retry_after_ns: u64,
    },
}

impl PolicyError {
    /// Construct an `InvalidThreshold` error with a float representation.
    #[must_use]
    pub fn invalid_threshold(name: &'static str, value: f32, reason: &'static str) -> Self {
        Self::InvalidThreshold {
            name,
            value_bits: value.to_bits(),
            reason,
        }
    }
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidThreshold {
                name,
                value_bits,
                reason,
            } => {
                let val = f32::from_bits(*value_bits);
                write!(f, "invalid threshold for {name} ({val}): {reason}")
            }
            Self::RateLimitExceeded {
                uid,
                retry_after_ns,
            } => {
                write!(
                    f,
                    "rate limit exceeded for UID {uid}: retry after {retry_after_ns} ns"
                )
            }
        }
    }
}

impl std::error::Error for PolicyError {}

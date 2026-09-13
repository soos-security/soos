//! # soos-policy
//!
//! Pure authorization logic and per-UID rate limiting (Zero I/O) for `soos`.
//!
//! ## Architectural Invariants
//!
//! - **Zero I/O**: Contains zero file, network, or asynchronous dependencies.
//! - **Deterministic Clockless Evaluation**: Rate limiting accepts caller-supplied monotonic timestamps.
//! - **Memory Safe**: `#![forbid(unsafe_code)]` is strictly enforced.
//! - **Fail Closed**: All invalid contexts degrade to non-authorizing verdicts (`Deny` or `ProtocolError`).

#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    reason = "Idiomatic policy type naming"
)]

pub mod decision;
pub mod error;
pub mod rate_limit;
pub mod threshold;

pub use decision::{evaluate_decision, AuthContext, AuthorizationEngine};
pub use error::PolicyError;
pub use rate_limit::{RateLimitConfig, RateLimiter};
pub use threshold::{ThresholdConfig, ThresholdConfigBuilder};

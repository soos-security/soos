//! Biometric authorization decision engine adhering to ARCHITECTURE.md §3.

use crate::error::PolicyError;
use crate::rate_limit::RateLimiter;
use crate::threshold::ThresholdConfig;
use soos_protocol::{ReasonClass, Verdict};

/// Structured biometric verification context provided by the daemon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuthContext {
    /// Cosine similarity score between captured frame and enrolled embedding.
    pub score: f32,
    /// Presentation attack detection (anti-spoof / liveness) status.
    pub pad_passed: bool,
    /// Number of detected human faces in the capture frame.
    pub face_count: u8,
    /// Target user ID asserted for authentication.
    pub uid: u32,
    /// Whether the peer process has an active, valid session matching the asserted UID.
    pub session_valid: bool,
}

impl AuthContext {
    /// Construct a new [`AuthContext`].
    #[must_use]
    pub const fn new(
        score: f32,
        pad_passed: bool,
        face_count: u8,
        uid: u32,
        session_valid: bool,
    ) -> Self {
        Self {
            score,
            pad_passed,
            face_count,
            uid,
            session_valid,
        }
    }
}

/// Authorization engine evaluating biometric contexts against thresholds and rate limits.
#[derive(Debug, Clone)]
pub struct AuthorizationEngine {
    pub(crate) thresholds: ThresholdConfig,
    pub(crate) rate_limiter: Option<RateLimiter>,
}

impl AuthorizationEngine {
    /// Construct an engine with the given threshold configuration.
    #[must_use]
    pub const fn new(thresholds: ThresholdConfig) -> Self {
        Self {
            thresholds,
            rate_limiter: None,
        }
    }

    /// Construct an engine with thresholds and an active per-UID rate limiter.
    #[must_use]
    pub fn with_rate_limiter(thresholds: ThresholdConfig, rate_limiter: RateLimiter) -> Self {
        Self {
            thresholds,
            rate_limiter: Some(rate_limiter),
        }
    }

    /// Access the configured thresholds.
    #[must_use]
    pub const fn thresholds(&self) -> &ThresholdConfig {
        &self.thresholds
    }

    /// Access the internal rate limiter if configured.
    #[must_use]
    pub fn rate_limiter(&self) -> Option<&RateLimiter> {
        self.rate_limiter.as_ref()
    }

    /// Mutably access the internal rate limiter if configured.
    pub fn rate_limiter_mut(&mut self) -> Option<&mut RateLimiter> {
        self.rate_limiter.as_mut()
    }

    /// Check whether a request from `uid` is allowed under the configured rate limiter.
    ///
    /// If no rate limiter is configured, this always succeeds.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::RateLimitExceeded`] if currently blocked.
    pub fn check_allowed(&self, uid: u32, now_monotonic_ns: u64) -> Result<(), PolicyError> {
        if let Some(ref limiter) = self.rate_limiter {
            limiter.check_allowed(uid, now_monotonic_ns)
        } else {
            Ok(())
        }
    }

    /// Pure evaluation of an [`AuthContext`] without rate-limiting state changes.
    ///
    /// Adheres to the Request State Matrix in `AI/ARCHITECTURE.md` §3:
    /// - `Allow`: only if `score >= threshold && pad_passed && face_count == 1 && session_valid`.
    /// - `Deny`: if `face_count != 1`, `pad_passed == false`, or `score < threshold`.
    /// - `ProtocolError`: if `session_valid == false` (or rate-limited).
    #[must_use]
    pub fn evaluate(&self, ctx: &AuthContext) -> (Verdict, ReasonClass) {
        if !ctx.session_valid {
            return (Verdict::ProtocolError, ReasonClass::UidMismatch);
        }
        if ctx.face_count == 0 {
            return (Verdict::Deny, ReasonClass::NoFace);
        }
        if ctx.face_count > 1 {
            return (Verdict::Deny, ReasonClass::MultipleFaces);
        }
        if !ctx.pad_passed {
            return (Verdict::Deny, ReasonClass::PadFailed);
        }
        if ctx.score.is_nan() || ctx.score < self.thresholds.match_threshold() {
            return (Verdict::Deny, ReasonClass::ScoreBelowThreshold);
        }

        (Verdict::Allow, ReasonClass::FaceMatch)
    }

    /// Evaluates an [`AuthContext`] with rate-limiting enforcement.
    ///
    /// If the rate limiter is configured and the attempt limit is exceeded,
    /// returns `(Verdict::ProtocolError, ReasonClass::RateLimited)`.
    /// Otherwise, evaluates the context and records the attempt.
    pub fn evaluate_with_rate_limit(
        &mut self,
        ctx: &AuthContext,
        now_monotonic_ns: u64,
    ) -> (Verdict, ReasonClass) {
        if let Some(ref mut limiter) = self.rate_limiter {
            if limiter.check_and_record(ctx.uid, now_monotonic_ns).is_err() {
                return (Verdict::ProtocolError, ReasonClass::RateLimited);
            }
        }
        self.evaluate(ctx)
    }
}

/// Convenience standalone function for pure evaluation against a [`ThresholdConfig`].
#[must_use]
pub fn evaluate_decision(
    thresholds: &ThresholdConfig,
    ctx: &AuthContext,
) -> (Verdict, ReasonClass) {
    AuthorizationEngine::new(*thresholds).evaluate(ctx)
}

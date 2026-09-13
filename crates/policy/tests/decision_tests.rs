//! Contractual integration tests for AuthorizationDecision engine (PO1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions and proptest"
)]

use proptest::prelude::*;
use soos_policy::{
    evaluate_decision, AuthContext, AuthorizationEngine, RateLimitConfig, RateLimiter,
    ThresholdConfig,
};
use soos_protocol::{ReasonClass, Verdict};

#[test]
fn test_decision_allow_nominal() {
    let thresholds = ThresholdConfig::default(); // match: 0.70, pad: 0.85
    let ctx = AuthContext::new(0.85, true, 1, 1000, true);

    let (verdict, reason) = evaluate_decision(&thresholds, &ctx);
    assert_eq!(verdict, Verdict::Allow);
    assert_eq!(reason, ReasonClass::FaceMatch);
    assert!(!verdict.should_ignore());
}

#[test]
fn test_decision_allow_exact_threshold() {
    let thresholds = ThresholdConfig::default();
    let ctx = AuthContext::new(thresholds.match_threshold(), true, 1, 1000, true);

    let (verdict, reason) = evaluate_decision(&thresholds, &ctx);
    assert_eq!(verdict, Verdict::Allow);
    assert_eq!(reason, ReasonClass::FaceMatch);
}

#[test]
fn test_decision_deny_score_below_threshold() {
    let thresholds = ThresholdConfig::default();
    let ctx = AuthContext::new(0.6999, true, 1, 1000, true);

    let (verdict, reason) = evaluate_decision(&thresholds, &ctx);
    assert_eq!(verdict, Verdict::Deny);
    assert_eq!(reason, ReasonClass::ScoreBelowThreshold);
    assert!(verdict.should_ignore());
}

#[test]
fn test_decision_deny_score_nan() {
    let thresholds = ThresholdConfig::default();
    let ctx = AuthContext::new(f32::NAN, true, 1, 1000, true);

    let (verdict, reason) = evaluate_decision(&thresholds, &ctx);
    assert_eq!(verdict, Verdict::Deny);
    assert_eq!(reason, ReasonClass::ScoreBelowThreshold);
    assert!(verdict.should_ignore());
}

#[test]
fn test_decision_deny_pad_failed() {
    let thresholds = ThresholdConfig::default();
    let ctx = AuthContext::new(0.95, false, 1, 1000, true);

    let (verdict, reason) = evaluate_decision(&thresholds, &ctx);
    assert_eq!(verdict, Verdict::Deny);
    assert_eq!(reason, ReasonClass::PadFailed);
    assert!(verdict.should_ignore());
}

#[test]
fn test_decision_deny_no_face() {
    let thresholds = ThresholdConfig::default();
    let ctx = AuthContext::new(0.95, true, 0, 1000, true);

    let (verdict, reason) = evaluate_decision(&thresholds, &ctx);
    assert_eq!(verdict, Verdict::Deny);
    assert_eq!(reason, ReasonClass::NoFace);
    assert!(verdict.should_ignore());
}

#[test]
fn test_decision_deny_multiple_faces() {
    let thresholds = ThresholdConfig::default();

    for face_count in [2, 3, 5, 255] {
        let ctx = AuthContext::new(0.95, true, face_count, 1000, true);
        let (verdict, reason) = evaluate_decision(&thresholds, &ctx);
        assert_eq!(
            verdict,
            Verdict::Deny,
            "Face count {face_count} must result in Deny"
        );
        assert_eq!(
            reason,
            ReasonClass::MultipleFaces,
            "Face count {face_count} must produce MultipleFaces"
        );
        assert!(verdict.should_ignore());
    }
}

#[test]
fn test_decision_protocol_error_session_invalid() {
    let thresholds = ThresholdConfig::default();
    let ctx = AuthContext::new(0.95, true, 1, 1000, false);

    let (verdict, reason) = evaluate_decision(&thresholds, &ctx);
    assert_eq!(verdict, Verdict::ProtocolError);
    assert_eq!(reason, ReasonClass::UidMismatch);
    assert!(verdict.should_ignore());
}

#[test]
fn test_decision_with_rate_limiter_integrated() {
    let thresholds = ThresholdConfig::default();
    let rate_limiter = RateLimiter::new(RateLimitConfig::new(2, 60_000_000_000));
    let mut engine = AuthorizationEngine::with_rate_limiter(thresholds, rate_limiter);

    let ctx = AuthContext::new(0.90, true, 1, 1000, true);
    let t0 = 1_000_000_000;

    // First two requests pass
    let (v1, r1) = engine.evaluate_with_rate_limit(&ctx, t0);
    assert_eq!(v1, Verdict::Allow);
    assert_eq!(r1, ReasonClass::FaceMatch);

    let (v2, r2) = engine.evaluate_with_rate_limit(&ctx, t0 + 100);
    assert_eq!(v2, Verdict::Allow);
    assert_eq!(r2, ReasonClass::FaceMatch);

    // Third request triggers rate limit -> ProtocolError / RateLimited
    let (v3, r3) = engine.evaluate_with_rate_limit(&ctx, t0 + 200);
    assert_eq!(v3, Verdict::ProtocolError);
    assert_eq!(r3, ReasonClass::RateLimited);
    assert!(v3.should_ignore());
}

// ---------------------------------------------------------------------------
// Property-Based Testing (Proptest)
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_decision_allow_invariant(
        score in -1.0_f32..2.0_f32,
        pad_passed in any::<bool>(),
        face_count in any::<u8>(),
        uid in any::<u32>(),
        session_valid in any::<bool>(),
        match_threshold in 0.0_f32..1.0_f32,
        pad_threshold in 0.0_f32..1.0_f32,
    ) {
        let thresholds = ThresholdConfig::new_raw(match_threshold, pad_threshold);
        let ctx = AuthContext::new(score, pad_passed, face_count, uid, session_valid);

        let (verdict, reason) = evaluate_decision(&thresholds, &ctx);

        let expected_allow = !score.is_nan()
            && score >= match_threshold
            && pad_passed
            && face_count == 1
            && session_valid;

        if expected_allow {
            prop_assert_eq!(verdict, Verdict::Allow);
            prop_assert_eq!(reason, ReasonClass::FaceMatch);
            prop_assert!(!verdict.should_ignore());
        } else {
            prop_assert_ne!(verdict, Verdict::Allow);
            prop_assert!(verdict.should_ignore());
        }
    }
}

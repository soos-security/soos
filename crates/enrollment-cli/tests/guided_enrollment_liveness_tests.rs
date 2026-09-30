//! Contractual tests for session-level liveness tracking in guided enrollment
//! (GitHub #217 / PAD-12): a sample needs k consecutive live frames, a spoof frame
//! discards the current step, and repeated spoof frames abort the whole session.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses direct assertions and geometric fixtures"
)]

use soos_enrollment_cli::guided_enrollment::{
    EnrollmentStep, EnrollmentStepFeedback, GuidedEnrollmentSession, LivenessPolicy,
    DEFAULT_MAX_SPOOF_EVENTS, DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES, MAX_LIVENESS_POLICY_BOUND,
};
use soos_vision::pose::HeadPose;

const DIM: usize = 512;

fn identity() -> Vec<f32> {
    let mut v = vec![0.0f32; DIM];
    v[0] = 1.0;
    v
}

fn frontal() -> HeadPose {
    HeadPose {
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
    }
}

fn live(session: &mut GuidedEnrollmentSession) -> EnrollmentStepFeedback {
    session.process_sample(&frontal(), &identity(), true, true)
}

fn spoof(session: &mut GuidedEnrollmentSession) -> EnrollmentStepFeedback {
    session.process_sample(&frontal(), &identity(), false, true)
}

fn strict_session(per_step: usize) -> GuidedEnrollmentSession {
    GuidedEnrollmentSession::with_liveness_policy(per_step, LivenessPolicy::strict())
}

#[test]
fn test_liveness_policy_constants_and_strict_defaults() {
    assert_eq!(DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES, 3);
    assert_eq!(DEFAULT_MAX_SPOOF_EVENTS, 3);
    assert_eq!(MAX_LIVENESS_POLICY_BOUND, 32);
    let strict = LivenessPolicy::strict();
    assert_eq!(strict, LivenessPolicy::default());
    assert_eq!(
        strict.min_consecutive_live_frames,
        DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES
    );
    assert_eq!(strict.max_spoof_events, DEFAULT_MAX_SPOOF_EVENTS);
    let single = LivenessPolicy::single_frame();
    assert_eq!(single.min_consecutive_live_frames, 1);
    assert_eq!(single.max_spoof_events, DEFAULT_MAX_SPOOF_EVENTS);
}

#[test]
fn test_strict_session_requires_k_consecutive_live_frames_before_first_sample() {
    let mut session = strict_session(4);
    for _ in 1..DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES {
        assert_eq!(live(&mut session), EnrollmentStepFeedback::PromptHoldStill);
    }
    assert_eq!(session.progress_percent(), 0.0);
    assert_eq!(
        live(&mut session),
        EnrollmentStepFeedback::SampleAccepted {
            step: EnrollmentStep::Frontal,
            collected: 1,
            target: 4
        }
    );
}

#[test]
fn test_intermittent_liveness_never_yields_a_sample() {
    // A presentation that passes PAD on every other frame must never be sampled.
    let mut session = GuidedEnrollmentSession::with_liveness_policy(
        4,
        LivenessPolicy {
            min_consecutive_live_frames: 3,
            max_spoof_events: MAX_LIVENESS_POLICY_BOUND,
        },
    );
    for _ in 0..10 {
        assert_eq!(live(&mut session), EnrollmentStepFeedback::PromptHoldStill);
        assert_eq!(spoof(&mut session), EnrollmentStepFeedback::SpoofDetected);
    }
    assert_eq!(session.progress_percent(), 0.0);
    assert!(session.compute_composite_embedding().is_err());
}

#[test]
fn test_spoof_frame_discards_samples_of_current_step() {
    let mut session = strict_session(4);
    for _ in 0..DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES + 1 {
        live(&mut session);
    }
    assert!(
        session.progress_percent() > 0.0,
        "two frontal samples expected"
    );
    assert_eq!(spoof(&mut session), EnrollmentStepFeedback::SpoofDetected);
    assert_eq!(session.progress_percent(), 0.0);
    assert_eq!(session.current_step(), EnrollmentStep::Frontal);
    assert_eq!(session.spoof_events(), 1);
    // The streak restarts: the next live frames are not sampled immediately.
    assert_eq!(live(&mut session), EnrollmentStepFeedback::PromptHoldStill);
}

#[test]
fn test_repeated_spoof_frames_abort_the_session() {
    let mut session = strict_session(1);
    // Complete the frontal step first.
    for _ in 0..DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES {
        live(&mut session);
    }
    assert_eq!(session.current_step(), EnrollmentStep::TurnLeft);

    for _ in 1..DEFAULT_MAX_SPOOF_EVENTS {
        assert_eq!(spoof(&mut session), EnrollmentStepFeedback::SpoofDetected);
        assert!(!session.is_aborted());
    }
    assert_eq!(spoof(&mut session), EnrollmentStepFeedback::SessionAborted);
    assert!(session.is_aborted());
    // Every sample, including earlier completed steps, is discarded.
    assert_eq!(session.progress_percent(), 0.0);
    assert!(session.compute_composite_embedding().is_err());
    // An aborted session never accepts samples again.
    for _ in 0..10 {
        assert_eq!(live(&mut session), EnrollmentStepFeedback::SessionAborted);
    }
    assert_eq!(session.progress_percent(), 0.0);
}

#[test]
fn test_new_session_keeps_single_frame_gate_but_aborts_on_repeated_spoofs() {
    let mut session = GuidedEnrollmentSession::new(4);
    for _ in 1..DEFAULT_MAX_SPOOF_EVENTS {
        assert_eq!(spoof(&mut session), EnrollmentStepFeedback::SpoofDetected);
    }
    assert_eq!(spoof(&mut session), EnrollmentStepFeedback::SessionAborted);
    assert!(session.is_aborted());
}

#[test]
fn test_interrupted_streak_requires_fresh_consecutive_live_frames() {
    let mut session = strict_session(4);
    for _ in 1..DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES {
        live(&mut session);
    }
    // A frame without a PAD verdict (no crop, quality gate) breaks the streak
    // without counting as a spoof event.
    session.interrupt_liveness_streak();
    assert_eq!(live(&mut session), EnrollmentStepFeedback::PromptHoldStill);
    assert_eq!(session.spoof_events(), 0);
    assert!(!session.is_aborted());
}

#[test]
fn test_liveness_policy_values_are_clamped() {
    let mut session = GuidedEnrollmentSession::with_liveness_policy(
        1,
        LivenessPolicy {
            min_consecutive_live_frames: 0,
            max_spoof_events: 0,
        },
    );
    // min_consecutive_live_frames clamps to 1: first live frame is sampled.
    assert_eq!(
        live(&mut session),
        EnrollmentStepFeedback::StepCompleted {
            next_step: EnrollmentStep::TurnLeft
        }
    );
    // max_spoof_events clamps to 1: first spoof aborts.
    assert_eq!(spoof(&mut session), EnrollmentStepFeedback::SessionAborted);

    let mut huge = GuidedEnrollmentSession::with_liveness_policy(
        1,
        LivenessPolicy {
            min_consecutive_live_frames: usize::MAX,
            max_spoof_events: usize::MAX,
        },
    );
    for _ in 1..MAX_LIVENESS_POLICY_BOUND {
        assert_eq!(live(&mut huge), EnrollmentStepFeedback::PromptHoldStill);
    }
    assert_eq!(
        live(&mut huge),
        EnrollmentStepFeedback::StepCompleted {
            next_step: EnrollmentStep::TurnLeft
        }
    );
}

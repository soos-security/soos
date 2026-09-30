//! Contractual tests for guided enrollment sample validation and fusion safety
//! (GitHub #183 / STO-10): finiteness, identity consistency, bounded sample count and
//! enforced pose ranges.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::float_cmp,
    clippy::panic,
    reason = "Contractual test suite uses direct assertions and geometric fixtures"
)]

use soos_enrollment_cli::guided_enrollment::{
    EnrollmentStep, EnrollmentStepFeedback, GuidedEnrollmentSession, MAX_GUIDED_EMBEDDING_DIM,
    MAX_SAMPLES_PER_STEP, MIN_SAMPLE_CONSISTENCY_COSINE,
};
use soos_vision::pose::HeadPose;

const DIM: usize = 512;

/// Unit vector along axis `axis`, optionally blended with axis 0 by `mix`.
fn unit(axis: usize, mix: f32) -> Vec<f32> {
    let mut v = vec![0.0f32; DIM];
    v[0] += mix;
    v[axis] += 1.0;
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in &mut v {
        *x /= norm;
    }
    v
}

fn pose(yaw: f32, pitch: f32, roll: f32) -> HeadPose {
    HeadPose { yaw, pitch, roll }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Session with the frontal step completed using the identity direction `unit(0, 0)`.
fn session_after_frontal(per_step: usize) -> GuidedEnrollmentSession {
    let mut session = GuidedEnrollmentSession::new(per_step);
    for _ in 0..per_step {
        session.process_sample(&pose(0.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    }
    assert_eq!(session.current_step(), EnrollmentStep::TurnLeft);
    session
}

#[test]
fn test_consistency_threshold_and_bounds_are_documented_constants() {
    const { assert!(MIN_SAMPLE_CONSISTENCY_COSINE > 0.0 && MIN_SAMPLE_CONSISTENCY_COSINE < 1.0) };
    assert_eq!(MIN_SAMPLE_CONSISTENCY_COSINE, 0.5);
    assert_eq!(MAX_SAMPLES_PER_STEP, 20);
    const { assert!(MAX_GUIDED_EMBEDDING_DIM >= DIM) };
}

#[test]
fn test_sample_count_is_bounded_per_step() {
    let mut session = GuidedEnrollmentSession::new(10_000);
    let mut accepted = 0usize;
    loop {
        match session.process_sample(&pose(0.0, 0.0, 0.0), &unit(0, 0.0), true, true) {
            EnrollmentStepFeedback::SampleAccepted { target, .. } => {
                assert_eq!(target, MAX_SAMPLES_PER_STEP);
                accepted += 1;
            }
            EnrollmentStepFeedback::StepCompleted { .. } => {
                accepted += 1;
                break;
            }
            other => panic!("Unexpected feedback {other:?}"),
        }
        assert!(accepted <= MAX_SAMPLES_PER_STEP);
    }
    assert_eq!(accepted, MAX_SAMPLES_PER_STEP);
}

#[test]
fn test_non_finite_embeddings_are_rejected() {
    let mut session = GuidedEnrollmentSession::new(2);
    let mut nan = unit(0, 0.0);
    nan[7] = f32::NAN;
    let mut inf = unit(0, 0.0);
    inf[3] = f32::INFINITY;
    for bad in [nan, inf] {
        let fb = session.process_sample(&pose(0.0, 0.0, 0.0), &bad, true, true);
        assert_eq!(fb, EnrollmentStepFeedback::InvalidEmbedding);
    }
    assert_eq!(session.progress_percent(), 0.0);
    assert!(session.compute_composite_embedding().is_err());
}

#[test]
fn test_zero_norm_embedding_is_rejected() {
    let mut session = GuidedEnrollmentSession::new(2);
    let fb = session.process_sample(&pose(0.0, 0.0, 0.0), &vec![0.0; DIM], true, true);
    assert_eq!(fb, EnrollmentStepFeedback::InvalidEmbedding);
    assert_eq!(session.progress_percent(), 0.0);
}

#[test]
fn test_dimension_mismatch_and_oversized_embeddings_are_rejected() {
    let mut session = GuidedEnrollmentSession::new(3);
    session.process_sample(&pose(0.0, 0.0, 0.0), &unit(0, 0.0), true, true);

    let short = vec![1.0f32; 128];
    let fb = session.process_sample(&pose(0.0, 0.0, 0.0), &short, true, true);
    assert_eq!(fb, EnrollmentStepFeedback::InvalidEmbedding);

    let mut fresh = GuidedEnrollmentSession::new(3);
    let mut huge = vec![0.0f32; MAX_GUIDED_EMBEDDING_DIM + 1];
    huge[0] = 1.0;
    let fb = fresh.process_sample(&pose(0.0, 0.0, 0.0), &huge, true, true);
    assert_eq!(fb, EnrollmentStepFeedback::InvalidEmbedding);
    assert_eq!(fresh.progress_percent(), 0.0);
}

#[test]
fn test_inconsistent_frontal_sample_is_rejected() {
    let mut session = GuidedEnrollmentSession::new(3);
    session.process_sample(&pose(0.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    // Orthogonal identity in front of the camera during the frontal step.
    let fb = session.process_sample(&pose(0.0, 0.0, 0.0), &unit(5, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::IdentityMismatch);
    let fb = session.process_sample(&pose(0.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(
        fb,
        EnrollmentStepFeedback::SampleAccepted {
            step: EnrollmentStep::Frontal,
            collected: 2,
            target: 3
        }
    );
}

#[test]
fn test_second_identity_during_off_axis_steps_does_not_contaminate_template() {
    let mut session = session_after_frontal(2);

    // Another person (orthogonal embedding) during every off-axis step.
    let intruder = unit(9, 0.0);
    assert_eq!(
        session.process_sample(&pose(-15.0, 0.0, 0.0), &intruder, true, true),
        EnrollmentStepFeedback::IdentityMismatch
    );
    for _ in 0..2 {
        session.process_sample(&pose(-15.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    }
    assert_eq!(
        session.process_sample(&pose(15.0, 0.0, 0.0), &intruder, true, true),
        EnrollmentStepFeedback::IdentityMismatch
    );
    for _ in 0..2 {
        session.process_sample(&pose(15.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    }
    assert_eq!(
        session.process_sample(&pose(0.0, -12.0, 0.0), &intruder, true, true),
        EnrollmentStepFeedback::IdentityMismatch
    );
    session.process_sample(&pose(0.0, -12.0, 0.0), &unit(0, 0.0), true, true);
    let fb = session.process_sample(&pose(0.0, -12.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::AllStepsCompleted);

    let composite = session.compute_composite_embedding().unwrap();
    assert!(composite.iter().all(|x| x.is_finite()));
    assert!(
        (dot(&composite, &unit(0, 0.0)) - 1.0).abs() < 1e-5,
        "composite must equal the frontal identity direction"
    );
    assert!(dot(&composite, &intruder).abs() < 1e-5);
}

#[test]
fn test_consistent_off_axis_sample_above_threshold_is_accepted() {
    let mut session = session_after_frontal(1);
    // cos(unit(0), unit(4, 1.0)) = 1/sqrt(2) ~ 0.707 >= 0.5
    let fb = session.process_sample(&pose(-15.0, 0.0, 0.0), &unit(4, 1.0), true, true);
    assert_eq!(
        fb,
        EnrollmentStepFeedback::StepCompleted {
            next_step: EnrollmentStep::TurnRight
        }
    );
    // cos(unit(0), unit(4, 0.5)) = 0.5/sqrt(1.25) ~ 0.447 < 0.5
    let fb = session.process_sample(&pose(15.0, 0.0, 0.0), &unit(4, 0.5), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::IdentityMismatch);
}

#[test]
fn test_extreme_turn_left_is_out_of_range() {
    let mut session = session_after_frontal(1);
    let fb = session.process_sample(&pose(-80.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::PoseOutOfRange);
    assert_eq!(session.current_step(), EnrollmentStep::TurnLeft);
    // Boundaries of the documented [-25, -10] range are accepted.
    let fb = session.process_sample(&pose(-25.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(
        fb,
        EnrollmentStepFeedback::StepCompleted {
            next_step: EnrollmentStep::TurnRight
        }
    );
}

#[test]
fn test_extreme_turn_right_is_out_of_range() {
    let mut session = session_after_frontal(1);
    session.process_sample(&pose(-15.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    let fb = session.process_sample(&pose(80.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::PoseOutOfRange);
    let fb = session.process_sample(&pose(26.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::PoseOutOfRange);
    assert_eq!(session.current_step(), EnrollmentStep::TurnRight);
}

#[test]
fn test_tilt_up_enforces_documented_range() {
    let mut session = session_after_frontal(1);
    session.process_sample(&pose(-15.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    session.process_sample(&pose(15.0, 0.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(session.current_step(), EnrollmentStep::TiltUp);

    // -6 degrees is not enough: the documented lower bound is -8.
    let fb = session.process_sample(&pose(0.0, -6.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::PromptTiltUp);
    let fb = session.process_sample(&pose(0.0, -60.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::PoseOutOfRange);
    assert_eq!(session.current_step(), EnrollmentStep::TiltUp);
    let fb = session.process_sample(&pose(0.0, -8.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::AllStepsCompleted);
}

#[test]
fn test_off_axis_steps_reject_combined_extreme_rotations() {
    let mut session = session_after_frontal(1);
    // Correct yaw but extreme roll or pitch: not a meaningful turn-left sample.
    let fb = session.process_sample(&pose(-15.0, 0.0, 40.0), &unit(0, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::PromptHoldStill);
    let fb = session.process_sample(&pose(-15.0, -45.0, 0.0), &unit(0, 0.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::PoseOutOfRange);
    assert_eq!(session.current_step(), EnrollmentStep::TurnLeft);
}

#[test]
fn test_non_finite_pose_is_rejected() {
    let mut session = GuidedEnrollmentSession::new(1);
    let fb = session.process_sample(&pose(f32::NAN, 0.0, 0.0), &unit(0, 0.0), true, true);
    assert_ne!(
        fb,
        EnrollmentStepFeedback::StepCompleted {
            next_step: EnrollmentStep::TurnLeft
        }
    );
    assert_eq!(session.progress_percent(), 0.0);
}

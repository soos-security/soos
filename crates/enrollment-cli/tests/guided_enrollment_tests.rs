//! Contractual test suite for guided multi-step enrollment state machine and embedding fusion.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses direct assertions and geometric fixtures"
)]

use soos_enrollment_cli::guided_enrollment::{
    EnrollmentStep, EnrollmentStepFeedback, GuidedEnrollmentSession,
};
use soos_vision::pose::HeadPose;

fn dummy_embedding(val: f32) -> Vec<f32> {
    let mut v = vec![val; 512];
    // Normalize to unit vector
    let norm = (v.iter().map(|x| x * x).sum::<f32>()).sqrt();
    for x in &mut v {
        *x /= norm;
    }
    v
}

#[test]
fn test_guided_enrollment_full_workflow() {
    let mut session = GuidedEnrollmentSession::new(2); // 2 samples per step
    assert_eq!(session.current_step(), EnrollmentStep::Frontal);
    assert_eq!(session.progress_percent(), 0.0);

    // 1. Submit sample that is spoofed -> rejected
    let frontal_pose = HeadPose {
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
    };
    let fb = session.process_sample(&frontal_pose, &dummy_embedding(1.0), false, true);
    assert_eq!(fb, EnrollmentStepFeedback::SpoofDetected);
    assert_eq!(session.progress_percent(), 0.0);

    // 2. Submit valid frontal sample 1
    let fb = session.process_sample(&frontal_pose, &dummy_embedding(1.0), true, true);
    assert_eq!(
        fb,
        EnrollmentStepFeedback::SampleAccepted {
            step: EnrollmentStep::Frontal,
            collected: 1,
            target: 2
        }
    );

    // 3. Submit valid frontal sample 2 -> transitions to TurnLeft
    let fb = session.process_sample(&frontal_pose, &dummy_embedding(1.0), true, true);
    assert_eq!(
        fb,
        EnrollmentStepFeedback::StepCompleted {
            next_step: EnrollmentStep::TurnLeft
        }
    );
    assert_eq!(session.current_step(), EnrollmentStep::TurnLeft);

    // 4. Submit frontal pose when TurnLeft is expected -> feedback prompts TurnHeadLeft
    let fb = session.process_sample(&frontal_pose, &dummy_embedding(1.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::PromptTurnLeft);

    // 5. Submit valid TurnLeft samples (yaw = -15.0)
    let left_pose = HeadPose {
        yaw: -15.0,
        pitch: 0.0,
        roll: 0.0,
    };
    session.process_sample(&left_pose, &dummy_embedding(1.0), true, true);
    let fb = session.process_sample(&left_pose, &dummy_embedding(1.0), true, true);
    assert_eq!(
        fb,
        EnrollmentStepFeedback::StepCompleted {
            next_step: EnrollmentStep::TurnRight
        }
    );
    assert_eq!(session.current_step(), EnrollmentStep::TurnRight);

    // 6. Submit valid TurnRight samples (yaw = +15.0)
    let right_pose = HeadPose {
        yaw: 15.0,
        pitch: 0.0,
        roll: 0.0,
    };
    session.process_sample(&right_pose, &dummy_embedding(1.0), true, true);
    let fb = session.process_sample(&right_pose, &dummy_embedding(1.0), true, true);
    assert_eq!(
        fb,
        EnrollmentStepFeedback::StepCompleted {
            next_step: EnrollmentStep::TiltUp
        }
    );
    assert_eq!(session.current_step(), EnrollmentStep::TiltUp);

    // 7. Submit valid TiltUp samples (pitch = -12.0)
    let up_pose = HeadPose {
        yaw: 0.0,
        pitch: -12.0,
        roll: 0.0,
    };
    session.process_sample(&up_pose, &dummy_embedding(1.0), true, true);
    let fb = session.process_sample(&up_pose, &dummy_embedding(1.0), true, true);
    assert_eq!(fb, EnrollmentStepFeedback::AllStepsCompleted);
    assert_eq!(session.current_step(), EnrollmentStep::Completed);
    assert_eq!(session.progress_percent(), 100.0);

    // 8. Compute composite embedding: should be 512D unit vector
    let composite = session.compute_composite_embedding().unwrap();
    assert_eq!(composite.len(), 512);

    let norm = (composite.iter().map(|x| x * x).sum::<f32>()).sqrt();
    assert!((norm - 1.0).abs() < 1e-4);
}

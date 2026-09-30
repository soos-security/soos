//! GitHub #285 (row RFX10): a spoof PAD verdict is counted by the GUI guided-enrollment
//! session even when the frame produced no pose or no embedding (alignment or extraction
//! failure), so repeated attacks still abort the session.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses direct assertions and synthetic fixtures"
)]

use soos_enrollment_cli::guided_enrollment::{EnrollmentStepFeedback, LivenessPolicy};
use soos_gui::worker::{feed_guided_enrollment, new_guided_enrollment_session};
use soos_inference_ort::{AttackType, BoundingBox, FaceDetection, PadResult};
use soos_vision::pose::HeadPose;
use soos_vision::VisionAnalysis;
use zeroize::Zeroizing;

const W: u32 = 640;
const H: u32 = 480;
const PAD_THRESHOLD: f32 = 0.85;

fn spoof_without(pose: bool, embedding: bool) -> VisionAnalysis {
    let mut vector = vec![0.0f32; 512];
    vector[0] = 1.0;
    VisionAnalysis {
        rgb: Zeroizing::new(Vec::new()),
        detections: vec![FaceDetection::new(
            BoundingBox::new(220.0, 140.0, 420.0, 340.0),
            0.95,
        )],
        pad_result: Some(PadResult::spoof(0.05, AttackType::ScreenReplay)),
        pose: pose.then_some(HeadPose {
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
        }),
        aligned_crop: None,
        embedding: embedding.then(|| Zeroizing::new(vector)),
        quality_rejection: None,
    }
}

#[test]
fn test_spoof_without_embedding_counts_as_spoof_event() {
    let mut session = new_guided_enrollment_session();
    assert_eq!(
        feed_guided_enrollment(
            &mut session,
            &spoof_without(true, false),
            W,
            H,
            PAD_THRESHOLD
        ),
        Some(EnrollmentStepFeedback::SpoofDetected)
    );
    assert_eq!(session.spoof_events(), 1);
}

#[test]
fn test_spoof_without_pose_counts_as_spoof_event() {
    let mut session = new_guided_enrollment_session();
    assert_eq!(
        feed_guided_enrollment(
            &mut session,
            &spoof_without(false, true),
            W,
            H,
            PAD_THRESHOLD
        ),
        Some(EnrollmentStepFeedback::SpoofDetected)
    );
    assert_eq!(session.spoof_events(), 1);
}

#[test]
fn test_repeated_spoofs_without_embedding_abort_the_session() {
    let mut session = new_guided_enrollment_session();
    let max = LivenessPolicy::strict().max_spoof_events;
    let mut last = None;
    for _ in 0..max {
        last = feed_guided_enrollment(
            &mut session,
            &spoof_without(true, false),
            W,
            H,
            PAD_THRESHOLD,
        );
    }
    assert_eq!(last, Some(EnrollmentStepFeedback::SessionAborted));
    assert!(session.is_aborted());
    assert!(session.compute_composite_embedding().is_err());
}

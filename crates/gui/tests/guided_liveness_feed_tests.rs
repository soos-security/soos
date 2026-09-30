//! Contractual tests for the GUI guided-enrollment feed (GitHub #217 / #218):
//! the GUI session uses the strict session-level liveness policy, frames without a PAD
//! verdict break the live streak without counting as spoofs, and quality-gated faces
//! are never sampled.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses direct assertions and synthetic fixtures"
)]

use soos_enrollment_cli::guided_enrollment::{
    EnrollmentStep, EnrollmentStepFeedback, DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES,
};
use soos_gui::worker::{feed_guided_enrollment, new_guided_enrollment_session};
use soos_inference_ort::{AttackType, BoundingBox, FaceDetection, PadResult};
use soos_vision::pose::HeadPose;
use soos_vision::{FaceQualityRejection, VisionAnalysis};
use zeroize::Zeroizing;

const W: u32 = 640;
const H: u32 = 480;
const PAD_THRESHOLD: f32 = 0.85;

fn analysis(
    pad_result: Option<PadResult>,
    quality_rejection: Option<FaceQualityRejection>,
) -> VisionAnalysis {
    let mut embedding = vec![0.0f32; 512];
    embedding[0] = 1.0;
    let with_embedding = quality_rejection.is_none();
    VisionAnalysis {
        rgb: Zeroizing::new(Vec::new()),
        detections: vec![FaceDetection::new(
            BoundingBox::new(220.0, 140.0, 420.0, 340.0),
            0.95,
        )],
        pad_result,
        pose: Some(HeadPose {
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
        }),
        aligned_crop: None,
        embedding: with_embedding.then(|| Zeroizing::new(embedding)),
        quality_rejection,
    }
}

fn live() -> VisionAnalysis {
    analysis(Some(PadResult::live(0.98)), None)
}

#[test]
fn test_gui_session_requires_consecutive_live_frames() {
    let mut session = new_guided_enrollment_session();
    for _ in 1..DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES {
        assert_eq!(
            feed_guided_enrollment(&mut session, &live(), W, H, PAD_THRESHOLD),
            Some(EnrollmentStepFeedback::PromptHoldStill)
        );
    }
    assert!(matches!(
        feed_guided_enrollment(&mut session, &live(), W, H, PAD_THRESHOLD),
        Some(EnrollmentStepFeedback::SampleAccepted {
            step: EnrollmentStep::Frontal,
            collected: 1,
            ..
        })
    ));
}

#[test]
fn test_missing_pad_verdict_breaks_streak_without_spoof_event() {
    let mut session = new_guided_enrollment_session();
    for _ in 1..DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES {
        feed_guided_enrollment(&mut session, &live(), W, H, PAD_THRESHOLD);
    }
    assert_eq!(
        feed_guided_enrollment(&mut session, &analysis(None, None), W, H, PAD_THRESHOLD),
        Some(EnrollmentStepFeedback::PromptHoldStill)
    );
    assert_eq!(session.spoof_events(), 0);
    // The streak restarted: the next live frame is not sampled.
    assert_eq!(
        feed_guided_enrollment(&mut session, &live(), W, H, PAD_THRESHOLD),
        Some(EnrollmentStepFeedback::PromptHoldStill)
    );
    assert_eq!(session.progress_percent(), 0.0);
}

#[test]
fn test_quality_rejected_face_is_never_sampled() {
    let mut session = new_guided_enrollment_session();
    for _ in 1..DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES {
        feed_guided_enrollment(&mut session, &live(), W, H, PAD_THRESHOLD);
    }
    assert_eq!(
        feed_guided_enrollment(
            &mut session,
            &analysis(None, Some(FaceQualityRejection::TooSmall)),
            W,
            H,
            PAD_THRESHOLD
        ),
        Some(EnrollmentStepFeedback::FaceQualityTooLow)
    );
    assert_eq!(
        feed_guided_enrollment(&mut session, &live(), W, H, PAD_THRESHOLD),
        Some(EnrollmentStepFeedback::PromptHoldStill)
    );
    assert_eq!(session.progress_percent(), 0.0);
}

#[test]
fn test_spoof_and_below_threshold_frames_count_as_spoof_events() {
    let mut session = new_guided_enrollment_session();
    assert_eq!(
        feed_guided_enrollment(
            &mut session,
            &analysis(Some(PadResult::spoof(0.1, AttackType::PrintPhoto)), None),
            W,
            H,
            PAD_THRESHOLD
        ),
        Some(EnrollmentStepFeedback::SpoofDetected)
    );
    assert_eq!(
        feed_guided_enrollment(
            &mut session,
            &analysis(Some(PadResult::live(0.50)), None),
            W,
            H,
            PAD_THRESHOLD
        ),
        Some(EnrollmentStepFeedback::SpoofDetected)
    );
    assert_eq!(session.spoof_events(), 2);
}

#[test]
fn test_frame_without_face_breaks_streak_and_reports_nothing() {
    let mut session = new_guided_enrollment_session();
    for _ in 1..DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES {
        feed_guided_enrollment(&mut session, &live(), W, H, PAD_THRESHOLD);
    }
    let no_face = VisionAnalysis {
        rgb: Zeroizing::new(Vec::new()),
        detections: Vec::new(),
        pad_result: None,
        pose: None,
        aligned_crop: None,
        embedding: None,
        quality_rejection: None,
    };
    assert_eq!(
        feed_guided_enrollment(&mut session, &no_face, W, H, PAD_THRESHOLD),
        None
    );
    assert_eq!(
        feed_guided_enrollment(&mut session, &live(), W, H, PAD_THRESHOLD),
        Some(EnrollmentStepFeedback::PromptHoldStill)
    );
}

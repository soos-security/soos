#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::quality::{select_best_frame, CandidateEvaluation};
use soos_inference_ort::{BoundingBox, FaceDetection};

fn make_candidate(frame_idx: usize, score: f32, face_count: usize) -> CandidateEvaluation {
    let detections = match face_count {
        0 => vec![],
        1 => vec![FaceDetection {
            box_: BoundingBox::new(10.0, 10.0, 100.0, 100.0),
            score,
        }],
        n => (0..n)
            .map(|i| FaceDetection {
                box_: BoundingBox::new(10.0 * (i as f32 + 1.0), 10.0, 50.0, 50.0),
                score,
            })
            .collect(),
    };

    CandidateEvaluation {
        frame_idx,
        detections,
    }
}

#[test]
fn test_quality_selects_highest_scoring_single_face() {
    let candidates = vec![
        make_candidate(0, 0.72, 1),
        make_candidate(1, 0.95, 1), // Best
        make_candidate(2, 0.81, 1),
    ];

    let best = select_best_frame(&candidates, 0.70).expect("Should select best frame");
    assert_eq!(best.frame_idx, 1);
    assert!((best.score - 0.95).abs() < f32::EPSILON);
}

#[test]
fn test_quality_ignores_multi_face_or_zero_face_frames() {
    let candidates = vec![
        make_candidate(0, 0.99, 2), // 2 faces -> rejected
        make_candidate(1, 0.0, 0),  // 0 faces -> rejected
        make_candidate(2, 0.85, 1), // 1 face -> valid and best
        make_candidate(3, 0.98, 3), // 3 faces -> rejected
    ];

    let best = select_best_frame(&candidates, 0.70).expect("Should select valid candidate");
    assert_eq!(best.frame_idx, 2);
    assert!((best.score - 0.85).abs() < f32::EPSILON);
}

#[test]
fn test_quality_fails_when_all_frames_below_confidence() {
    let candidates = vec![make_candidate(0, 0.50, 1), make_candidate(1, 0.65, 1)];

    let err = select_best_frame(&candidates, 0.70).unwrap_err();
    match err {
        EnrollmentCliError::LowQualityFrames { evaluated, valid } => {
            assert_eq!(evaluated, 2);
            assert_eq!(valid, 0);
        }
        other => panic!("Expected LowQualityFrames error, got: {other:?}"),
    }
}

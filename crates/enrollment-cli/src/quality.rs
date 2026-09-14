//! Multi-frame quality assessment and candidate selection for enrollment.

use crate::error::EnrollmentCliError;
use soos_inference_ort::FaceDetection;

/// Candidate frame evaluation containing index and detected faces.
#[derive(Debug, Clone)]
pub struct CandidateEvaluation {
    pub frame_idx: usize,
    pub detections: Vec<FaceDetection>,
}

/// Selected best candidate frame details.
#[derive(Debug, Clone)]
pub struct BestCandidate {
    pub frame_idx: usize,
    pub score: f32,
    pub detection: FaceDetection,
}

/// Selects the highest quality candidate frame that strictly meets the single-face invariant
/// and exceeds the minimum detection confidence threshold.
pub fn select_best_frame(
    candidates: &[CandidateEvaluation],
    min_confidence: f32,
) -> Result<BestCandidate, EnrollmentCliError> {
    let mut valid_candidates: Vec<BestCandidate> = Vec::new();

    for cand in candidates {
        if cand.detections.len() == 1 {
            if let Some(det) = cand.detections.first() {
                if det.score >= min_confidence {
                    valid_candidates.push(BestCandidate {
                        frame_idx: cand.frame_idx,
                        score: det.score,
                        detection: det.clone(),
                    });
                }
            }
        }
    }

    if valid_candidates.is_empty() {
        return Err(EnrollmentCliError::LowQualityFrames {
            evaluated: candidates.len(),
            valid: 0,
        });
    }

    // Sort descending by detection score
    valid_candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    valid_candidates
        .into_iter()
        .next()
        .ok_or(EnrollmentCliError::LowQualityFrames {
            evaluated: candidates.len(),
            valid: 0,
        })
}

//! Cosine similarity biometric feature matching.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "Vector dot product, Euclidean norm computation, and score threshold comparison"
)]

use soos_inference_ort::BiometricEmbedding;

use crate::error::VisionError;

/// Result of a biometric template matching comparison.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchResult {
    /// True if similarity score meets or exceeds the configured threshold.
    pub matched: bool,
    /// Cosine similarity score in range `[-1.0, 1.0]`.
    pub score: f32,
    /// The threshold score used for the match decision.
    pub threshold: f32,
}

/// Computes the cosine similarity between two feature slices:
/// `similarity = dot(a, b) / (||a|| * ||b||)` clamped to `[-1.0, 1.0]`.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Result<f32, VisionError> {
    if a.len() != b.len() {
        return Err(VisionError::DimensionMismatch {
            expected: a.len(),
            actual: b.len(),
        });
    }

    if a.is_empty() {
        return Err(VisionError::DegenerateEmbedding(
            "Empty vector slice provided for cosine similarity".to_string(),
        ));
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum();
    let norm_a_sq: f32 = a.iter().map(|&x| x * x).sum();
    let norm_b_sq: f32 = b.iter().map(|&x| x * x).sum();

    if norm_a_sq <= 1e-12 || norm_b_sq <= 1e-12 {
        return Err(VisionError::DegenerateEmbedding(
            "Zero or near-zero Euclidean norm in embedding vector".to_string(),
        ));
    }

    let norm_product = (norm_a_sq * norm_b_sq).sqrt();
    if norm_product <= 1e-12 {
        return Err(VisionError::DegenerateEmbedding(
            "Zero norm product during cosine calculation".to_string(),
        ));
    }

    let similarity = (dot / norm_product).clamp(-1.0, 1.0);
    Ok(similarity)
}

/// Matches a candidate embedding against an enrolled biometric template.
pub fn match_embeddings(
    enrolled: &BiometricEmbedding,
    candidate: &BiometricEmbedding,
    threshold: f32,
) -> Result<MatchResult, VisionError> {
    let score = cosine_similarity(enrolled.as_slice(), candidate.as_slice())?;
    let matched = score >= threshold;
    Ok(MatchResult {
        matched,
        score,
        threshold,
    })
}

//! Multi-scale Presentation Attack Detection fusion (review finding PAD-07, GitHub #212).
//!
//! Upstream Silent-Face-Anti-Spoofing sums the softmax vectors of its 2.7-scale MiniFASNetV2
//! and 4.0-scale MiniFASNetV1SE and divides by the number of models. Each member's
//! [`PadResult::score`] is its softmax live probability, so the fused live probability is the
//! arithmetic mean of the member scores. soos then applies its own threshold (0.85 colour,
//! stricter for IR), which is at least 0.5 and therefore implies the upstream argmax-live
//! condition: the fused decision is never looser than upstream.

use soos_inference_ort::{AttackType, PadResult};

/// Fuses per-model PAD results into a single decision.
///
/// - No member: `None` (callers treat this as a failure).
/// - One member: returned unchanged (single-model behaviour, bit-identical to before fusion).
/// - Several members: `score` is the mean of the member scores; `is_live` is
///   `score.is_finite() && score >= threshold` (a NaN/inf member score or a NaN threshold
///   rejects). A rejected result carries the attack type of the lowest-scoring member, or
///   [`AttackType::UnknownSpoof`] when that member reports none.
pub fn fuse_pad_results(results: &[PadResult], threshold: f32) -> Option<PadResult> {
    match results {
        [] => None,
        [single] => Some(single.clone()),
        members => {
            let count = f32::from(u16::try_from(members.len()).unwrap_or(u16::MAX));
            let sum: f32 = members.iter().map(|r| r.score).sum();
            let score = sum / count;
            if score.is_finite() && score >= threshold {
                return Some(PadResult::live(score));
            }
            let attack = members
                .iter()
                .min_by(|a, b| a.score.total_cmp(&b.score))
                .and_then(|r| r.attack_type)
                .unwrap_or(AttackType::UnknownSpoof);
            Some(PadResult::spoof(score, attack))
        }
    }
}

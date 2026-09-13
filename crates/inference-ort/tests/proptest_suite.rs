//! Adversarial property-based test suite for geometry, NMS, and biometric embedding math.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use proptest::prelude::*;
use soos_inference_ort::detector::{nms, BoundingBox, FaceDetection};
use soos_inference_ort::embedding::BiometricEmbedding;

proptest! {
    // 1. IoU mathematical invariants
    #[test]
    fn prop_iou_bounded_and_symmetric(
        x1 in 0.0f32..500.0,
        y1 in 0.0f32..500.0,
        w1 in 1.0f32..200.0,
        h1 in 1.0f32..200.0,
        x2 in 0.0f32..500.0,
        y2 in 0.0f32..500.0,
        w2 in 1.0f32..200.0,
        h2 in 1.0f32..200.0,
    ) {
        let box1 = BoundingBox::new(x1, y1, x1 + w1, y1 + h1);
        let box2 = BoundingBox::new(x2, y2, x2 + w2, y2 + h2);

        let iou12 = box1.iou(&box2);
        let iou21 = box2.iou(&box1);

        // Bounded in [0.0, 1.0]
        prop_assert!((0.0..=1.0).contains(&iou12));
        // Symmetric
        prop_assert!((iou12 - iou21).abs() < 1e-6);
        // Self IoU is 1.0
        prop_assert!((box1.iou(&box1) - 1.0).abs() < 1e-6);
    }

    // 2. NMS property invariants
    #[test]
    fn prop_nms_invariants(
        scores in prop::collection::vec(0.1f32..0.99, 1..20),
        coords in prop::collection::vec((0.0f32..400.0, 0.0f32..400.0, 10.0f32..100.0, 10.0f32..100.0), 1..20),
        threshold in 0.2f32..0.8,
    ) {
        let count = scores.len().min(coords.len());
        let mut candidates = Vec::with_capacity(count);

        for i in 0..count {
            let (x, y, w, h) = coords[i];
            let s = scores[i];
            candidates.push(FaceDetection::new(BoundingBox::new(x, y, x + w, y + h), s));
        }

        let suppressed = nms(&candidates, threshold);

        // Invariant A: result length <= input length
        prop_assert!(suppressed.len() <= candidates.len());

        // Invariant B: no pair in the result exceeds the IoU threshold
        for i in 0..suppressed.len() {
            for j in (i + 1)..suppressed.len() {
                let iou = suppressed[i].box_.iou(&suppressed[j].box_);
                prop_assert!(iou <= threshold + 1e-5);
            }
        }
    }

    // 3. Biometric embedding normalization (Criterion V2)
    #[test]
    fn prop_embedding_normalization_criterion_v2(
        vals in prop::collection::vec(-10.0f32..10.0, 128),
    ) {
        // Filter out completely zero vector
        let sum_abs: f32 = vals.iter().map(|&x| x.abs()).sum();
        if sum_abs > 1e-4 {
            let mut emb = BiometricEmbedding::new(vals);
            let norm_result = emb.normalize();
            prop_assert!(norm_result.is_ok());

            // Criterion V2 invariant: norm ≈ 1.0
            let norm = emb.l2_norm();
            prop_assert!((norm - 1.0).abs() < 1e-5, "Norm {} deviates from 1.0", norm);
            prop_assert!(emb.is_normalized(1e-4));
        }
    }

    // 4. Cosine similarity mathematical invariants
    #[test]
    fn prop_cosine_similarity_invariants(
        vals1 in prop::collection::vec(-5.0f32..5.0, 128),
        vals2 in prop::collection::vec(-5.0f32..5.0, 128),
    ) {
        let sum1: f32 = vals1.iter().map(|&x| x.abs()).sum();
        let sum2: f32 = vals2.iter().map(|&x| x.abs()).sum();

        if sum1 > 1e-4 && sum2 > 1e-4 {
            let mut emb1 = BiometricEmbedding::new(vals1);
            let mut emb2 = BiometricEmbedding::new(vals2);
            let _ = emb1.normalize();
            let _ = emb2.normalize();

            let sim12 = emb1.cosine_similarity(&emb2).unwrap();
            let sim21 = emb2.cosine_similarity(&emb1).unwrap();

            // Range [-1.0, 1.0]
            prop_assert!((-1.0 - 1e-6..=1.0 + 1e-6).contains(&sim12));
            // Symmetry
            prop_assert!((sim12 - sim21).abs() < 1e-5);
            // Self-similarity
            let sim_self = emb1.cosine_similarity(&emb1).unwrap();
            prop_assert!((sim_self - 1.0).abs() < 1e-5);
        }
    }
}

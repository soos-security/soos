//! SCRFD decoding robustness and scratch-buffer contracts (GitHub #252, #254).
//!
//! - #254 (VIS-12): candidates with a non-finite score, box or keypoint (or whose decoded
//!   coordinates overflow to infinity) must be skipped by `decode_stride`, never emitted with
//!   NaN landmarks (`f32::clamp` propagates NaN).
//! - #252 (VIS-10): the detector reuses one letterbox scratch tensor per detector; the
//!   in-place letterbox must be bit-identical to the allocating one even when the scratch
//!   still holds a previous frame.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Test suite utilizes direct assertions and fixture arithmetic"
)]

use soos_inference_ort::detector::{letterbox_pad, letterbox_pad_into, OrtScrfdDetector};
use soos_inference_ort::InferenceError;

/// One-cell, one-anchor grid: a valid candidate.
fn valid_inputs() -> ([f32; 1], [f32; 4], [f32; 10]) {
    (
        [0.9],
        [1.0, 1.0, 1.0, 1.0],
        [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0],
    )
}

fn decode(scores: &[f32], bboxes: &[f32], kps: &[f32]) -> usize {
    let detections = OrtScrfdDetector::decode_stride(
        8, 1, 1, 1, scores, bboxes, kps, 0.5, 1.0, 0.0, 0.0, 640, 640,
    );
    for det in &detections {
        assert!(det.score.is_finite(), "emitted non-finite score");
        let b = det.box_;
        assert!(
            b.x1.is_finite() && b.y1.is_finite() && b.x2.is_finite() && b.y2.is_finite(),
            "emitted non-finite box {b:?}"
        );
        if let Some(lm) = det.landmarks {
            for p in lm.as_array() {
                assert!(
                    p.x.is_finite() && p.y.is_finite(),
                    "emitted non-finite landmark"
                );
            }
        }
    }
    detections.len()
}

#[test]
fn test_decode_stride_keeps_valid_candidate() {
    let (s, b, k) = valid_inputs();
    assert_eq!(decode(&s, &b, &k), 1);
}

#[test]
fn test_decode_stride_skips_non_finite() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let (mut s, b, k) = valid_inputs();
        s[0] = bad;
        assert_eq!(decode(&s, &b, &k), 0, "score {bad} must be skipped");

        for i in 0..4 {
            let (s, mut b, k) = valid_inputs();
            b[i] = bad;
            assert_eq!(decode(&s, &b, &k), 0, "bbox[{i}] = {bad} must be skipped");
        }

        for i in 0..10 {
            let (s, b, mut k) = valid_inputs();
            k[i] = bad;
            assert_eq!(decode(&s, &b, &k), 0, "kps[{i}] = {bad} must be skipped");
        }
    }
}

#[test]
fn test_decode_stride_skips_overflowing_coordinates() {
    // Finite raw values whose decoded coordinates overflow f32 (x * stride = inf).
    let (s, mut b, k) = valid_inputs();
    b[2] = f32::MAX;
    assert_eq!(decode(&s, &b, &k), 0, "overflowing bbox must be skipped");

    let (s, b, mut k) = valid_inputs();
    k[3] = f32::MAX;
    assert_eq!(
        decode(&s, &b, &k),
        0,
        "overflowing keypoint must be skipped"
    );
}

fn frame(width: u32, height: u32, seed: u8) -> Vec<u8> {
    (0..(width as usize * height as usize * 3))
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

#[test]
fn test_letterbox_pad_into_matches_letterbox_pad_with_dirty_scratch() {
    let target = 64;
    let mut scratch = vec![9.75f32; 3 * target * target];

    for (w, h, seed) in [(80u32, 40u32, 1u8), (30, 60, 2), (64, 64, 3), (17, 5, 4)] {
        let rgb = frame(w, h, seed);
        let (expected, e_scale, e_px, e_py) = letterbox_pad(&rgb, w, h, target);
        let (scale, px, py) =
            letterbox_pad_into(&rgb, w, h, target, &mut scratch).expect("scratch has the size");
        assert_eq!(
            (scale, px, py),
            (e_scale, e_px, e_py),
            "geometry for {w}x{h}"
        );
        assert_eq!(
            scratch.as_slice(),
            expected.as_slice(),
            "stale scratch data leaked into the {w}x{h} tensor"
        );
    }
}

#[test]
fn test_letterbox_pad_into_invalid_frame_yields_zero_tensor() {
    let target = 16;
    let mut scratch = vec![1.0f32; 3 * target * target];
    let (scale, px, py) =
        letterbox_pad_into(&[0u8; 5], 4, 4, target, &mut scratch).expect("scratch has the size");
    assert_eq!((scale, px, py), (1.0, 0.0, 0.0));
    assert!(scratch.iter().all(|&v| v == 0.0));
}

#[test]
fn test_letterbox_pad_into_rejects_wrong_scratch_length() {
    let mut scratch = vec![0.0f32; 10];
    let rgb = frame(4, 4, 0);
    let result = letterbox_pad_into(&rgb, 4, 4, 16, &mut scratch);
    assert!(
        matches!(
            result,
            Err(InferenceError::InvalidBufferSize {
                expected: 768,
                actual: 10
            })
        ),
        "got {result:?}"
    );
}

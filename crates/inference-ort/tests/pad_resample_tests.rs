//! PAD input resampling when the crop is not already 80x80 (review finding PAD-08,
//! GitHub #213): `OrtPadDetector::prepare_input` uses half-pixel centre bilinear sampling
//! (the `cv2.resize` `INTER_LINEAR` convention the model was trained with) instead of
//! top-left nearest-neighbour. An 80x80 crop stays an exact identity copy.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "Contractual test suite uses assertions, unwrap and tensor indexing"
)]

use soos_inference_ort::pad::OrtPadDetector;

const PLANE: usize = 80 * 80;

/// RGB crop whose red channel is `x` and green channel is `y` (linear ramps).
fn ramp(edge: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((edge * edge * 3) as usize);
    for y in 0..edge {
        for x in 0..edge {
            rgb.extend_from_slice(&[x as u8, y as u8, 200]);
        }
    }
    rgb
}

#[test]
fn test_pad_prepare_input_downscale_uses_half_pixel_centres() {
    // 160 -> 80: output x samples source 2x + 0.5, so R = 2x + 0.5 (raw [0, 255] range).
    let tensor = OrtPadDetector::prepare_input(&ramp(160), 160, 160).expect("prepare_input");
    for y in 0..80usize {
        for x in 0..80usize {
            let r = tensor[2 * PLANE + y * 80 + x];
            let g = tensor[PLANE + y * 80 + x];
            let b = tensor[y * 80 + x];
            let expected_r = 2.0 * x as f32 + 0.5;
            let expected_g = 2.0 * y as f32 + 0.5;
            assert!((r - expected_r).abs() < 1e-5, "R({x},{y}) = {r}");
            assert!((g - expected_g).abs() < 1e-5, "G({x},{y}) = {g}");
            assert!((b - 200.0).abs() < 1e-6, "B({x},{y}) = {b}");
        }
    }
}

#[test]
fn test_pad_prepare_input_upscale_clamps_to_border() {
    // 40 -> 80: output x samples source (x + 0.5) / 2 - 0.5, clamped to [0, 39].
    let tensor = OrtPadDetector::prepare_input(&ramp(40), 40, 40).expect("prepare_input");
    for x in 0..80usize {
        let r = tensor[2 * PLANE + x];
        let expected = ((x as f32 + 0.5) * 0.5 - 0.5).clamp(0.0, 39.0);
        assert!(
            (r - expected).abs() < 1e-5,
            "R({x},0) = {r}, expected {expected}"
        );
    }
}

#[test]
fn test_pad_prepare_input_80x80_is_identity() {
    let tensor = OrtPadDetector::prepare_input(&ramp(80), 80, 80).expect("prepare_input");
    for y in 0..80usize {
        for x in 0..80usize {
            assert_eq!(tensor[2 * PLANE + y * 80 + x], x as f32);
            assert_eq!(tensor[PLANE + y * 80 + x], y as f32);
        }
    }
}

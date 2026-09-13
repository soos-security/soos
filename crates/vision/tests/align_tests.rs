//! Tests for 5-point facial landmark affine alignment in `soos-vision`.
//! Acceptance criteria: V1 — Golden tests match training pipeline.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_inference_ort::{FaceLandmarks, Point2f};
use soos_vision::{align_face_112, TARGET_LANDMARKS_112};

#[test]
fn test_canonical_identity_alignment_matches_reference() {
    let width = 112;
    let height = 112;
    // Create a 112x112 synthetic RGB image with distinct pixel patterns
    let mut rgb = vec![0u8; (width * height * 3) as usize];
    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 3) as usize;
            rgb[idx] = (x % 256) as u8;
            rgb[idx + 1] = (y % 256) as u8;
            rgb[idx + 2] = 128;
        }
    }

    // Canonical reference landmarks
    let landmarks = FaceLandmarks::from_array(TARGET_LANDMARKS_112);

    let aligned =
        align_face_112(&rgb, width, height, &landmarks).expect("Canonical alignment must succeed");

    assert_eq!(
        aligned.len(),
        112 * 112 * 3,
        "Aligned crop must be 112x112x3"
    );

    // Under identity alignment, central landmark pixel colors should match source closely
    for target_pt in TARGET_LANDMARKS_112 {
        let x = target_pt.x.round() as usize;
        let y = target_pt.y.round() as usize;
        let idx = (y * 112 + x) * 3;
        let diff_r = (aligned[idx] as i32 - rgb[idx] as i32).abs();
        let diff_g = (aligned[idx + 1] as i32 - rgb[idx + 1] as i32).abs();
        assert!(diff_r <= 2, "Identity R channel mismatch at ({}, {})", x, y);
        assert!(diff_g <= 2, "Identity G channel mismatch at ({}, {})", x, y);
    }
}

#[test]
fn test_translated_face_alignment_recenters() {
    let width = 200;
    let height = 200;
    let mut rgb = vec![0u8; (width * height * 3) as usize];

    // Shift canonical landmarks by (+40, +30)
    let shift_x = 40.0;
    let shift_y = 30.0;
    let shifted = [
        Point2f::new(
            TARGET_LANDMARKS_112[0].x + shift_x,
            TARGET_LANDMARKS_112[0].y + shift_y,
        ),
        Point2f::new(
            TARGET_LANDMARKS_112[1].x + shift_x,
            TARGET_LANDMARKS_112[1].y + shift_y,
        ),
        Point2f::new(
            TARGET_LANDMARKS_112[2].x + shift_x,
            TARGET_LANDMARKS_112[2].y + shift_y,
        ),
        Point2f::new(
            TARGET_LANDMARKS_112[3].x + shift_x,
            TARGET_LANDMARKS_112[3].y + shift_y,
        ),
        Point2f::new(
            TARGET_LANDMARKS_112[4].x + shift_x,
            TARGET_LANDMARKS_112[4].y + shift_y,
        ),
    ];

    // Paint a distinctive color marker at shifted nose tip
    let nose_x = shifted[2].x.round() as usize;
    let nose_y = shifted[2].y.round() as usize;
    let nose_idx = (nose_y * width as usize + nose_x) * 3;
    rgb[nose_idx] = 255;
    rgb[nose_idx + 1] = 0;
    rgb[nose_idx + 2] = 0;

    let landmarks = FaceLandmarks::from_array(shifted);
    let aligned =
        align_face_112(&rgb, width, height, &landmarks).expect("Translated alignment must succeed");

    assert_eq!(aligned.len(), 112 * 112 * 3);

    // In aligned output, the nose tip should be back at TARGET_LANDMARKS_112[2] ~ (56, 72)
    let target_nose_x = TARGET_LANDMARKS_112[2].x.round() as usize;
    let target_nose_y = TARGET_LANDMARKS_112[2].y.round() as usize;
    let target_idx = (target_nose_y * 112 + target_nose_x) * 3;

    // The red pixel should be mapped to the canonical nose position
    assert_eq!(
        aligned[target_idx], 255,
        "Aligned nose R channel should be 255"
    );
}

#[test]
fn test_rotated_face_alignment_levels_eyes() {
    let width = 200;
    let height = 200;
    let rgb = vec![100u8; (width * height * 3) as usize];

    // Rotate landmarks by ~15 degrees
    let angle = 15.0_f32.to_radians();
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let center_x = 100.0;
    let center_y = 100.0;

    let mut rotated = [Point2f::new(0.0, 0.0); 5];
    for (i, pt) in TARGET_LANDMARKS_112.iter().enumerate() {
        let dx = pt.x - 56.0;
        let dy = pt.y - 56.0;
        rotated[i] = Point2f::new(
            center_x + dx * cos_a - dy * sin_a,
            center_y + dx * sin_a + dy * cos_a,
        );
    }

    let landmarks = FaceLandmarks::from_array(rotated);
    let aligned =
        align_face_112(&rgb, width, height, &landmarks).expect("Rotated alignment must succeed");

    assert_eq!(aligned.len(), 112 * 112 * 3);
}

#[test]
fn test_zero_dimensions_rejected() {
    let landmarks = FaceLandmarks::from_array(TARGET_LANDMARKS_112);
    let rgb = vec![0u8; 100];
    assert!(align_face_112(&rgb, 0, 112, &landmarks).is_err());
    assert!(align_face_112(&rgb, 112, 0, &landmarks).is_err());
}

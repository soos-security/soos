//! Contractual test suite for letterbox resize and coordinate projection in `soos-vision`.
//!
//! Enforces:
//! - Sub-issue #42.1: `letterbox_resize()` utility in `vision` crate
//! - Verification Matrix Criterion NGM14: Letterbox padding preserves aspect ratio
//!   with correct coordinate un-projection (property test).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "Contractual test suite uses assertions and unwrap"
)]

use proptest::prelude::*;
use soos_inference_ort::BoundingBox;
use soos_vision::letterbox::{letterbox_params, letterbox_resize, LetterboxParams};
use soos_vision::VisionError;

#[test]
fn test_letterbox_640x480_to_640x640() {
    // 640x480 to 640x640:
    // Scale should be 1.0 (640/640 = 1.0 < 640/480 = 1.333...)
    // Scaled size: 640x480
    // Padding: pad_x = 0.0, pad_y = (640 - 480) / 2.0 = 80.0
    let params = letterbox_params(640, 480, 640, 640).expect("Params computation should succeed");
    assert!((params.scale - 1.0).abs() < 1e-5);
    assert_eq!(params.pad_x, 0.0);
    assert_eq!(params.pad_y, 80.0);

    // Create a 640x480 white image (RGB = 255, 255, 255)
    let w = 640u32;
    let h = 480u32;
    let rgb = vec![255u8; (w * h * 3) as usize];

    let (resized, out_params) =
        letterbox_resize(&rgb, w, h, 640, 640).expect("Letterbox resize should succeed");
    assert_eq!(out_params, params);
    assert_eq!(resized.len(), 640 * 640 * 3);

    // Top padding region: rows 0..80 should be black (0)
    for y in 0..80 {
        for x in 0..640 {
            let idx = (y * 640 + x) * 3;
            assert_eq!(resized[idx], 0, "Top padding at ({x},{y}) R must be 0");
            assert_eq!(resized[idx + 1], 0, "Top padding at ({x},{y}) G must be 0");
            assert_eq!(resized[idx + 2], 0, "Top padding at ({x},{y}) B must be 0");
        }
    }

    // Active image region: rows 80..560 should be white (255)
    for y in 80..560 {
        for x in 0..640 {
            let idx = (y * 640 + x) * 3;
            assert_eq!(resized[idx], 255, "Active image at ({x},{y}) R must be 255");
            assert_eq!(
                resized[idx + 1],
                255,
                "Active image at ({x},{y}) G must be 255"
            );
            assert_eq!(
                resized[idx + 2],
                255,
                "Active image at ({x},{y}) B must be 255"
            );
        }
    }

    // Bottom padding region: rows 560..640 should be black (0)
    for y in 560..640 {
        for x in 0..640 {
            let idx = (y * 640 + x) * 3;
            assert_eq!(resized[idx], 0, "Bottom padding at ({x},{y}) R must be 0");
            assert_eq!(
                resized[idx + 1],
                0,
                "Bottom padding at ({x},{y}) G must be 0"
            );
            assert_eq!(
                resized[idx + 2],
                0,
                "Bottom padding at ({x},{y}) B must be 0"
            );
        }
    }
}

#[test]
fn test_letterbox_1280x720_to_640x640() {
    // 1280x720 to 640x640:
    // Scale: min(640/1280 = 0.5, 640/720 = 0.888...) = 0.5
    // Scaled dimensions: 1280 * 0.5 = 640, 720 * 0.5 = 360
    // Padding: pad_x = 0.0, pad_y = (640 - 360) / 2.0 = 140.0
    let params = letterbox_params(1280, 720, 640, 640).expect("Params computation should succeed");
    assert!((params.scale - 0.5).abs() < 1e-5);
    assert_eq!(params.pad_x, 0.0);
    assert_eq!(params.pad_y, 140.0);

    let w = 1280u32;
    let h = 720u32;
    let rgb = vec![128u8; (w * h * 3) as usize];

    let (resized, out_params) =
        letterbox_resize(&rgb, w, h, 640, 640).expect("Letterbox resize should succeed");
    assert_eq!(out_params, params);
    assert_eq!(resized.len(), 640 * 640 * 3);

    // Top padding: rows 0..140 black
    let idx_top = (70 * 640 + 320) * 3;
    assert_eq!(resized[idx_top], 0);

    // Active center: row 320 should be 128
    let idx_center = (320 * 640 + 320) * 3;
    assert_eq!(resized[idx_center], 128);

    // Bottom padding: rows 500..640 black
    let idx_bottom = (600 * 640 + 320) * 3;
    assert_eq!(resized[idx_bottom], 0);
}

#[test]
fn test_letterbox_square_no_padding() {
    // Square image 500x500 to 640x640:
    // Scale: 640 / 500 = 1.28
    // Scaled dimensions: 500 * 1.28 = 640x640
    // Padding: pad_x = 0.0, pad_y = 0.0
    let params = letterbox_params(500, 500, 640, 640).expect("Params computation should succeed");
    assert!((params.scale - 1.28).abs() < 1e-4);
    assert_eq!(params.pad_x, 0.0);
    assert_eq!(params.pad_y, 0.0);

    let w = 500u32;
    let h = 500u32;
    let rgb = vec![200u8; (w * h * 3) as usize];

    let (resized, out_params) =
        letterbox_resize(&rgb, w, h, 640, 640).expect("Letterbox resize should succeed");
    assert_eq!(out_params, params);
    assert_eq!(resized.len(), 640 * 640 * 3);

    // Corners should be 200, zero padding
    assert_eq!(resized[0], 200);
    assert_eq!(resized[resized.len() - 1], 200);
}

#[test]
fn test_letterbox_tall_portrait_padding() {
    // Tall portrait 480x640 to 640x640:
    // Scale: min(640/480 = 1.333..., 640/640 = 1.0) = 1.0
    // Scaled dimensions: 480x640
    // Padding: pad_x = (640 - 480) / 2.0 = 80.0, pad_y = 0.0
    let params = letterbox_params(480, 640, 640, 640).expect("Params computation should succeed");
    assert!((params.scale - 1.0).abs() < 1e-5);
    assert_eq!(params.pad_x, 80.0);
    assert_eq!(params.pad_y, 0.0);

    let w = 480u32;
    let h = 640u32;
    let rgb = vec![180u8; (w * h * 3) as usize];

    let (resized, out_params) =
        letterbox_resize(&rgb, w, h, 640, 640).expect("Letterbox resize should succeed");
    assert_eq!(out_params, params);

    // Left border: column 40 at row 320 should be black (0)
    let idx_left = (320 * 640 + 40) * 3;
    assert_eq!(resized[idx_left], 0);

    // Center: column 320 at row 320 should be 180
    let idx_center = (320 * 640 + 320) * 3;
    assert_eq!(resized[idx_center], 180);

    // Right border: column 600 at row 320 should be black (0)
    let idx_right = (320 * 640 + 600) * 3;
    assert_eq!(resized[idx_right], 0);
}

#[test]
fn test_letterbox_zero_dimensions_fail_closed() {
    assert!(matches!(
        letterbox_params(0, 480, 640, 640),
        Err(VisionError::InvalidDimensions {
            width: 0,
            height: 480
        })
    ));
    assert!(matches!(
        letterbox_params(640, 0, 640, 640),
        Err(VisionError::InvalidDimensions {
            width: 640,
            height: 0
        })
    ));
    assert!(matches!(
        letterbox_params(640, 480, 0, 640),
        Err(VisionError::InvalidDimensions {
            width: 0,
            height: 640
        })
    ));
    assert!(matches!(
        letterbox_params(640, 480, 640, 0),
        Err(VisionError::InvalidDimensions {
            width: 640,
            height: 0
        })
    ));

    let rgb = vec![0u8; 640 * 480 * 3];
    assert!(letterbox_resize(&rgb, 0, 480, 640, 640).is_err());
    assert!(letterbox_resize(&rgb, 640, 480, 0, 640).is_err());
}

#[test]
fn test_letterbox_buffer_length_mismatch_fails_closed() {
    let truncated_rgb = vec![0u8; 100];
    let res = letterbox_resize(&truncated_rgb, 640, 480, 640, 640);
    assert!(matches!(
        res,
        Err(VisionError::InvalidBufferSize {
            expected: 921600,
            actual: 100
        })
    ));
}

#[test]
fn test_letterbox_unproject_bbox() {
    // 1280x720 to 640x640: scale 0.5, pad_x 0.0, pad_y 140.0
    let params = LetterboxParams::new(0.5, 0.0, 140.0);

    // A detection bbox in letterbox space: x1=100, y1=240, x2=200, y2=340
    let letterbox_bbox = BoundingBox::new(100.0, 240.0, 200.0, 340.0);
    let orig_bbox = params.unproject_bbox(&letterbox_bbox);

    // Orig coords:
    // x1 = (100 - 0) / 0.5 = 200.0
    // y1 = (240 - 140) / 0.5 = 200.0
    // x2 = (200 - 0) / 0.5 = 400.0
    // y2 = (340 - 140) / 0.5 = 400.0
    assert!((orig_bbox.x1 - 200.0).abs() < 1e-4);
    assert!((orig_bbox.y1 - 200.0).abs() < 1e-4);
    assert!((orig_bbox.x2 - 400.0).abs() < 1e-4);
    assert!((orig_bbox.y2 - 400.0).abs() < 1e-4);
}

#[test]
fn test_letterbox_degenerate_scale_unproject_safe() {
    let params = LetterboxParams::new(0.0, 10.0, 10.0);
    let (x, y) = params.unproject(50.0, 50.0);
    assert_eq!((x, y), (0.0, 0.0));

    let neg_params = LetterboxParams::new(-1.0, 10.0, 10.0);
    let (x_neg, y_neg) = neg_params.unproject(50.0, 50.0);
    assert_eq!((x_neg, y_neg), (0.0, 0.0));
}

// Property Test for Criterion NGM14:
// Letterbox padding preserves aspect ratio with correct coordinate un-projection
proptest! {
    #[test]
    fn test_letterbox_unproject_roundtrip(
        w in 32u32..2560u32,
        h in 32u32..2560u32,
        target in 64u32..1024u32,
        pt_x in 0.0f32..2560.0f32,
        pt_y in 0.0f32..2560.0f32,
    ) {
        let params = letterbox_params(w, h, target, target).expect("Params must succeed for valid dims");
        prop_assert!(params.scale > 0.0);
        prop_assert!(params.pad_x >= 0.0);
        prop_assert!(params.pad_y >= 0.0);

        // Clamp test point to source image range
        let x = pt_x.min((w - 1) as f32);
        let y = pt_y.min((h - 1) as f32);

        // Project into letterbox canvas space
        let (lx, ly) = params.project(x, y);

        // Unproject back to original coordinate space
        let (orig_x, orig_y) = params.unproject(lx, ly);

        // Assert roundtrip equivalence within numerical tolerance
        prop_assert!(
            (orig_x - x).abs() < 1e-2,
            "X roundtrip mismatch: orig={orig_x}, expected={x}, scale={}, pad_x={}",
            params.scale, params.pad_x
        );
        prop_assert!(
            (orig_y - y).abs() < 1e-2,
            "Y roundtrip mismatch: orig={orig_y}, expected={y}, scale={}, pad_y={}",
            params.scale, params.pad_y
        );
    }
}

//! Contractual test suite for crop-and-resize and bounding box expansion utilities in `soos-vision`.
//!
//! Enforces:
//! - Sub-issue #42.2: `crop_and_resize()` utility in `vision` crate
//! - Verification of out-of-bounds black padding and bilinear interpolation accuracy

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

use soos_inference_ort::BoundingBox;
use soos_vision::crop::{crop_and_resize, expand_bbox_for_pad};
use soos_vision::VisionError;

#[test]
fn test_crop_and_resize_known_image() {
    // 4x4 known image with solid quadrants
    // Top-left: Red (255, 0, 0)
    // Top-right: Green (0, 255, 0)
    // Bottom-left: Blue (0, 0, 255)
    // Bottom-right: White (255, 255, 255)
    let w = 4u32;
    let h = 4u32;
    let mut rgb = vec![0u8; (w * h * 3) as usize];

    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) * 3) as usize;
            let (r, g, b) = match (x < 2, y < 2) {
                (true, true) => (255, 0, 0),       // Top-left: Red
                (false, true) => (0, 255, 0),      // Top-right: Green
                (true, false) => (0, 0, 255),      // Bottom-left: Blue
                (false, false) => (255, 255, 255), // Bottom-right: White
            };
            rgb[idx] = r;
            rgb[idx + 1] = g;
            rgb[idx + 2] = b;
        }
    }

    // Crop exactly the top-left quadrant (x: [0.0, 2.0], y: [0.0, 2.0]) and resize to 2x2
    let tl_bbox = BoundingBox::new(0.0, 0.0, 2.0, 2.0);
    let tl_cropped = crop_and_resize(&rgb, w, h, &tl_bbox, 2, 2)
        .expect("Crop of top-left quadrant should succeed");
    assert_eq!(tl_cropped.len(), 2 * 2 * 3);
    for i in 0..4 {
        assert_eq!(tl_cropped[i * 3], 255, "R channel must be 255");
        assert_eq!(tl_cropped[i * 3 + 1], 0, "G channel must be 0");
        assert_eq!(tl_cropped[i * 3 + 2], 0, "B channel must be 0");
    }

    // Crop exactly the top-right quadrant (x: [2.0, 4.0], y: [0.0, 2.0]) and resize to 2x2
    let tr_bbox = BoundingBox::new(2.0, 0.0, 4.0, 2.0);
    let tr_cropped = crop_and_resize(&rgb, w, h, &tr_bbox, 2, 2)
        .expect("Crop of top-right quadrant should succeed");
    for i in 0..4 {
        assert_eq!(tr_cropped[i * 3], 0);
        assert_eq!(tr_cropped[i * 3 + 1], 255);
        assert_eq!(tr_cropped[i * 3 + 2], 0);
    }

    // Crop exactly the bottom-left quadrant (x: [0.0, 2.0], y: [2.0, 4.0]) and resize to 2x2
    let bl_bbox = BoundingBox::new(0.0, 2.0, 2.0, 4.0);
    let bl_cropped = crop_and_resize(&rgb, w, h, &bl_bbox, 2, 2)
        .expect("Crop of bottom-left quadrant should succeed");
    for i in 0..4 {
        assert_eq!(bl_cropped[i * 3], 0);
        assert_eq!(bl_cropped[i * 3 + 1], 0);
        assert_eq!(bl_cropped[i * 3 + 2], 255);
    }
}

#[test]
fn test_crop_and_resize_out_of_bounds_padding() {
    // 4x4 solid white image
    let w = 4u32;
    let h = 4u32;
    let rgb = vec![255u8; (w * h * 3) as usize];

    // Crop box extends partially outside the image boundaries:
    // x: [-2.0, 2.0], y: [-2.0, 2.0]
    // The negative coordinates (-2 to 0) must be black (0) padding.
    // The valid coordinates (0 to 2) should sample the white image.
    let oob_bbox = BoundingBox::new(-2.0, -2.0, 2.0, 2.0);
    let cropped = crop_and_resize(&rgb, w, h, &oob_bbox, 4, 4)
        .expect("OOB crop should succeed with black padding");
    assert_eq!(cropped.len(), 4 * 4 * 3);

    // Pixel (0, 0) corresponds to (-2.0, -2.0) which is fully out of bounds -> black (0)
    assert_eq!(cropped[0], 0);
    assert_eq!(cropped[1], 0);
    assert_eq!(cropped[2], 0);

    // Pixel (3, 3) corresponds to (+1.0, +1.0) which is inside [0, 4] -> white (255)
    let idx_bottom_right = ((3 * 4 + 3) * 3) as usize;
    assert_eq!(cropped[idx_bottom_right], 255);
    assert_eq!(cropped[idx_bottom_right + 1], 255);
    assert_eq!(cropped[idx_bottom_right + 2], 255);
}

#[test]
fn test_crop_and_resize_zero_dimensions_rejected() {
    let rgb = vec![0u8; 100 * 100 * 3];
    let bbox = BoundingBox::new(10.0, 10.0, 50.0, 50.0);

    assert!(matches!(
        crop_and_resize(&rgb, 0, 100, &bbox, 50, 50),
        Err(VisionError::InvalidDimensions {
            width: 0,
            height: 100
        })
    ));
    assert!(matches!(
        crop_and_resize(&rgb, 100, 0, &bbox, 50, 50),
        Err(VisionError::InvalidDimensions {
            width: 100,
            height: 0
        })
    ));
    assert!(matches!(
        crop_and_resize(&rgb, 100, 100, &bbox, 0, 50),
        Err(VisionError::InvalidDimensions {
            width: 0,
            height: 50
        })
    ));
    assert!(matches!(
        crop_and_resize(&rgb, 100, 100, &bbox, 50, 0),
        Err(VisionError::InvalidDimensions {
            width: 50,
            height: 0
        })
    ));
}

#[test]
fn test_crop_and_resize_buffer_size_mismatch_rejected() {
    let wrong_rgb = vec![0u8; 50];
    let bbox = BoundingBox::new(10.0, 10.0, 50.0, 50.0);
    let res = crop_and_resize(&wrong_rgb, 100, 100, &bbox, 50, 50);
    assert!(matches!(
        res,
        Err(VisionError::InvalidBufferSize {
            expected: 30000,
            actual: 50
        })
    ));
}

#[test]
fn test_crop_and_resize_degenerate_bbox_returns_black() {
    let w = 10u32;
    let h = 10u32;
    let rgb = vec![255u8; (w * h * 3) as usize];

    // Inverted or zero-area bounding box
    let degenerate = BoundingBox::new(50.0, 50.0, 10.0, 10.0);
    let cropped = crop_and_resize(&rgb, w, h, &degenerate, 5, 5)
        .expect("Degenerate bbox should return black buffer");
    assert_eq!(cropped.len(), 5 * 5 * 3);
    assert!(cropped.iter().all(|&byte| byte == 0));
}

#[test]
fn test_expand_bbox_for_pad_expansion_and_clamping() {
    let bbox = BoundingBox::new(100.0, 100.0, 200.0, 200.0); // 100x100 box, center (150, 150)
    let expanded = expand_bbox_for_pad(&bbox, 2.7, 640, 480);

    // Expected size: 100 * 2.7 = 270x270, half_w = half_h = 135
    // x1 = 150 - 135 = 15.0
    // y1 = 150 - 135 = 15.0
    // x2 = 150 + 135 = 285.0
    // y2 = 150 + 135 = 285.0
    assert!((expanded.x1 - 15.0).abs() < 1e-4);
    assert!((expanded.y1 - 15.0).abs() < 1e-4);
    assert!((expanded.x2 - 285.0).abs() < 1e-4);
    assert!((expanded.y2 - 285.0).abs() < 1e-4);

    // Bounding box at edge clamped to [0, max]
    let edge_bbox = BoundingBox::new(0.0, 0.0, 50.0, 50.0);
    let edge_expanded = expand_bbox_for_pad(&edge_bbox, 2.7, 640, 480);
    assert_eq!(edge_expanded.x1, 0.0);
    assert_eq!(edge_expanded.y1, 0.0);
    assert!(edge_expanded.x2 <= 640.0);
    assert!(edge_expanded.y2 <= 480.0);
}

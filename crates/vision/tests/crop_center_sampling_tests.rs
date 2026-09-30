//! Half-pixel centre sampling of the generic `crop_and_resize` (review finding PAD-08,
//! GitHub #213): output pixel `u` samples `x1 + (u + 0.5) * sx - 0.5`, the `cv2.resize`
//! `INTER_LINEAR` convention, instead of the top-left aligned `x1 + u * sx`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite uses assertions, unwrap and pixel indexing"
)]

use soos_inference_ort::BoundingBox;
use soos_vision::crop::crop_and_resize;

fn ramp_row(width: u32, step: u8) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(width as usize * 3);
    for x in 0..width {
        let v = (x as u8).wrapping_mul(step);
        rgb.extend_from_slice(&[v, v, v]);
    }
    rgb
}

#[test]
fn test_crop_and_resize_downscale_samples_pixel_centres() {
    // 8x1 ramp 0, 10, ..., 70 resized to 4x1: centres at x = 0.5, 2.5, 4.5, 6.5.
    let rgb = ramp_row(8, 10);
    let out = crop_and_resize(&rgb, 8, 1, &BoundingBox::new(0.0, 0.0, 8.0, 1.0), 4, 1)
        .expect("crop succeeds");
    let reds: Vec<u8> = out.chunks(3).map(|px| px[0]).collect();
    assert_eq!(reds, vec![5, 25, 45, 65]);
}

#[test]
fn test_crop_and_resize_upscale_replicates_border_half_pixel() {
    // 4x1 ramp 0, 40, 80, 120 upscaled to 8x1: sources -0.25 (clamped to 0), 0.25, 0.75,
    // ..., 3.25 (clamped to 3) as in cv2 INTER_LINEAR.
    let rgb = ramp_row(4, 40);
    let out = crop_and_resize(&rgb, 4, 1, &BoundingBox::new(0.0, 0.0, 4.0, 1.0), 8, 1)
        .expect("crop succeeds");
    let reds: Vec<u8> = out.chunks(3).map(|px| px[0]).collect();
    assert_eq!(reds, vec![0, 10, 30, 50, 70, 90, 110, 120]);
}

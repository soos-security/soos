//! Bounding box expansion and RGB crop-and-resize utilities for PAD and vision pipeline.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Bilinear interpolation coordinates, scale math, and pixel buffer indexing"
)]

use soos_inference_ort::BoundingBox;

use crate::error::VisionError;

/// Expands a bounding box from its center by `scale` factor and clamps the result to `[0, img_w]` and `[0, img_h]`.
///
/// Designed for Presentation Attack Detection (PAD) context cropping (typically 2.7x expansion).
pub fn expand_bbox_for_pad(bbox: &BoundingBox, scale: f32, img_w: u32, img_h: u32) -> BoundingBox {
    if img_w == 0 || img_h == 0 {
        return BoundingBox::new(0.0, 0.0, 0.0, 0.0);
    }

    let effective_scale = if scale > 0.0 { scale } else { 1.0 };
    let orig_w = (bbox.x2 - bbox.x1).max(0.0);
    let orig_h = (bbox.y2 - bbox.y1).max(0.0);
    let cx = (bbox.x1 + bbox.x2) / 2.0;
    let cy = (bbox.y1 + bbox.y2) / 2.0;

    let new_w = orig_w * effective_scale;
    let new_h = orig_h * effective_scale;
    let half_w = new_w / 2.0;
    let half_h = new_h / 2.0;

    let max_x = img_w as f32;
    let max_y = img_h as f32;

    let x1 = (cx - half_w).clamp(0.0, max_x);
    let y1 = (cy - half_h).clamp(0.0, max_y);
    let x2 = (cx + half_w).clamp(0.0, max_x);
    let y2 = (cy + half_h).clamp(0.0, max_y);

    BoundingBox::new(x1, y1, x2, y2)
}

/// Crops an RGB24 buffer by bounding box and resizes to target dimensions using bilinear interpolation.
/// Out-of-bounds regions are padded with black (0).
pub fn crop_and_resize(
    rgb: &[u8],
    img_w: u32,
    img_h: u32,
    bbox: &BoundingBox,
    target_w: u32,
    target_h: u32,
) -> Result<Vec<u8>, VisionError> {
    if img_w == 0 || img_h == 0 {
        return Err(VisionError::InvalidDimensions {
            width: img_w,
            height: img_h,
        });
    }
    if target_w == 0 || target_h == 0 {
        return Err(VisionError::InvalidDimensions {
            width: target_w,
            height: target_h,
        });
    }

    let expected_len = (img_w as usize)
        .checked_mul(img_h as usize)
        .and_then(|px| px.checked_mul(3))
        .ok_or(VisionError::InvalidDimensions {
            width: img_w,
            height: img_h,
        })?;

    if rgb.len() != expected_len {
        return Err(VisionError::InvalidBufferSize {
            expected: expected_len,
            actual: rgb.len(),
        });
    }

    let out_len = (target_w as usize)
        .checked_mul(target_h as usize)
        .and_then(|px| px.checked_mul(3))
        .ok_or(VisionError::InvalidDimensions {
            width: target_w,
            height: target_h,
        })?;

    let mut output = Vec::with_capacity(out_len);

    let bw = bbox.x2 - bbox.x1;
    let bh = bbox.y2 - bbox.y1;

    if bw <= 0.0 || bh <= 0.0 {
        output.resize(out_len, 0);
        return Ok(output);
    }

    let max_x = (img_w - 1) as f32;
    let max_y = (img_h - 1) as f32;
    let w_usize = img_w as usize;

    let sx = bw / (target_w as f32);
    let sy = bh / (target_h as f32);

    for v in 0..target_h {
        let vf = v as f32;
        let ys = bbox.y1 + vf * sy;

        for u in 0..target_w {
            let uf = u as f32;
            let xs = bbox.x1 + uf * sx;

            if xs >= 0.0 && xs <= max_x && ys >= 0.0 && ys <= max_y {
                let x0 = xs.floor() as usize;
                let y0 = ys.floor() as usize;
                let x1 = (x0 + 1).min(img_w as usize - 1);
                let y1 = (y0 + 1).min(img_h as usize - 1);

                let dx = xs - (x0 as f32);
                let dy = ys - (y0 as f32);

                let w00 = (1.0 - dx) * (1.0 - dy);
                let w10 = dx * (1.0 - dy);
                let w01 = (1.0 - dx) * dy;
                let w11 = dx * dy;

                let idx00 = (y0 * w_usize + x0) * 3;
                let idx10 = (y0 * w_usize + x1) * 3;
                let idx01 = (y1 * w_usize + x0) * 3;
                let idx11 = (y1 * w_usize + x1) * 3;

                for c in 0..3 {
                    let p00 = rgb[idx00 + c] as f32;
                    let p10 = rgb[idx10 + c] as f32;
                    let p01 = rgb[idx01 + c] as f32;
                    let p11 = rgb[idx11 + c] as f32;

                    let val = (w00 * p00 + w10 * p10 + w01 * p01 + w11 * p11)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    output.push(val);
                }
            } else {
                output.push(0);
                output.push(0);
                output.push(0);
            }
        }
    }

    Ok(output)
}

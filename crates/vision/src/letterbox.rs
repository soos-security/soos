//! Letterbox padding and coordinate projection utilities for vision models (SCRFD).

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::indexing_slicing,
    reason = "Bilinear interpolation coordinates, scale math, and pixel buffer indexing"
)]

use soos_inference_ort::BoundingBox;

use crate::error::VisionError;

/// Scaling and offset parameters produced by letterbox padding.
///
/// Enables forward projection of image coordinates into letterbox canvas space
/// and inverse un-projection from letterbox coordinates back to original frame coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LetterboxParams {
    /// Uniform scaling factor applied to the input image.
    pub scale: f32,
    /// Horizontal offset (padding / 2) in target pixel coordinates.
    pub pad_x: f32,
    /// Vertical offset (padding / 2) in target pixel coordinates.
    pub pad_y: f32,
}

impl LetterboxParams {
    /// Creates a new `LetterboxParams` with given scale and padding offsets.
    pub fn new(scale: f32, pad_x: f32, pad_y: f32) -> Self {
        Self {
            scale,
            pad_x,
            pad_y,
        }
    }

    /// Unprojects coordinates from letterbox canvas space back to original image space.
    ///
    /// If `scale <= 0.0`, fails closed and returns `(0.0, 0.0)`.
    pub fn unproject(&self, x: f32, y: f32) -> (f32, f32) {
        if self.scale <= 0.0 {
            return (0.0, 0.0);
        }
        let orig_x = (x - self.pad_x) / self.scale;
        let orig_y = (y - self.pad_y) / self.scale;
        (orig_x, orig_y)
    }

    /// Unprojects a bounding box from letterbox canvas space back to original image space.
    pub fn unproject_bbox(&self, bbox: &BoundingBox) -> BoundingBox {
        let (x1, y1) = self.unproject(bbox.x1, bbox.y1);
        let (x2, y2) = self.unproject(bbox.x2, bbox.y2);
        BoundingBox::new(x1, y1, x2, y2)
    }

    /// Projects coordinates from original image space into letterbox canvas space.
    pub fn project(&self, x: f32, y: f32) -> (f32, f32) {
        let lx = x * self.scale + self.pad_x;
        let ly = y * self.scale + self.pad_y;
        (lx, ly)
    }
}

/// Computes the uniform scaling factor and symmetric padding offsets to fit an image
/// of dimensions `(img_w, img_h)` into a canvas of dimensions `(target_w, target_h)`
/// while preserving the aspect ratio.
pub fn letterbox_params(
    img_w: u32,
    img_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<LetterboxParams, VisionError> {
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

    let scale = (target_w as f32 / img_w as f32).min(target_h as f32 / img_h as f32);
    let scaled_w = ((img_w as f32 * scale).round() as u32).min(target_w);
    let scaled_h = ((img_h as f32 * scale).round() as u32).min(target_h);

    let pad_x = ((target_w as f32 - scaled_w as f32) / 2.0).max(0.0);
    let pad_y = ((target_h as f32 - scaled_h as f32) / 2.0).max(0.0);

    Ok(LetterboxParams::new(scale, pad_x, pad_y))
}

/// Resizes an RGB24 image buffer to target dimensions `(target_w, target_h)` preserving
/// the aspect ratio using letterbox padding and bilinear interpolation.
///
/// Returns the padded RGB24 byte vector alongside `LetterboxParams` for coordinate un-projection.
/// Any padded canvas border area is filled with black (RGB 0, 0, 0).
pub fn letterbox_resize(
    rgb: &[u8],
    img_w: u32,
    img_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<(Vec<u8>, LetterboxParams), VisionError> {
    let params = letterbox_params(img_w, img_h, target_w, target_h)?;

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

    let mut output = vec![0u8; out_len];

    let scaled_w = ((img_w as f32 * params.scale).round() as u32).min(target_w);
    let scaled_h = ((img_h as f32 * params.scale).round() as u32).min(target_h);

    let pad_x_int = params.pad_x.round() as u32;
    let pad_y_int = params.pad_y.round() as u32;

    let max_src_x = (img_w - 1) as f32;
    let max_src_y = (img_h - 1) as f32;
    let src_stride = (img_w as usize) * 3;
    let dst_stride = (target_w as usize) * 3;

    for dy in 0..scaled_h {
        let dst_y = dy + pad_y_int;
        if dst_y >= target_h {
            break;
        }

        let ys = (dy as f32 / params.scale).clamp(0.0, max_src_y);
        let y0 = ys.floor() as usize;
        let y1 = (y0 + 1).min(img_h as usize - 1);
        let dy_weight = ys - (y0 as f32);

        let row0_offset = y0 * src_stride;
        let row1_offset = y1 * src_stride;
        let dst_row_offset = (dst_y as usize) * dst_stride;

        for dx in 0..scaled_w {
            let dst_x = dx + pad_x_int;
            if dst_x >= target_w {
                break;
            }

            let xs = (dx as f32 / params.scale).clamp(0.0, max_src_x);
            let x0 = xs.floor() as usize;
            let x1 = (x0 + 1).min(img_w as usize - 1);
            let dx_weight = xs - (x0 as f32);

            let w00 = (1.0 - dx_weight) * (1.0 - dy_weight);
            let w10 = dx_weight * (1.0 - dy_weight);
            let w01 = (1.0 - dx_weight) * dy_weight;
            let w11 = dx_weight * dy_weight;

            let idx00 = row0_offset + x0 * 3;
            let idx10 = row0_offset + x1 * 3;
            let idx01 = row1_offset + x0 * 3;
            let idx11 = row1_offset + x1 * 3;

            let dst_pixel_idx = dst_row_offset + (dst_x as usize) * 3;

            for c in 0..3 {
                let p00 = rgb[idx00 + c] as f32;
                let p10 = rgb[idx10 + c] as f32;
                let p01 = rgb[idx01 + c] as f32;
                let p11 = rgb[idx11 + c] as f32;

                let val = (w00 * p00 + w10 * p10 + w01 * p01 + w11 * p11)
                    .round()
                    .clamp(0.0, 255.0) as u8;

                output[dst_pixel_idx + c] = val;
            }
        }
    }

    Ok((output, params))
}

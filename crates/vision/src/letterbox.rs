//! Letterbox padding and coordinate projection utilities for vision models (SCRFD).

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "Letterbox geometry conversions and bounded canvas offsets"
)]

use soos_inference_ort::{letterbox_bilinear, letterbox_geometry, BoundingBox, LetterboxGeometry};

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
///
/// Delegates to [`soos_inference_ort::letterbox_geometry`], the single implementation shared
/// with the production SCRFD input packing: the offsets are exact integers (GitHub #248).
pub fn letterbox_params(
    img_w: u32,
    img_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<LetterboxParams, VisionError> {
    geometry(img_w, img_h, target_w, target_h).map(|g| params_of(&g))
}

fn geometry(
    img_w: u32,
    img_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<LetterboxGeometry, VisionError> {
    if img_w == 0 || img_h == 0 {
        return Err(VisionError::InvalidDimensions {
            width: img_w,
            height: img_h,
        });
    }
    letterbox_geometry(img_w, img_h, target_w, target_h).ok_or(VisionError::InvalidDimensions {
        width: target_w,
        height: target_h,
    })
}

fn params_of(g: &LetterboxGeometry) -> LetterboxParams {
    LetterboxParams::new(g.scale, g.pad_x as f32, g.pad_y as f32)
}

/// Resizes an RGB24 image buffer to target dimensions `(target_w, target_h)` preserving
/// the aspect ratio using letterbox padding and bilinear interpolation.
///
/// Returns the padded RGB24 byte vector alongside `LetterboxParams` for coordinate un-projection.
/// Any padded canvas border area is filled with black (RGB 0, 0, 0). The pixels are produced by
/// [`soos_inference_ort::letterbox_bilinear`], the same resampler that packs the SCRFD tensor.
pub fn letterbox_resize(
    rgb: &[u8],
    img_w: u32,
    img_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<(Vec<u8>, LetterboxParams), VisionError> {
    let geometry = geometry(img_w, img_h, target_w, target_h)?;

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
    let dst_stride = (target_w as usize) * 3;
    let (tw, th) = (target_w as usize, target_h as usize);

    letterbox_bilinear(rgb, img_w, img_h, &geometry, |dst_x, dst_y, px| {
        if dst_x >= tw || dst_y >= th {
            return;
        }
        let idx = dst_y * dst_stride + dst_x * 3;
        if let Some(slot) = output.get_mut(idx..idx + 3) {
            slot.copy_from_slice(&px);
        }
    });

    Ok((output, params_of(&geometry)))
}

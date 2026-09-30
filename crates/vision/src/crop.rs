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
/// Continuous-coordinate context box (right/bottom edges at most `img_w` / `img_h`), used for
/// display overlays. The PAD model input is built by [`crop_pad_context`], whose window
/// ([`pad_crop_window`]) follows the upstream inclusive-bounds geometry exactly (GitHub #213).
pub fn expand_bbox_for_pad(bbox: &BoundingBox, scale: f32, img_w: u32, img_h: u32) -> BoundingBox {
    if img_w == 0 || img_h == 0 {
        return BoundingBox::new(0.0, 0.0, 0.0, 0.0);
    }

    let orig_w = (bbox.x2 - bbox.x1).max(0.0);
    let orig_h = (bbox.y2 - bbox.y1).max(0.0);
    if orig_w <= 0.0 || orig_h <= 0.0 {
        return BoundingBox::new(0.0, 0.0, 0.0, 0.0);
    }

    let max_x = img_w as f32;
    let max_y = img_h as f32;

    let requested_scale = if scale > 0.0 { scale } else { 1.0 };
    // Bound effective scale so expanded box does not exceed total image canvas
    let effective_scale = requested_scale.min(max_x / orig_w).min(max_y / orig_h);

    let new_w = orig_w * effective_scale;
    let new_h = orig_h * effective_scale;
    let cx = (bbox.x1 + bbox.x2) / 2.0;
    let cy = (bbox.y1 + bbox.y2) / 2.0;

    let mut x1 = cx - new_w / 2.0;
    let mut y1 = cy - new_h / 2.0;
    let mut x2 = cx + new_w / 2.0;
    let mut y2 = cy + new_h / 2.0;

    // Translation-preserving shifting matching Minivision CropImage::_get_new_box:
    // When expanding over an image boundary, shift the window inward rather than
    // clamping coordinates independently, maintaining context scale and 1:1 aspect ratio.
    if x1 < 0.0 {
        x2 -= x1;
        x1 = 0.0;
    }
    if y1 < 0.0 {
        y2 -= y1;
        y1 = 0.0;
    }

    if x2 > max_x {
        x1 -= x2 - max_x;
        x2 = max_x;
    }
    if y2 > max_y {
        y1 -= y2 - max_y;
        y2 = max_y;
    }

    let clamped_x1 = x1.clamp(0.0, max_x);
    let clamped_y1 = y1.clamp(0.0, max_y);
    let clamped_x2 = x2.clamp(0.0, max_x);
    let clamped_y2 = y2.clamp(0.0, max_y);

    BoundingBox::new(clamped_x1, clamped_y1, clamped_x2, clamped_y2)
}

/// Crops an RGB24 buffer by bounding box and resizes to target dimensions using bilinear interpolation.
///
/// Output pixel `(u, v)` samples `(x1 + (u + 0.5) * sx - 0.5, y1 + (v + 0.5) * sy - 0.5)`, the
/// half-pixel centre convention of `cv2.resize` `INTER_LINEAR` (GitHub #213). Samples within
/// half a pixel of the image replicate the border; regions further out are padded with black (0).
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
        // Half-pixel centre convention of cv2.resize INTER_LINEAR (GitHub #213).
        let ys = bbox.y1 + (v as f32 + 0.5) * sy - 0.5;

        for u in 0..target_w {
            let xs = bbox.x1 + (u as f32 + 0.5) * sx - 0.5;

            // A sample within half a pixel of the image replicates the border pixel;
            // anything further out is padded with black.
            if xs >= -0.5 && xs <= max_x + 0.5 && ys >= -0.5 && ys <= max_y + 0.5 {
                let px = bilinear_rgb(
                    rgb,
                    w_usize,
                    xs.clamp(0.0, max_x),
                    ys.clamp(0.0, max_y),
                    (0, 0),
                    (img_w as usize - 1, img_h as usize - 1),
                );
                output.extend_from_slice(&px);
            } else {
                output.extend_from_slice(&[0, 0, 0]);
            }
        }
    }

    Ok(output)
}

/// Samples one RGB pixel at `(xs, ys)` by bilinear interpolation.
///
/// `xs`/`ys` must already lie in `[lo, hi]` (absolute image coordinates); the second tap is
/// clamped to `hi`, which replicates the border exactly like `cv2.resize` does inside the
/// source window.
fn bilinear_rgb(
    rgb: &[u8],
    stride: usize,
    xs: f32,
    ys: f32,
    lo: (usize, usize),
    hi: (usize, usize),
) -> [u8; 3] {
    let x0 = (xs.floor() as usize).clamp(lo.0, hi.0);
    let y0 = (ys.floor() as usize).clamp(lo.1, hi.1);
    let x1 = (x0 + 1).min(hi.0);
    let y1 = (y0 + 1).min(hi.1);

    let dx = (xs - x0 as f32).clamp(0.0, 1.0);
    let dy = (ys - y0 as f32).clamp(0.0, 1.0);

    let w00 = (1.0 - dx) * (1.0 - dy);
    let w10 = dx * (1.0 - dy);
    let w01 = (1.0 - dx) * dy;
    let w11 = dx * dy;

    let idx00 = (y0 * stride + x0) * 3;
    let idx10 = (y0 * stride + x1) * 3;
    let idx01 = (y1 * stride + x0) * 3;
    let idx11 = (y1 * stride + x1) * 3;

    let mut px = [0u8; 3];
    for (c, slot) in px.iter_mut().enumerate() {
        let p00 = f32::from(rgb[idx00 + c]);
        let p10 = f32::from(rgb[idx10 + c]);
        let p01 = f32::from(rgb[idx01 + c]);
        let p11 = f32::from(rgb[idx11 + c]);
        *slot = (w00 * p00 + w10 * p10 + w01 * p01 + w11 * p11)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    px
}

/// Integer pixel window of a PAD context crop: columns `x..x + width`, rows `y..y + height`.
///
/// Always non-empty and inside the frame when returned by [`pad_crop_window`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadCropWindow {
    /// First column (inclusive).
    pub x: u32,
    /// First row (inclusive).
    pub y: u32,
    /// Number of columns (at least 1).
    pub width: u32,
    /// Number of rows (at least 1).
    pub height: u32,
}

/// Computes the PAD context window exactly like upstream Silent-Face-Anti-Spoofing
/// `CropImage._get_new_box` + `CropImage.crop` (GitHub #213).
///
/// With `(x, y, box_w, box_h)` taken from the detector box (`x1`, `y1`, `x2 - x1`, `y2 - y1`):
/// - the scale is capped by `(img_h - 1) / box_h` and `(img_w - 1) / box_w`;
/// - the scaled box is centred on the face, shifted inward against `0` and the **last pixel
///   index** `img_w - 1` / `img_h - 1` (inclusive bounds);
/// - corners are truncated like Python `int()` and the slice is inclusive (`right + 1`).
///
/// Returns `None` (fail-closed) for zero frame dimensions, non-finite or non-positive box
/// sizes, or a non-finite / non-positive scale.
pub fn pad_crop_window(
    bbox: &BoundingBox,
    scale: f32,
    img_w: u32,
    img_h: u32,
) -> Option<PadCropWindow> {
    if img_w == 0 || img_h == 0 || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let coords = [bbox.x1, bbox.y1, bbox.x2, bbox.y2];
    if coords.iter().any(|c| !c.is_finite()) {
        return None;
    }
    // f64 keeps the transcription as close as possible to the Python float arithmetic.
    let (x, y) = (f64::from(bbox.x1), f64::from(bbox.y1));
    let box_w = f64::from(bbox.x2) - x;
    let box_h = f64::from(bbox.y2) - y;
    if !(box_w > 0.0 && box_h > 0.0) {
        return None;
    }
    let src_w = f64::from(img_w);
    let src_h = f64::from(img_h);

    // Upstream parses the scale from the model name as a Python float ("2.7" -> the f64
    // nearest 2.7). Widening the f32 would give 2.7000000477 and move a corner across an
    // integer boundary, so the scale is snapped to 6 decimals first.
    let requested = (f64::from(scale) * 1e6).round() / 1e6;
    let scale = ((src_h - 1.0) / box_h)
        .min((src_w - 1.0) / box_w)
        .min(requested);

    let new_width = box_w * scale;
    let new_height = box_h * scale;
    let center_x = box_w / 2.0 + x;
    let center_y = box_h / 2.0 + y;

    let mut left_top_x = center_x - new_width / 2.0;
    let mut left_top_y = center_y - new_height / 2.0;
    let mut right_bottom_x = center_x + new_width / 2.0;
    let mut right_bottom_y = center_y + new_height / 2.0;

    if left_top_x < 0.0 {
        right_bottom_x -= left_top_x;
        left_top_x = 0.0;
    }
    if left_top_y < 0.0 {
        right_bottom_y -= left_top_y;
        left_top_y = 0.0;
    }
    if right_bottom_x > src_w - 1.0 {
        left_top_x -= right_bottom_x - src_w + 1.0;
        right_bottom_x = src_w - 1.0;
    }
    if right_bottom_y > src_h - 1.0 {
        left_top_y -= right_bottom_y - src_h + 1.0;
        right_bottom_y = src_h - 1.0;
    }

    // Python int() truncates toward zero; clamping keeps the window inside the frame even
    // for a box entirely outside it (numpy slicing would clip the same way).
    let last_x = i64::from(img_w) - 1;
    let last_y = i64::from(img_h) - 1;
    let left = (left_top_x.trunc() as i64).clamp(0, last_x);
    let top = (left_top_y.trunc() as i64).clamp(0, last_y);
    let right = (right_bottom_x.trunc() as i64).clamp(0, last_x);
    let bottom = (right_bottom_y.trunc() as i64).clamp(0, last_y);
    if right < left || bottom < top {
        return None;
    }

    Some(PadCropWindow {
        x: u32::try_from(left).ok()?,
        y: u32::try_from(top).ok()?,
        width: u32::try_from(right - left + 1).ok()?,
        height: u32::try_from(bottom - top + 1).ok()?,
    })
}

/// Produces the PAD model input crop with upstream-parity geometry (GitHub #213).
///
/// The window is [`pad_crop_window`]; it is resized to `target_w x target_h` with the
/// `cv2.resize` `INTER_LINEAR` convention: half-pixel centre mapping
/// `src = (dst + 0.5) * (window / target) - 0.5`, clamped to the window (replicated border),
/// bilinear weights, round to nearest. A degenerate box yields an all-black crop, which the
/// PAD model scores as a spoof (fail-closed), mirroring [`crop_and_resize`].
pub fn crop_pad_context(
    rgb: &[u8],
    img_w: u32,
    img_h: u32,
    bbox: &BoundingBox,
    scale: f32,
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

    let Some(window) = pad_crop_window(bbox, scale, img_w, img_h) else {
        return Ok(vec![0u8; out_len]);
    };

    let mut output = Vec::with_capacity(out_len);
    let lo = (window.x as usize, window.y as usize);
    let hi = (
        (window.x + window.width - 1) as usize,
        (window.y + window.height - 1) as usize,
    );
    let scale_x = f64::from(window.width) / f64::from(target_w);
    let scale_y = f64::from(window.height) / f64::from(target_h);
    let max_fx = f64::from(window.width - 1);
    let max_fy = f64::from(window.height - 1);

    for v in 0..target_h {
        let fy = ((f64::from(v) + 0.5) * scale_y - 0.5).clamp(0.0, max_fy);
        let ys = (f64::from(window.y) + fy) as f32;
        for u in 0..target_w {
            let fx = ((f64::from(u) + 0.5) * scale_x - 0.5).clamp(0.0, max_fx);
            let xs = (f64::from(window.x) + fx) as f32;
            output.extend_from_slice(&bilinear_rgb(rgb, img_w as usize, xs, ys, lo, hi));
        }
    }

    Ok(output)
}

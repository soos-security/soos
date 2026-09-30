//! Single letterbox implementation of the workspace (GitHub #248, review finding VIS-06).
//!
//! Both the production SCRFD input packing ([`crate::detector::letterbox_pad`]) and the
//! `soos-vision` RGB utilities (`letterbox_params`, `letterbox_resize`) are thin wrappers over
//! this module, so the tested code is the code the daemon runs.
//!
//! - The integer placement offsets are computed first, and the same integer values are
//!   returned for un-projection: an odd padding never introduces a half-pixel landmark offset.
//! - Resampling is bilinear (source coordinate `dst / scale`, clamped to the image), with the
//!   interpolated value rounded to `u8` before any normalization.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "Bounded letterbox geometry and bilinear interpolation arithmetic; every index is \
              clamped to the validated image or canvas dimensions"
)]

/// Letterbox geometry: uniform scale, scaled image size and integer placement offsets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LetterboxGeometry {
    /// Uniform scaling factor applied to the source image.
    pub scale: f32,
    /// Width of the scaled image inside the canvas, in pixels.
    pub scaled_w: u32,
    /// Height of the scaled image inside the canvas, in pixels.
    pub scaled_h: u32,
    /// Integer horizontal offset of the scaled image in the canvas.
    pub pad_x: u32,
    /// Integer vertical offset of the scaled image in the canvas.
    pub pad_y: u32,
}

/// Computes the letterbox geometry fitting `(img_w, img_h)` into `(target_w, target_h)`.
///
/// Returns `None` when any dimension is zero. The offsets are
/// `floor((target - scaled) / 2)`, exact integers used both to place and to un-project.
#[must_use]
pub fn letterbox_geometry(
    img_w: u32,
    img_h: u32,
    target_w: u32,
    target_h: u32,
) -> Option<LetterboxGeometry> {
    if img_w == 0 || img_h == 0 || target_w == 0 || target_h == 0 {
        return None;
    }
    let scale = (target_w as f32 / img_w as f32).min(target_h as f32 / img_h as f32);
    let scaled_w = ((img_w as f32 * scale).round() as u32).clamp(1, target_w);
    let scaled_h = ((img_h as f32 * scale).round() as u32).clamp(1, target_h);
    Some(LetterboxGeometry {
        scale,
        scaled_w,
        scaled_h,
        pad_x: (target_w - scaled_w) / 2,
        pad_y: (target_h - scaled_h) / 2,
    })
}

/// Bilinearly resamples an RGB24 image into the letterbox canvas described by `geometry`.
///
/// Calls `sink(dst_x, dst_y, [r, g, b])` once for every canvas pixel covered by the scaled
/// image (padding pixels are left to the caller). Returns `false` without calling `sink` when
/// the buffer length does not match `img_w * img_h * 3` or a dimension is zero.
pub fn letterbox_bilinear<F>(
    rgb: &[u8],
    img_w: u32,
    img_h: u32,
    geometry: &LetterboxGeometry,
    mut sink: F,
) -> bool
where
    F: FnMut(usize, usize, [u8; 3]),
{
    let expected = (img_w as usize)
        .checked_mul(img_h as usize)
        .and_then(|px| px.checked_mul(3));
    if img_w == 0 || img_h == 0 || geometry.scale <= 0.0 || expected != Some(rgb.len()) {
        return false;
    }

    let max_src_x = (img_w - 1) as f32;
    let max_src_y = (img_h - 1) as f32;
    let last_x = img_w as usize - 1;
    let last_y = img_h as usize - 1;
    let stride = img_w as usize * 3;
    let pixel = |idx: usize| -> [f32; 3] {
        match rgb.get(idx..idx + 3) {
            Some(&[r, g, b]) => [f32::from(r), f32::from(g), f32::from(b)],
            _ => [0.0; 3],
        }
    };

    for dy in 0..geometry.scaled_h as usize {
        let ys = (dy as f32 / geometry.scale).clamp(0.0, max_src_y);
        let y0 = (ys.floor() as usize).min(last_y);
        let y1 = (y0 + 1).min(last_y);
        let wy = ys - y0 as f32;

        for dx in 0..geometry.scaled_w as usize {
            let xs = (dx as f32 / geometry.scale).clamp(0.0, max_src_x);
            let x0 = (xs.floor() as usize).min(last_x);
            let x1 = (x0 + 1).min(last_x);
            let wx = xs - x0 as f32;

            let p00 = pixel(y0 * stride + x0 * 3);
            let p10 = pixel(y0 * stride + x1 * 3);
            let p01 = pixel(y1 * stride + x0 * 3);
            let p11 = pixel(y1 * stride + x1 * 3);

            let (w00, w10) = ((1.0 - wx) * (1.0 - wy), wx * (1.0 - wy));
            let (w01, w11) = ((1.0 - wx) * wy, wx * wy);
            let blend = |c: usize| -> u8 {
                let at = |p: &[f32; 3]| p.get(c).copied().unwrap_or(0.0);
                let v = w00 * at(&p00) + w10 * at(&p10) + w01 * at(&p01) + w11 * at(&p11);
                v.round().clamp(0.0, 255.0) as u8
            };
            let out = [blend(0), blend(1), blend(2)];
            sink(
                dx + geometry.pad_x as usize,
                dy + geometry.pad_y as usize,
                out,
            );
        }
    }
    true
}

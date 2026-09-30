//! Pre-PAD face quality gate (GitHub #218 / PAD-13).
//!
//! MiniFASNet scores are unreliable on tiny or blurred faces (a phone held at arm's
//! length, a print seen from far): the 2.7x context crop is upsampled into the 80x80
//! PAD input and the texture cues the model relies on are gone. The pipeline therefore
//! rejects, before the PAD model is consulted:
//! - a face whose smaller bounding-box side is below `min_face_width_px`
//!   ([`DEFAULT_MIN_FACE_WIDTH_PX`]);
//! - a PAD crop whose sharpness (variance of the 4-neighbour Laplacian of the luma,
//!   [`laplacian_variance`]) is below `min_pad_crop_sharpness`
//!   ([`DEFAULT_MIN_PAD_CROP_SHARPNESS`], disabled until calibrated on real hardware).
//!
//! Every comparison is written fail-closed: a non-finite size, sharpness or threshold
//! rejects the face.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::indexing_slicing,
    reason = "Bounded per-pixel arithmetic over a buffer whose length is validated first"
)]

/// Default minimum face size in pixels (smaller side of the detector bounding box).
///
/// Above the no-upsampling floor of the PAD crop (`80 / 2.7 ~= 29.6` px) with margin,
/// and below the smallest face produced at normal laptop distance on a 640x480 sensor.
pub const DEFAULT_MIN_FACE_WIDTH_PX: f32 = 48.0;

/// Default minimum PAD crop sharpness (variance of the Laplacian).
///
/// `0.0` disables the blur gate: no real-camera calibration exists yet, and an
/// uncalibrated positive floor would deny genuine users (see `AI/DECISIONS.md`,
/// "Pre-PAD Face Quality Gate").
pub const DEFAULT_MIN_PAD_CROP_SHARPNESS: f32 = 0.0;

/// Reason a face was rejected by the quality gate (reported by `analyze_frame`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceQualityRejection {
    /// Face bounding box is smaller than `min_face_width_px` (or not finite).
    TooSmall,
    /// PAD crop sharpness is below `min_pad_crop_sharpness` (or not finite).
    Blurred,
}

/// Returns `true` when `size` passes `min` (fail-closed on any non-finite value).
pub(crate) fn passes_min(value: f32, min: f32) -> bool {
    value.is_finite() && min.is_finite() && value >= min
}

/// Smaller side of a bounding box given by its corners (may be non-finite or negative).
pub(crate) fn face_size_px(x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let w = x2 - x1;
    let h = y2 - y1;
    if w.is_nan() || h.is_nan() {
        return f32::NAN;
    }
    w.min(h)
}

/// Variance of the 4-neighbour Laplacian of the luma of an RGB24 image.
///
/// Returns `None` when the buffer length does not match `width * height * 3` or when
/// the image is smaller than 3x3 (no interior pixel). A flat image scores `0.0`.
pub fn laplacian_variance(rgb: &[u8], width: u32, height: u32) -> Option<f32> {
    let w = usize::try_from(width).ok()?;
    let h = usize::try_from(height).ok()?;
    if w < 3 || h < 3 {
        return None;
    }
    let expected = w.checked_mul(h)?.checked_mul(3)?;
    if rgb.len() != expected {
        return None;
    }

    // Integer BT.601 luma keeps the Laplacian exact (a flat image scores exactly 0).
    let luma: Vec<i32> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| (77 * i32::from(p[0]) + 150 * i32::from(p[1]) + 29 * i32::from(p[2])) >> 8)
        .collect();

    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut count = 0u64;
    for y in 1..h - 1 {
        let row = y * w;
        for x in 1..w - 1 {
            let i = row + x;
            let lap = luma[i - 1] + luma[i + 1] + luma[i - w] + luma[i + w] - 4 * luma[i];
            let lap = f64::from(lap);
            sum += lap;
            sum_sq += lap * lap;
            count += 1;
        }
    }
    let n = count as f64;
    let mean = sum / n;
    let variance = (sum_sq / n - mean * mean).max(0.0);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Variance of 8-bit Laplacian responses is far below f32::MAX"
    )]
    let variance = variance as f32;
    Some(variance)
}

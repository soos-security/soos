//! Format-aware Presentation Attack Detection policy (review finding PAD-03, GitHub #169).
//!
//! MiniFASNetV2 is trained on colour captures. A `PixelFormat::Grey` frame (typically the
//! near-infrared sensor of a dual-sensor laptop, which `CameraConfig` prefers by default) is
//! replicated into three identical channels by `convert_to_rgb`, which is out-of-distribution
//! for the model: its scores on such input are uncalibrated. Until an IR-calibrated threshold
//! (measured on captured frames, GitHub #172 / PAD-06) or an IR-trained PAD model exists, a
//! monochrome frame is handled by a conservative, fail-closed short-term policy:
//!
//! 1. **IR gate** on the PAD context crop (exposure, contrast, local texture). Under active
//!    NIR illumination a live face is well exposed and textured; LCD/OLED screens emit almost
//!    no 850/940 nm light (a replay appears as a dark or flat rectangle), and a missing or
//!    blocked emitter produces an unlit crop. Any gate failure rejects the capture before the
//!    RGB-trained model is consulted.
//! 2. **Stricter liveness threshold**: the MiniFASNet score must reach
//!    `max(pad_threshold, ir_pad_threshold)`, never less than the RGB threshold.
//!
//! The gate constants below are **not calibrated measurements**: they are loose sanity
//! bounds that only reject clearly degenerate crops. They never grant liveness on their own.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::indexing_slicing,
    reason = "Bounded per-pixel statistics over a fixed-size PAD crop (80x80 by default)"
)]

use std::fmt;

use soos_camera_v4l::PixelFormat;

use crate::error::VisionError;

/// Default liveness threshold applied to monochrome (IR) frames.
///
/// Conservative, **uncalibrated** floor: it is deliberately stricter than the RGB default
/// `pad_threshold` (0.85) because MiniFASNetV2 scores on replicated-grey NIR input are
/// out-of-distribution. It is not derived from a measurement; replacing it with an
/// IR-calibrated value requires captured-frame calibration on hardware (GitHub #172).
pub const DEFAULT_IR_PAD_THRESHOLD: f32 = 0.95;

/// Minimum mean luma (0–255) of the PAD crop: below it the crop is treated as unlit
/// (IR emitter off or blocked, or a screen replay that emits no NIR light).
pub const IR_MIN_MEAN_LUMA: f32 = 20.0;

/// Maximum mean luma (0–255) of the PAD crop: above it the crop is saturated.
pub const IR_MAX_MEAN_LUMA: f32 = 235.0;

/// Minimum luma standard deviation of the PAD crop: below it the crop is flat.
pub const IR_MIN_LUMA_STDDEV: f32 = 10.0;

/// Minimum mean absolute neighbour difference (horizontal + vertical) of the PAD crop:
/// below it the crop has no local texture (flat surface, single edge, or smooth gradient).
pub const IR_MIN_TEXTURE_ENERGY: f32 = 2.0;

/// Colour modality of the pixels handed to the PAD stage, derived from the source format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadInputModality {
    /// Colour source (`Rgb24`, `Yuyv`, `Nv12`, `Mjpeg`): the RGB-trained PAD path applies.
    Color,
    /// Single-channel source (`Grey`, typically an IR sensor): the IR policy applies.
    Monochrome,
}

impl PadInputModality {
    /// Maps a camera pixel format to the PAD input modality.
    pub const fn from_pixel_format(format: PixelFormat) -> Self {
        match format {
            PixelFormat::Grey => Self::Monochrome,
            PixelFormat::Rgb24 | PixelFormat::Yuyv | PixelFormat::Nv12 | PixelFormat::Mjpeg => {
                Self::Color
            }
        }
    }

    /// True for single-channel (IR) sources.
    pub const fn is_monochrome(self) -> bool {
        matches!(self, Self::Monochrome)
    }
}

/// Reason why the IR gate rejected a monochrome PAD crop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrGateRejection {
    /// Mean luma below [`IR_MIN_MEAN_LUMA`].
    Underexposed,
    /// Mean luma above [`IR_MAX_MEAN_LUMA`].
    Overexposed,
    /// Luma standard deviation below [`IR_MIN_LUMA_STDDEV`].
    LowContrast,
    /// Texture energy below [`IR_MIN_TEXTURE_ENERGY`].
    LowTexture,
    /// The crop buffer or its dimensions are malformed.
    InvalidCrop,
}

impl fmt::Display for IrGateRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Underexposed => "crop is underexposed (no IR illumination)",
            Self::Overexposed => "crop is overexposed",
            Self::LowContrast => "crop has insufficient contrast",
            Self::LowTexture => "crop has insufficient texture",
            Self::InvalidCrop => "crop buffer is malformed",
        };
        f.write_str(text)
    }
}

/// Aggregate luma statistics of a PAD crop (no pixel data is retained).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrCropStatistics {
    /// Mean luma in `[0, 255]`.
    pub mean_luma: f32,
    /// Luma standard deviation.
    pub luma_stddev: f32,
    /// Mean absolute horizontal difference plus mean absolute vertical difference.
    pub texture_energy: f32,
}

/// Computes luma statistics over an RGB24 crop of `width * height` pixels.
///
/// Luma is the channel mean `(r + g + b) / 3`, which equals the grey value for a replicated
/// monochrome crop. Returns an error for zero dimensions, overflow, or a size mismatch.
pub fn ir_crop_statistics(
    crop_rgb: &[u8],
    width: u32,
    height: u32,
) -> Result<IrCropStatistics, VisionError> {
    if width == 0 || height == 0 {
        return Err(VisionError::InvalidDimensions { width, height });
    }
    let w = width as usize;
    let h = height as usize;
    let expected = w
        .checked_mul(h)
        .and_then(|px| px.checked_mul(3))
        .ok_or(VisionError::InvalidDimensions { width, height })?;
    if crop_rgb.len() != expected {
        return Err(VisionError::InvalidBufferSize {
            expected,
            actual: crop_rgb.len(),
        });
    }

    let luma: Vec<f64> = crop_rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|px| (f64::from(px[0]) + f64::from(px[1]) + f64::from(px[2])) / 3.0)
        .collect();

    let count = luma.len() as f64;
    let mean = luma.iter().sum::<f64>() / count;
    let variance = luma.iter().map(|&v| (v - mean) * (v - mean)).sum::<f64>() / count;

    let mut dx_sum = 0.0f64;
    let mut dx_count = 0usize;
    let mut dy_sum = 0.0f64;
    let mut dy_count = 0usize;
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let v = luma[row + x];
            if x + 1 < w {
                dx_sum += (luma[row + x + 1] - v).abs();
                dx_count += 1;
            }
            if y + 1 < h {
                dy_sum += (luma[row + w + x] - v).abs();
                dy_count += 1;
            }
        }
    }
    let mean_dx = if dx_count > 0 {
        dx_sum / dx_count as f64
    } else {
        0.0
    };
    let mean_dy = if dy_count > 0 {
        dy_sum / dy_count as f64
    } else {
        0.0
    };

    #[allow(
        clippy::cast_possible_truncation,
        reason = "Statistics are bounded by the 8-bit pixel range"
    )]
    Ok(IrCropStatistics {
        mean_luma: mean as f32,
        luma_stddev: variance.sqrt() as f32,
        texture_energy: (mean_dx + mean_dy) as f32,
    })
}

/// Fail-closed IR gate: `Ok(())` only for a well-exposed, contrasted and textured crop.
///
/// Checks are ordered exposure → contrast → texture; the first failure is reported.
/// A malformed crop is rejected ([`IrGateRejection::InvalidCrop`]), never accepted.
pub fn evaluate_ir_gate(crop_rgb: &[u8], width: u32, height: u32) -> Result<(), IrGateRejection> {
    let stats =
        ir_crop_statistics(crop_rgb, width, height).map_err(|_| IrGateRejection::InvalidCrop)?;

    // A non-finite statistic can never pass: it is rejected as a malformed crop.
    if !(stats.mean_luma.is_finite()
        && stats.luma_stddev.is_finite()
        && stats.texture_energy.is_finite())
    {
        return Err(IrGateRejection::InvalidCrop);
    }
    if stats.mean_luma < IR_MIN_MEAN_LUMA {
        return Err(IrGateRejection::Underexposed);
    }
    if stats.mean_luma > IR_MAX_MEAN_LUMA {
        return Err(IrGateRejection::Overexposed);
    }
    if stats.luma_stddev < IR_MIN_LUMA_STDDEV {
        return Err(IrGateRejection::LowContrast);
    }
    if stats.texture_energy < IR_MIN_TEXTURE_ENERGY {
        return Err(IrGateRejection::LowTexture);
    }
    Ok(())
}

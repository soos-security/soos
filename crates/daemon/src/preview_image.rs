//! Daemon-side preparation of preview frames (review finding CAM-14, GitHub #196).
//!
//! A raw camera frame can exceed `MAX_PREVIEW_MESSAGE_SIZE` (2 MiB): a 1920x1080 YUYV frame is
//! 4,147,200 bytes. Encoding it failed with `CodecError::MessageTooLarge`, which closed the
//! connection and put the GUI into a 100 ms reconnect loop. Frames are therefore reduced here
//! before encoding:
//!
//! - a frame at most [`MAX_PREVIEW_WIDTH`] pixels wide (or a compressed MJPEG frame of any
//!   width) whose payload fits [`MAX_PREVIEW_PIXEL_BYTES`] is forwarded unchanged;
//! - a larger frame is downscaled by integer decimation to at most [`MAX_PREVIEW_WIDTH`] pixels
//!   wide: greyscale stays greyscale (wire format 1), every colour format becomes RGB24
//!   (wire format 0);
//! - a frame that cannot be converted (truncated buffer, undecodable MJPEG, zero dimension)
//!   yields an explicit empty image ([`PREVIEW_FORMAT_EMPTY`]), never an error;
//! - a frame stamped [`SensorType::Infrared`] is always sent as greyscale (wire format 1), the
//!   luma of its pixels, whatever the pixel format its node streams (GitHub #305). The
//!   `PreviewResponse` carries no sensor field, so this is how the IR stamp reaches the GUI:
//!   a greyscale preview takes the Monochrome PAD path (IR gate and stricter IR threshold,
//!   GitHub #169) instead of the colour path.
//!
//! Only preview-authorized peers ever receive the result (see [`crate::preview`]); no pixel
//! data is logged, and every intermediate buffer is zeroized.

use soos_camera_v4l::{Frame, PixelFormat, SensorType};
use soos_protocol::types::MAX_PREVIEW_MESSAGE_SIZE;
use zeroize::{Zeroize, Zeroizing};

/// Maximum width, in pixels, of a downscaled preview frame.
pub const MAX_PREVIEW_WIDTH: u32 = 640;

/// Wire format code of an empty preview response (no frame available or not convertible).
pub const PREVIEW_FORMAT_EMPTY: u8 = 255;

/// Headroom reserved in [`MAX_PREVIEW_MESSAGE_SIZE`] for the `PreviewResponse` header fields
/// (version, sequence, dimensions, format, timestamp, length prefixes).
pub const PREVIEW_HEADER_RESERVE: usize = 1024;

/// Largest pixel payload placed in a `PreviewResponse`.
pub const MAX_PREVIEW_PIXEL_BYTES: usize = MAX_PREVIEW_MESSAGE_SIZE - PREVIEW_HEADER_RESERVE;

/// Pixel payload of a preview response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewImage {
    /// Wire format code (`0` RGB24, `1` Grey, `2` YUYV, `3` NV12, `4` MJPEG, `255` empty).
    pub format: u8,
    /// Width in pixels (`0` when empty).
    pub width: u32,
    /// Height in pixels (`0` when empty).
    pub height: u32,
    /// Pixel bytes (empty when [`PREVIEW_FORMAT_EMPTY`]).
    pub data: Vec<u8>,
}

impl PreviewImage {
    /// Returns the explicit empty image ([`PREVIEW_FORMAT_EMPTY`], no data).
    pub const fn empty() -> Self {
        Self {
            format: PREVIEW_FORMAT_EMPTY,
            width: 0,
            height: 0,
            data: Vec::new(),
        }
    }
}

impl Drop for PreviewImage {
    fn drop(&mut self) {
        self.data.zeroize();
    }
}

/// Returns the preview wire format code of a camera pixel format.
pub const fn wire_format_code(format: PixelFormat) -> u8 {
    match format {
        PixelFormat::Rgb24 => 0,
        PixelFormat::Grey => 1,
        PixelFormat::Yuyv => 2,
        PixelFormat::Nv12 => 3,
        PixelFormat::Mjpeg => 4,
    }
}

/// Builds the preview payload of `frame` (see the module documentation for the rules).
pub fn preview_image_for_frame(frame: &Frame) -> PreviewImage {
    if frame.sensor_type == SensorType::Infrared && frame.format != PixelFormat::Grey {
        // IR stamp (GitHub #305): convert to greyscale first, then apply the usual rules to
        // the greyscale frame. The intermediate frame is zeroized on drop (`Drop for Frame`).
        return match infrared_luma(frame) {
            Some(mut luma) => preview_image_for_frame(&Frame {
                data: std::mem::take(&mut *luma),
                width: frame.width,
                height: frame.height,
                timestamp_mono_ns: frame.timestamp_mono_ns,
                format: PixelFormat::Grey,
                sequence: frame.sequence,
                sensor_type: SensorType::Infrared,
            }),
            None => PreviewImage::empty(),
        };
    }
    // A compressed MJPEG frame that fits the budget is forwarded as-is whatever its width:
    // decoding it at the preview poll rate would cost far more than sending it.
    let narrow_enough = frame.width <= MAX_PREVIEW_WIDTH || frame.format == PixelFormat::Mjpeg;
    if narrow_enough && frame.data.len() <= MAX_PREVIEW_PIXEL_BYTES {
        return PreviewImage {
            format: wire_format_code(frame.format),
            width: frame.width,
            height: frame.height,
            data: frame.data.clone(),
        };
    }
    downscale(frame).unwrap_or_else(PreviewImage::empty)
}

/// Extracts the 8-bit luma plane (`width * height` bytes) of a colour-format frame: the `Y`
/// samples of YUYV and NV12, BT.601 luma of RGB24 and of decoded MJPEG. `None` when the
/// buffer is too short for the geometry or the MJPEG frame cannot be decoded.
fn infrared_luma(frame: &Frame) -> Option<Zeroizing<Vec<u8>>> {
    let width = usize::try_from(frame.width).ok()?;
    let height = usize::try_from(frame.height).ok()?;
    let pixels = width.checked_mul(height)?;
    if pixels == 0 {
        return None;
    }
    let data = frame.data.as_slice();
    let luma: Zeroizing<Vec<u8>> = Zeroizing::new(match frame.format {
        PixelFormat::Grey => data.get(..pixels)?.to_vec(),
        PixelFormat::Yuyv => data
            .get(..pixels.checked_mul(2)?)?
            .iter()
            .step_by(2)
            .copied()
            .collect(),
        PixelFormat::Nv12 => data.get(..pixels)?.to_vec(),
        PixelFormat::Rgb24 => rgb_luma(data.get(..pixels.checked_mul(3)?)?),
        PixelFormat::Mjpeg => {
            let rgb = Zeroizing::new(
                soos_vision::convert_to_rgb(&frame.data, frame.width, frame.height, frame.format)
                    .ok()?,
            );
            rgb_luma(rgb.get(..pixels.checked_mul(3)?)?)
        }
    });
    (luma.len() == pixels).then_some(luma)
}

/// BT.601 luma of packed RGB24 pixels (`(77 R + 150 G + 29 B + 128) >> 8`).
#[allow(
    clippy::arithmetic_side_effects,
    reason = "u8 inputs widened to u32 and multiplied by constants summing to 256: the \
              largest intermediate value is 65,408"
)]
fn rgb_luma(rgb: &[u8]) -> Vec<u8> {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .map(|&[r, g, b]| {
            let y = (77 * u32::from(r) + 150 * u32::from(g) + 29 * u32::from(b) + 128) >> 8;
            u8::try_from(y).unwrap_or(u8::MAX)
        })
        .collect()
}

/// Source layout validated against the buffer length before any sampling.
#[derive(Clone, Copy)]
enum Source<'a> {
    Grey(&'a [u8]),
    Rgb(&'a [u8]),
    Yuyv(&'a [u8]),
    Nv12(&'a [u8]),
}

fn downscale(frame: &Frame) -> Option<PreviewImage> {
    let width = usize::try_from(frame.width).ok()?;
    let height = usize::try_from(frame.height).ok()?;
    let pixels = width.checked_mul(height)?;
    if pixels == 0 {
        return None;
    }

    // MJPEG is decoded first (bounded by soos-vision's MJPEG limits); the decoded RGB24 buffer
    // is zeroized when this function returns.
    let decoded: Option<Zeroizing<Vec<u8>>> = if frame.format == PixelFormat::Mjpeg {
        let rgb = soos_vision::convert_to_rgb(&frame.data, frame.width, frame.height, frame.format)
            .ok()?;
        Some(Zeroizing::new(rgb))
    } else {
        None
    };

    let data = frame.data.as_slice();
    let source = match frame.format {
        PixelFormat::Grey if data.len() >= pixels => Source::Grey(data),
        PixelFormat::Rgb24 if data.len() >= pixels.checked_mul(3)? => Source::Rgb(data),
        PixelFormat::Yuyv if data.len() >= pixels.checked_mul(2)? => Source::Yuyv(data),
        PixelFormat::Nv12
            if width.is_multiple_of(2)
                && height.is_multiple_of(2)
                && data.len() >= pixels.checked_add(pixels / 2)? =>
        {
            Source::Nv12(data)
        }
        PixelFormat::Mjpeg => {
            let rgb = decoded.as_deref()?;
            if rgb.len() < pixels.checked_mul(3)? {
                return None;
            }
            Source::Rgb(rgb.as_slice())
        }
        _ => return None,
    };

    let (format, bytes_per_pixel) = match source {
        Source::Grey(_) => (wire_format_code(PixelFormat::Grey), 1usize),
        _ => (wire_format_code(PixelFormat::Rgb24), 3usize),
    };

    // Integer decimation step: at most MAX_PREVIEW_WIDTH wide and within the byte budget.
    // `pixels` is bounded by the validated buffer length, so this loop is short.
    let mut step = frame.width.div_ceil(MAX_PREVIEW_WIDTH).max(1);
    let (out_width, out_height) = loop {
        let out_width = frame.width.checked_div(step)?;
        let out_height = frame.height.checked_div(step)?;
        if out_width == 0 || out_height == 0 {
            return None;
        }
        let bytes = usize::try_from(out_width)
            .ok()?
            .checked_mul(usize::try_from(out_height).ok()?)?
            .checked_mul(bytes_per_pixel)?;
        if bytes <= MAX_PREVIEW_PIXEL_BYTES {
            break (out_width, out_height);
        }
        step = step.checked_add(1)?;
    };

    let step = usize::try_from(step).ok()?;
    let out_w = usize::try_from(out_width).ok()?;
    let out_h = usize::try_from(out_height).ok()?;
    let mut out = Zeroizing::new(Vec::with_capacity(
        out_w.checked_mul(out_h)?.checked_mul(bytes_per_pixel)?,
    ));
    for oy in 0..out_h {
        let sy = oy.checked_mul(step)?;
        for ox in 0..out_w {
            let sx = ox.checked_mul(step)?;
            match source {
                Source::Grey(buf) => out.push(*buf.get(sy.checked_mul(width)?.checked_add(sx)?)?),
                _ => out.extend_from_slice(&sample_rgb(source, width, height, sx, sy)?),
            }
        }
    }

    Some(PreviewImage {
        format,
        width: out_width,
        height: out_height,
        data: std::mem::take(&mut *out),
    })
}

/// Samples the RGB value of source pixel (`x`, `y`) of a validated colour source.
fn sample_rgb(
    source: Source<'_>,
    width: usize,
    height: usize,
    x: usize,
    y: usize,
) -> Option<[u8; 3]> {
    let index = y.checked_mul(width)?.checked_add(x)?;
    match source {
        Source::Rgb(buf) => {
            let start = index.checked_mul(3)?;
            let px = buf.get(start..start.checked_add(3)?)?;
            Some([*px.first()?, *px.get(1)?, *px.get(2)?])
        }
        Source::Yuyv(buf) => {
            let luma = *buf.get(index.checked_mul(2)?)?;
            // Each 4-byte macropixel [Y0, U, Y1, V] covers two horizontal pixels.
            let pair = y.checked_mul(width)?.checked_add(x & !1)?.checked_mul(2)?;
            let u = *buf.get(pair.checked_add(1)?)?;
            let v = *buf.get(pair.checked_add(3)?)?;
            Some(yuv_to_rgb(luma, u, v))
        }
        Source::Nv12(buf) => {
            let luma = *buf.get(index)?;
            let plane = width.checked_mul(height)?;
            let uv = plane
                .checked_add((y / 2).checked_mul(width)?)?
                .checked_add(x & !1)?;
            let u = *buf.get(uv)?;
            let v = *buf.get(uv.checked_add(1)?)?;
            Some(yuv_to_rgb(luma, u, v))
        }
        Source::Grey(buf) => {
            let g = *buf.get(index)?;
            Some([g, g, g])
        }
    }
}

/// Full-range fixed-point YUV to RGB conversion (same coefficients as `soos_vision`).
#[allow(
    clippy::arithmetic_side_effects,
    reason = "Inputs are u8 widened to i32 and multiplied by constants below 2048: every \
              intermediate value stays far inside the i32 range"
)]
fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let y = i32::from(y);
    let u = i32::from(u) - 128;
    let v = i32::from(v) - 128;
    let r = y + (1436 * v + 512) / 1024;
    let g = y - (352 * u + 731 * v - 512) / 1024;
    let b = y + (1815 * u + 512) / 1024;
    [clamp_u8(r), clamp_u8(g), clamp_u8(b)]
}

fn clamp_u8(value: i32) -> u8 {
    u8::try_from(value.clamp(0, 255)).unwrap_or(u8::MAX)
}

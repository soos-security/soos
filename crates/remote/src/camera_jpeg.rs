//! Pure JPEG path of the live camera view (ADR 2026-10-07, architect spec §6).
//!
//! Validates one preview frame received from the daemon (Grey, YUYV or RGB24 only), converts
//! it (with an optional 2x2 box downscale) into one exactly sized, zeroized buffer and
//! encodes it as a baseline JPEG into a fixed-capacity, zeroized sink that never grows.
//! No I/O, no clock, no logging. NV12, MJPEG and every other format are refused: this crate
//! never parses compressed images.
//!
//! Residual (amended ADR item 7): `jpeg-encoder` keeps internal working buffers (blocks,
//! coefficients) that it frees without zeroization; every buffer owned here is `Zeroizing`.

use std::io::{self, Write};

use soos_protocol::types::{PREVIEW_FORMAT_GREY, PREVIEW_FORMAT_RGB24, PREVIEW_FORMAT_YUYV};
use zeroize::Zeroizing;

use crate::config::CameraWidth;
use crate::{
    CAMERA_HALF_WIDTH, CAMERA_MAX_SCRATCH_BYTES, CAMERA_MAX_SOURCE_HEIGHT, CAMERA_MAX_SOURCE_WIDTH,
    CAMERA_MIN_SOURCE_DIM, MAX_CAMERA_JPEG_BYTES,
};

/// A finished JPEG image. No `Debug`, no `Clone`; zeroized on drop.
pub struct JpegFrame {
    /// The JPEG bytes (`FF D8` … `FF D9`).
    pub bytes: Zeroizing<Vec<u8>>,
}

/// Why a frame could not be turned into a JPEG. Fixed texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// A format other than Grey, YUYV or RGB24.
    #[error("unsupported preview format")]
    UnsupportedFormat,
    /// Dimensions or data length out of bounds.
    #[error("preview geometry out of bounds")]
    Geometry,
    /// The JPEG does not fit its sink.
    #[error("jpeg larger than the bound")]
    TooLarge,
    /// Any other encoder failure.
    #[error("jpeg encoding failed")]
    Encode,
}

/// Encoder settings resolved from the camera configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegSettings {
    /// Output width mode.
    pub width: CameraWidth,
    /// JPEG quality (validated by the configuration).
    pub quality: u8,
}

/// Pixel layout of a converted buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelKind {
    /// 1 byte per pixel.
    Luma,
    /// 3 bytes per pixel, R G B.
    Rgb,
    /// 3 bytes per pixel, Y Cb Cr (full-range BT.601, as UVC YUYV).
    Ycbcr,
}

/// `std::io::Write` sink over a `Zeroizing<Vec<u8>>` preallocated to its final capacity;
/// a write beyond the capacity fails with `WriteZero` and never reallocates.
pub struct JpegSink {
    buf: Zeroizing<Vec<u8>>,
    limit: usize,
    overflowed: bool,
}

impl JpegSink {
    /// A sink of `MAX_CAMERA_JPEG_BYTES`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limit(MAX_CAMERA_JPEG_BYTES)
    }

    /// Test hook: a sink of `capacity` bytes.
    #[doc(hidden)]
    #[must_use]
    pub fn with_capacity_for_tests(capacity: usize) -> Self {
        Self::with_limit(capacity)
    }

    fn with_limit(limit: usize) -> Self {
        Self {
            buf: Zeroizing::new(Vec::with_capacity(limit)),
            limit,
            overflowed: false,
        }
    }

    /// The fixed capacity of the sink.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.limit
    }

    /// The bytes written so far, as a frame.
    #[must_use]
    pub fn into_frame(self) -> JpegFrame {
        JpegFrame { bytes: self.buf }
    }
}

impl Default for JpegSink {
    fn default() -> Self {
        Self::new()
    }
}

impl Write for JpegSink {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        let room = self.limit.saturating_sub(self.buf.len());
        if room == 0 {
            self.overflowed = true;
            return Err(io::Error::new(io::ErrorKind::WriteZero, "jpeg sink full"));
        }
        let take = room.min(data.len());
        let chunk = data.get(..take).unwrap_or_default();
        // Never grows: `take` fits the preallocated capacity.
        self.buf.extend_from_slice(chunk);
        if take < data.len() {
            self.overflowed = true;
        }
        Ok(take)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Bytes per source pixel of a supported format.
fn source_bpp(format: u8) -> Option<usize> {
    match format {
        PREVIEW_FORMAT_GREY => Some(1),
        PREVIEW_FORMAT_YUYV => Some(2),
        PREVIEW_FORMAT_RGB24 => Some(3),
        _ => None,
    }
}

/// Validated geometry: (width, height) as `usize`.
fn validate(
    format: u8,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<(usize, usize), FrameError> {
    let bpp = source_bpp(format).ok_or(FrameError::UnsupportedFormat)?;
    if !(CAMERA_MIN_SOURCE_DIM..=CAMERA_MAX_SOURCE_WIDTH).contains(&width)
        || !(CAMERA_MIN_SOURCE_DIM..=CAMERA_MAX_SOURCE_HEIGHT).contains(&height)
    {
        return Err(FrameError::Geometry);
    }
    if format == PREVIEW_FORMAT_YUYV && !width.is_multiple_of(2) {
        return Err(FrameError::Geometry);
    }
    let w = usize::try_from(width).map_err(|_| FrameError::Geometry)?;
    let h = usize::try_from(height).map_err(|_| FrameError::Geometry)?;
    let expected = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(bpp))
        .ok_or(FrameError::Geometry)?;
    if data.len() != expected {
        return Err(FrameError::Geometry);
    }
    Ok((w, h))
}

/// Round-half-up average of four bytes.
fn avg4(a: u8, b: u8, c: u8, d: u8) -> u8 {
    let sum = u16::from(a)
        .saturating_add(u16::from(b))
        .saturating_add(u16::from(c))
        .saturating_add(u16::from(d))
        .saturating_add(2);
    u8::try_from(sum / 4).unwrap_or(u8::MAX)
}

/// Round-half-up average of two bytes.
fn avg2(a: u8, b: u8) -> u8 {
    let sum = u16::from(a).saturating_add(u16::from(b)).saturating_add(1);
    u8::try_from(sum / 2).unwrap_or(u8::MAX)
}

/// Byte at `index`, `Geometry` when out of range (unreachable after validation).
fn at(data: &[u8], index: Option<usize>) -> Result<u8, FrameError> {
    index
        .and_then(|i| data.get(i).copied())
        .ok_or(FrameError::Geometry)
}

/// Output buffer of exactly `len` bytes (≤ `CAMERA_MAX_SCRATCH_BYTES`), allocated once.
fn output(len: usize) -> Result<Zeroizing<Vec<u8>>, FrameError> {
    if len > CAMERA_MAX_SCRATCH_BYTES {
        return Err(FrameError::Geometry);
    }
    Ok(Zeroizing::new(Vec::with_capacity(len)))
}

/// 2x2 box downscale of a packed frame of `bpp` bytes per pixel.
fn half_packed(
    data: &[u8],
    w: usize,
    h: usize,
    bpp: usize,
) -> Result<Zeroizing<Vec<u8>>, FrameError> {
    let (ow, oh) = (w / 2, h / 2);
    let len = ow
        .checked_mul(oh)
        .and_then(|n| n.checked_mul(bpp))
        .ok_or(FrameError::Geometry)?;
    let mut out = output(len)?;
    let row = w.checked_mul(bpp).ok_or(FrameError::Geometry)?;
    for oy in 0..oh {
        let top = oy.checked_mul(2).and_then(|y| y.checked_mul(row));
        let bottom = top.and_then(|t| t.checked_add(row));
        for ox in 0..ow {
            let col = ox.checked_mul(2).and_then(|x| x.checked_mul(bpp));
            for c in 0..bpp {
                let left = col.and_then(|x| x.checked_add(c));
                let right = left.and_then(|x| x.checked_add(bpp));
                let px = |r: Option<usize>, x: Option<usize>| {
                    at(data, r.zip(x).and_then(|(r, x)| r.checked_add(x)))
                };
                out.push(avg4(
                    px(top, left)?,
                    px(top, right)?,
                    px(bottom, left)?,
                    px(bottom, right)?,
                ));
            }
        }
    }
    Ok(out)
}

/// YUYV → YCbCr 4:4:4 at full size: `Y0 U Y1 V` → `(Y0,U,V),(Y1,U,V)`.
fn yuyv_full(data: &[u8], w: usize, h: usize) -> Result<Zeroizing<Vec<u8>>, FrameError> {
    let len = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(3))
        .ok_or(FrameError::Geometry)?;
    let mut out = output(len)?;
    for &[y0, u, y1, v] in data.as_chunks::<4>().0 {
        out.extend_from_slice(&[y0, u, v, y1, u, v]);
    }
    if out.len() != len {
        return Err(FrameError::Geometry);
    }
    Ok(out)
}

/// YUYV half size: one output pixel per pixel pair of two rows (luma box average, chroma
/// average of the two rows).
fn yuyv_half(data: &[u8], w: usize, h: usize) -> Result<Zeroizing<Vec<u8>>, FrameError> {
    let (ow, oh) = (w / 2, h / 2);
    let len = ow
        .checked_mul(oh)
        .and_then(|n| n.checked_mul(3))
        .ok_or(FrameError::Geometry)?;
    let mut out = output(len)?;
    let row = w.checked_mul(2).ok_or(FrameError::Geometry)?;
    for oy in 0..oh {
        let top = oy.checked_mul(2).and_then(|y| y.checked_mul(row));
        let bottom = top.and_then(|t| t.checked_add(row));
        for ox in 0..ow {
            let col = ox.checked_mul(4);
            let base0 = top.zip(col).and_then(|(r, c)| r.checked_add(c));
            let base1 = bottom.zip(col).and_then(|(r, c)| r.checked_add(c));
            let byte =
                |base: Option<usize>, off: usize| at(data, base.and_then(|b| b.checked_add(off)));
            let luma = avg4(
                byte(base0, 0)?,
                byte(base0, 2)?,
                byte(base1, 0)?,
                byte(base1, 2)?,
            );
            let u = avg2(byte(base0, 1)?, byte(base1, 1)?);
            let v = avg2(byte(base0, 3)?, byte(base1, 3)?);
            out.extend_from_slice(&[luma, u, v]);
        }
    }
    Ok(out)
}

/// Copies `data` into one exactly sized zeroized buffer.
fn copy(data: &[u8]) -> Result<Zeroizing<Vec<u8>>, FrameError> {
    let mut out = output(data.len())?;
    out.extend_from_slice(data);
    Ok(out)
}

/// Converted pixels: the zeroized buffer, the output width and height, and the pixel kind.
pub type Converted = (Zeroizing<Vec<u8>>, u16, u16, PixelKind);

/// Pure conversion step: validates the frame, then converts it (with a 2x2 box downscale for
/// `Half` when the source is wider than `CAMERA_HALF_WIDTH`). Returns the pixels, the output
/// width and height and the pixel kind.
///
/// # Errors
///
/// [`FrameError::UnsupportedFormat`] for any format other than Grey, YUYV or RGB24;
/// [`FrameError::Geometry`] for dimensions or a data length out of bounds.
pub fn convert_frame(
    format: u8,
    width: u32,
    height: u32,
    data: &[u8],
    width_mode: CameraWidth,
) -> Result<Converted, FrameError> {
    let (w, h) = validate(format, width, height, data)?;
    let half = width_mode == CameraWidth::Half && width > CAMERA_HALF_WIDTH;
    let (ow, oh) = if half { (w / 2, h / 2) } else { (w, h) };
    let out_w = u16::try_from(ow).map_err(|_| FrameError::Geometry)?;
    let out_h = u16::try_from(oh).map_err(|_| FrameError::Geometry)?;
    let (pixels, kind) = match (format, half) {
        (PREVIEW_FORMAT_GREY, false) => (copy(data)?, PixelKind::Luma),
        (PREVIEW_FORMAT_GREY, true) => (half_packed(data, w, h, 1)?, PixelKind::Luma),
        (PREVIEW_FORMAT_RGB24, false) => (copy(data)?, PixelKind::Rgb),
        (PREVIEW_FORMAT_RGB24, true) => (half_packed(data, w, h, 3)?, PixelKind::Rgb),
        (PREVIEW_FORMAT_YUYV, false) => (yuyv_full(data, w, h)?, PixelKind::Ycbcr),
        (PREVIEW_FORMAT_YUYV, true) => (yuyv_half(data, w, h)?, PixelKind::Ycbcr),
        _ => return Err(FrameError::UnsupportedFormat),
    };
    if out_w == 0 || out_h == 0 {
        return Err(FrameError::Geometry);
    }
    Ok((pixels, out_w, out_h, kind))
}

/// Pure: validates, converts and encodes one frame into a `MAX_CAMERA_JPEG_BYTES` sink.
///
/// # Errors
///
/// Every [`FrameError`] variant.
pub fn encode_preview_jpeg(
    format: u8,
    width: u32,
    height: u32,
    data: &[u8],
    settings: JpegSettings,
) -> Result<JpegFrame, FrameError> {
    encode_preview_jpeg_into(format, width, height, data, settings, JpegSink::new())
}

/// Test hook of [`encode_preview_jpeg`] over an explicit sink.
///
/// # Errors
///
/// Every [`FrameError`] variant; an encoder error while the sink overflowed is
/// [`FrameError::TooLarge`].
#[doc(hidden)]
pub fn encode_preview_jpeg_into(
    format: u8,
    width: u32,
    height: u32,
    data: &[u8],
    settings: JpegSettings,
    mut sink: JpegSink,
) -> Result<JpegFrame, FrameError> {
    let (pixels, w, h, kind) = convert_frame(format, width, height, data, settings.width)?;
    let color = match kind {
        PixelKind::Luma => jpeg_encoder::ColorType::Luma,
        PixelKind::Rgb => jpeg_encoder::ColorType::Rgb,
        PixelKind::Ycbcr => jpeg_encoder::ColorType::Ycbcr,
    };
    let result = {
        let mut encoder = jpeg_encoder::Encoder::new(&mut sink, settings.quality);
        encoder.set_progressive(false);
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
        encoder.encode(&pixels, w, h, color)
    };
    match result {
        Ok(()) if !sink.overflowed => Ok(sink.into_frame()),
        Ok(()) => Err(FrameError::TooLarge),
        Err(_) if sink.overflowed => Err(FrameError::TooLarge),
        Err(_) => Err(FrameError::Encode),
    }
}

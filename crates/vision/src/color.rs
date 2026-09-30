//! Color conversion from camera pixel formats to standard RGB24.

#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    clippy::chunks_exact_to_as_chunks,
    reason = "High-performance pixel format conversions, fixed-point integer arithmetic, and buffer indexing"
)]

use soos_camera_v4l::PixelFormat;

use crate::error::VisionError;

/// Maximum accepted size of one compressed MJPEG frame (16 MiB).
///
/// A UVC MJPEG frame is far smaller than its raw RGB24 equivalent; anything larger is a
/// malformed or hostile buffer and is rejected before the decoder sees it.
pub const MAX_MJPEG_COMPRESSED_BYTES: usize = 16 * 1024 * 1024;

/// Maximum accepted MJPEG frame width or height, in pixels (covers 4K UHD 3840x2160).
///
/// Bounds the decoder working set (at most ~48 MiB of RGB24 output) independently of the
/// frame header.
pub const MAX_MJPEG_DIMENSION: u32 = 4096;

/// Converts raw frame buffer from a supported camera `PixelFormat` into an RGB24 buffer.
///
/// Output buffer length will be exactly `width * height * 3` bytes. For
/// [`PixelFormat::Mjpeg`] the frame header must declare exactly `width` x `height`
/// (see [`MAX_MJPEG_COMPRESSED_BYTES`] and [`MAX_MJPEG_DIMENSION`] for the input bounds).
pub fn convert_to_rgb(
    raw_buffer: &[u8],
    width: u32,
    height: u32,
    format: PixelFormat,
) -> Result<Vec<u8>, VisionError> {
    if width == 0 || height == 0 {
        return Err(VisionError::InvalidDimensions { width, height });
    }

    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(VisionError::InvalidDimensions { width, height })?;

    let expected_rgb_len = pixel_count
        .checked_mul(3)
        .ok_or(VisionError::InvalidDimensions { width, height })?;

    match format {
        PixelFormat::Rgb24 => {
            if raw_buffer.len() != expected_rgb_len {
                return Err(VisionError::InvalidBufferSize {
                    expected: expected_rgb_len,
                    actual: raw_buffer.len(),
                });
            }
            Ok(raw_buffer.to_vec())
        }
        PixelFormat::Grey => {
            if raw_buffer.len() != pixel_count {
                return Err(VisionError::InvalidBufferSize {
                    expected: pixel_count,
                    actual: raw_buffer.len(),
                });
            }

            let mut rgb = Vec::with_capacity(expected_rgb_len);
            for &grey in raw_buffer {
                rgb.push(grey);
                rgb.push(grey);
                rgb.push(grey);
            }
            Ok(rgb)
        }
        PixelFormat::Yuyv => {
            let expected_yuyv_len = pixel_count
                .checked_mul(2)
                .ok_or(VisionError::InvalidDimensions { width, height })?;

            if raw_buffer.len() != expected_yuyv_len {
                return Err(VisionError::InvalidBufferSize {
                    expected: expected_yuyv_len,
                    actual: raw_buffer.len(),
                });
            }

            let mut rgb = Vec::with_capacity(expected_rgb_len);

            // Each 4-byte chunk contains 2 pixels: [Y0, U, Y1, V]
            for chunk in raw_buffer.chunks_exact(4) {
                let y0 = chunk[0] as i32;
                let u = chunk[1] as i32 - 128;
                let y1 = chunk[2] as i32;
                let v = chunk[3] as i32 - 128;

                // Full-range fixed-point YUV to RGB conversion
                // R = Y + 1.402 * V
                // G = Y - 0.344136 * U - 0.714136 * V
                // B = Y + 1.772 * U
                let r0 = (y0 + (1436 * v + 512) / 1024).clamp(0, 255) as u8;
                let g0 = (y0 - (352 * u + 731 * v - 512) / 1024).clamp(0, 255) as u8;
                let b0 = (y0 + (1815 * u + 512) / 1024).clamp(0, 255) as u8;

                let r1 = (y1 + (1436 * v + 512) / 1024).clamp(0, 255) as u8;
                let g1 = (y1 - (352 * u + 731 * v - 512) / 1024).clamp(0, 255) as u8;
                let b1 = (y1 + (1815 * u + 512) / 1024).clamp(0, 255) as u8;

                rgb.push(r0);
                rgb.push(g0);
                rgb.push(b0);
                rgb.push(r1);
                rgb.push(g1);
                rgb.push(b1);
            }

            Ok(rgb)
        }
        PixelFormat::Nv12 => {
            if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
                return Err(VisionError::InvalidDimensions { width, height });
            }

            let uv_len = pixel_count
                .checked_div(2)
                .ok_or(VisionError::InvalidDimensions { width, height })?;
            let expected_nv12_len = pixel_count
                .checked_add(uv_len)
                .ok_or(VisionError::InvalidDimensions { width, height })?;

            if raw_buffer.len() != expected_nv12_len {
                return Err(VisionError::InvalidBufferSize {
                    expected: expected_nv12_len,
                    actual: raw_buffer.len(),
                });
            }

            let y_plane = &raw_buffer[..pixel_count];
            let uv_plane = &raw_buffer[pixel_count..];

            let mut rgb = Vec::with_capacity(expected_rgb_len);
            let w = width as usize;
            let h = height as usize;

            for y in 0..h {
                let y_row_offset = y * w;
                let uv_row_offset = (y / 2) * w;
                for x in 0..w {
                    let y_val = y_plane[y_row_offset + x] as i32;
                    let uv_idx = uv_row_offset + (x / 2) * 2;
                    let u_val = uv_plane[uv_idx] as i32 - 128;
                    let v_val = uv_plane[uv_idx + 1] as i32 - 128;

                    let r = (y_val + (1436 * v_val + 512) / 1024).clamp(0, 255) as u8;
                    let g = (y_val - (352 * u_val + 731 * v_val - 512) / 1024).clamp(0, 255) as u8;
                    let b = (y_val + (1815 * u_val + 512) / 1024).clamp(0, 255) as u8;

                    rgb.push(r);
                    rgb.push(g);
                    rgb.push(b);
                }
            }

            Ok(rgb)
        }
        PixelFormat::Mjpeg => {
            decode_mjpeg(raw_buffer, width, height, pixel_count, expected_rgb_len)
        }
    }
}

/// Decodes one MJPEG frame into RGB24 with bounded allocations (review finding VIS-02).
///
/// The SOF header of a device-supplied frame is attacker-controlled, so nothing is allocated
/// from it before it has been checked:
/// 1. the compressed input is bounded by [`MAX_MJPEG_COMPRESSED_BYTES`];
/// 2. the negotiated frame dimensions are bounded by [`MAX_MJPEG_DIMENSION`];
/// 3. only the headers are parsed (`read_info`), and the SOF width/height must equal the
///    negotiated frame dimensions (a transposed frame of the same byte count is rejected);
/// 4. only 8-bit YCbCr/RGB and 8-bit greyscale are accepted (UVC MJPEG is always YCbCr);
///    CMYK and 16-bit frames fail closed;
/// 5. the decoder output is capped at the exact expected size, and the decoded length is
///    re-checked before the buffer is returned.
fn decode_mjpeg(
    raw_buffer: &[u8],
    width: u32,
    height: u32,
    pixel_count: usize,
    expected_rgb_len: usize,
) -> Result<Vec<u8>, VisionError> {
    if raw_buffer.len() > MAX_MJPEG_COMPRESSED_BYTES {
        return Err(VisionError::MjpegInputTooLarge {
            max: MAX_MJPEG_COMPRESSED_BYTES,
            actual: raw_buffer.len(),
        });
    }
    if width > MAX_MJPEG_DIMENSION || height > MAX_MJPEG_DIMENSION {
        return Err(VisionError::InvalidDimensions { width, height });
    }

    let mut decoder = jpeg_decoder::Decoder::new(raw_buffer);
    decoder
        .read_info()
        .map_err(|e| VisionError::ColorConversionFailed(format!("JPEG header error: {e}")))?;
    let info = decoder
        .info()
        .ok_or_else(|| VisionError::ColorConversionFailed("Missing JPEG metadata".to_string()))?;

    let header_width = u32::from(info.width);
    let header_height = u32::from(info.height);
    if header_width != width || header_height != height {
        return Err(VisionError::MjpegFrameMismatch {
            expected_width: width,
            expected_height: height,
            actual_width: header_width,
            actual_height: header_height,
        });
    }

    let (decoded_len, is_grey) = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => (expected_rgb_len, false),
        jpeg_decoder::PixelFormat::L8 => (pixel_count, true),
        other => {
            return Err(VisionError::ColorConversionFailed(format!(
                "Unsupported JPEG pixel format {other:?}: MJPEG frames must be 8-bit YCbCr or greyscale"
            )));
        }
    };

    decoder.set_max_decoding_buffer_size(decoded_len);
    let decoded_bytes = decoder
        .decode()
        .map_err(|e| VisionError::ColorConversionFailed(format!("JPEG decode error: {e}")))?;

    if decoded_bytes.len() != decoded_len {
        return Err(VisionError::InvalidBufferSize {
            expected: decoded_len,
            actual: decoded_bytes.len(),
        });
    }

    if !is_grey {
        return Ok(decoded_bytes);
    }

    let mut rgb = Vec::with_capacity(expected_rgb_len);
    for &grey in &decoded_bytes {
        rgb.push(grey);
        rgb.push(grey);
        rgb.push(grey);
    }
    Ok(rgb)
}

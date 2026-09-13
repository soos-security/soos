//! Color conversion from camera pixel formats to standard RGB24.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "High-performance pixel format conversions, fixed-point integer arithmetic, and buffer indexing"
)]

use soos_camera_v4l::PixelFormat;

use crate::error::VisionError;

/// Converts raw frame buffer from a supported camera `PixelFormat` into an RGB24 buffer.
///
/// Output buffer length will be exactly `width * height * 3` bytes.
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
        PixelFormat::Mjpeg => {
            let mut decoder = jpeg_decoder::Decoder::new(raw_buffer);
            let decoded_bytes = decoder.decode().map_err(|e| {
                VisionError::ColorConversionFailed(format!("JPEG decode error: {e}"))
            })?;

            let info = decoder.info().ok_or_else(|| {
                VisionError::ColorConversionFailed("Missing JPEG metadata".to_string())
            })?;

            match info.pixel_format {
                jpeg_decoder::PixelFormat::RGB24 => {
                    if decoded_bytes.len() != expected_rgb_len {
                        return Err(VisionError::InvalidBufferSize {
                            expected: expected_rgb_len,
                            actual: decoded_bytes.len(),
                        });
                    }
                    Ok(decoded_bytes)
                }
                jpeg_decoder::PixelFormat::L8 => {
                    if decoded_bytes.len() != pixel_count {
                        return Err(VisionError::InvalidBufferSize {
                            expected: pixel_count,
                            actual: decoded_bytes.len(),
                        });
                    }
                    let mut rgb = Vec::with_capacity(expected_rgb_len);
                    for &grey in &decoded_bytes {
                        rgb.push(grey);
                        rgb.push(grey);
                        rgb.push(grey);
                    }
                    Ok(rgb)
                }
                jpeg_decoder::PixelFormat::CMYK32 => {
                    // CMYK to RGB conversion: R = 255 * (1-C) * (1-K)
                    let mut rgb = Vec::with_capacity(expected_rgb_len);
                    for cmyk in decoded_bytes.chunks_exact(4) {
                        let c = cmyk[0] as f32 / 255.0;
                        let m = cmyk[1] as f32 / 255.0;
                        let y = cmyk[2] as f32 / 255.0;
                        let k = cmyk[3] as f32 / 255.0;

                        let r = (255.0 * (1.0 - c) * (1.0 - k)).clamp(0.0, 255.0) as u8;
                        let g = (255.0 * (1.0 - m) * (1.0 - k)).clamp(0.0, 255.0) as u8;
                        let b = (255.0 * (1.0 - y) * (1.0 - k)).clamp(0.0, 255.0) as u8;

                        rgb.push(r);
                        rgb.push(g);
                        rgb.push(b);
                    }
                    Ok(rgb)
                }
                jpeg_decoder::PixelFormat::L16 => {
                    let mut rgb = Vec::with_capacity(expected_rgb_len);
                    for chunk in decoded_bytes.chunks_exact(2) {
                        let high = chunk[0];
                        rgb.push(high);
                        rgb.push(high);
                        rgb.push(high);
                    }
                    Ok(rgb)
                }
            }
        }
    }
}

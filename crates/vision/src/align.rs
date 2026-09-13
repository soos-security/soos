//! 5-point facial landmark affine alignment to standard 112x112 ArcFace crop.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "2D similarity transform matrix calculations, bilinear interpolation coordinates, and pixel indexing"
)]

use soos_inference_ort::{FaceLandmarks, Point2f};

use crate::error::VisionError;

/// Canonical ArcFace / InsightFace reference 5-point landmarks for 112x112 canvas.
pub const TARGET_LANDMARKS_112: [Point2f; 5] = [
    Point2f {
        x: 38.2946,
        y: 51.6963,
    }, // Left eye
    Point2f {
        x: 73.5318,
        y: 51.5014,
    }, // Right eye
    Point2f {
        x: 56.0252,
        y: 71.7366,
    }, // Nose tip
    Point2f {
        x: 41.5493,
        y: 92.3655,
    }, // Mouth left corner
    Point2f {
        x: 70.7299,
        y: 92.2041,
    }, // Mouth right corner
];

/// Aligns a face from source RGB24 image into a 112x112 RGB24 crop using 5-point similarity transform.
pub fn align_face_112(
    rgb: &[u8],
    width: u32,
    height: u32,
    landmarks: &FaceLandmarks,
) -> Result<Vec<u8>, VisionError> {
    if width == 0 || height == 0 {
        return Err(VisionError::InvalidDimensions { width, height });
    }

    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(3))
        .ok_or(VisionError::InvalidDimensions { width, height })?;

    if rgb.len() != expected_len {
        return Err(VisionError::InvalidBufferSize {
            expected: expected_len,
            actual: rgb.len(),
        });
    }

    let src_pts = landmarks.as_array();
    let dst_pts = TARGET_LANDMARKS_112;

    // 1. Compute centroids
    let mut mean_src_x = 0.0_f32;
    let mut mean_src_y = 0.0_f32;
    let mut mean_dst_x = 0.0_f32;
    let mut mean_dst_y = 0.0_f32;

    for i in 0..5 {
        mean_src_x += src_pts[i].x;
        mean_src_y += src_pts[i].y;
        mean_dst_x += dst_pts[i].x;
        mean_dst_y += dst_pts[i].y;
    }

    mean_src_x /= 5.0;
    mean_src_y /= 5.0;
    mean_dst_x /= 5.0;
    mean_dst_y /= 5.0;

    // 2. Compute variance and cross-covariance for similarity transform: dst ≈ s * R * src + t
    let mut sum_xx_yy = 0.0_f32;
    let mut num_a = 0.0_f32;
    let mut num_b = 0.0_f32;

    for i in 0..5 {
        let dx_src = src_pts[i].x - mean_src_x;
        let dy_src = src_pts[i].y - mean_src_y;
        let dx_dst = dst_pts[i].x - mean_dst_x;
        let dy_dst = dst_pts[i].y - mean_dst_y;

        sum_xx_yy += dx_src * dx_src + dy_src * dy_src;
        num_a += dx_src * dx_dst + dy_src * dy_dst;
        num_b += dx_src * dy_dst - dy_src * dx_dst;
    }

    if sum_xx_yy <= 1e-6 {
        return Err(VisionError::AlignmentFailed(
            "Degenerate source landmark coordinates".to_string(),
        ));
    }

    let a = num_a / sum_xx_yy;
    let b = num_b / sum_xx_yy;

    // Translation components: t = mean_dst - M * mean_src
    let tx = mean_dst_x - (a * mean_src_x - b * mean_src_y);
    let ty = mean_dst_y - (b * mean_src_x + a * mean_src_y);

    // Inverse mapping matrix: M_inv = (1 / (a^2 + b^2)) * [[a, b], [-b, a]]
    let det = a * a + b * b;
    if det <= 1e-12 {
        return Err(VisionError::AlignmentFailed(
            "Singular similarity transform matrix".to_string(),
        ));
    }

    let c1 = a / det;
    let c2 = b / det;

    const CROP_SIZE: usize = 112;
    let mut output = Vec::with_capacity(CROP_SIZE * CROP_SIZE * 3);

    let max_x = (width - 1) as f32;
    let max_y = (height - 1) as f32;
    let w_usize = width as usize;

    for v in 0..CROP_SIZE {
        let vf = v as f32;
        for u in 0..CROP_SIZE {
            let uf = u as f32;

            // Invert target (u, v) back to source (xs, ys)
            let du = uf - tx;
            let dv = vf - ty;
            let xs = c1 * du + c2 * dv;
            let ys = -c2 * du + c1 * dv;

            if xs >= 0.0 && xs <= max_x && ys >= 0.0 && ys <= max_y {
                let x0 = xs.floor() as usize;
                let y0 = ys.floor() as usize;
                let x1 = (x0 + 1).min(width as usize - 1);
                let y1 = (y0 + 1).min(height as usize - 1);

                let dx = xs - (x0 as f32);
                let dy = ys - (y0 as f32);

                let w00 = (1.0 - dx) * (1.0 - dy);
                let w10 = dx * (1.0 - dy);
                let w01 = (1.0 - dx) * dy;
                let w11 = dx * dy;

                let idx00 = (y0 * w_usize + x0) * 3;
                let idx10 = (y0 * w_usize + x1) * 3;
                let idx01 = (y1 * w_usize + x0) * 3;
                let idx11 = (y1 * w_usize + x1) * 3;

                for c in 0..3 {
                    let p00 = rgb[idx00 + c] as f32;
                    let p10 = rgb[idx10 + c] as f32;
                    let p01 = rgb[idx01 + c] as f32;
                    let p11 = rgb[idx11 + c] as f32;

                    let val = (w00 * p00 + w10 * p10 + w01 * p01 + w11 * p11)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    output.push(val);
                }
            } else {
                // Out-of-bounds area padded with black
                output.push(0);
                output.push(0);
                output.push(0);
            }
        }
    }

    Ok(output)
}

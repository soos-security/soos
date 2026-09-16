//! Test fixtures for facial verification, synthetic frames, and pre-computed embeddings.

#![allow(
    dead_code,
    reason = "Shared test fixtures library used conditionally across test modules"
)]

pub mod synthetic {
    use soos_camera_v4l::{Frame, PixelFormat};

    /// Generates a synthetic RGB24 frame with a solid background and an optional colored rectangle.
    pub fn create_synthetic_rgb_frame(width: u32, height: u32, bg_color: [u8; 3]) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let mut data = Vec::with_capacity(pixel_count.saturating_mul(3));
        for _ in 0..pixel_count {
            data.extend_from_slice(&bg_color);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }

    /// Generates a synthetic YUYV frame (4:2:2).
    pub fn create_synthetic_yuyv_frame(width: u32, height: u32, y: u8, u: u8, v: u8) -> Frame {
        let num_pairs = (width as usize).saturating_mul(height as usize) / 2;
        let mut data = Vec::with_capacity(num_pairs.saturating_mul(4));
        for _ in 0..num_pairs {
            data.extend_from_slice(&[y, u, y, v]);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Yuyv, 1)
    }

    /// Generates a synthetic Grayscale frame.
    pub fn create_synthetic_grey_frame(width: u32, height: u32, value: u8) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let data = vec![value; pixel_count];
        Frame::new(data, width, height, 1_000_000, PixelFormat::Grey, 1)
    }
}

pub mod embeddings {
    /// Pre-computed 128D unit embedding vector for Subject A (Enrollment).
    pub fn subject_a_embedding() -> Vec<f32> {
        let mut vec = vec![0.08838834; 128]; // norm = sqrt(128 * (1/128)) = 1.0
        vec[0] = 0.15;
        // Re-normalize to exact unit length
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        vec.iter().map(|x| x / norm).collect()
    }

    /// Pre-computed 128D unit embedding vector for Subject A (Second capture, slight variation).
    pub fn subject_a_variant_embedding() -> Vec<f32> {
        let mut vec = subject_a_embedding();
        vec[1] += 0.05;
        vec[2] -= 0.03;
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        vec.iter().map(|x| x / norm).collect()
    }

    /// Pre-computed 128D unit embedding vector for Subject B (Different subject, orthogonal-ish).
    pub fn subject_b_embedding() -> Vec<f32> {
        let mut vec = vec![-0.08838834; 128];
        vec[64] = 0.50;
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        vec.iter().map(|x| x / norm).collect()
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_is_multiple_of,
    reason = "Synthetic fixture pixel generation and coordinate math"
)]
pub mod pad {
    use soos_camera_v4l::{Frame, PixelFormat};

    /// Generates a synthetic genuine live face camera frame.
    /// Real skin tones, natural high-frequency texture gradient, without moire or paper border.
    pub fn create_live_face_frame(width: u32, height: u32) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let mut data = Vec::with_capacity(pixel_count.saturating_mul(3));
        for i in 0..pixel_count {
            // Natural skin tone spectrum with subtle organic gradient
            let r = 210u8.saturating_add(((i % 17) as u8) / 2);
            let g = 160u8.saturating_add(((i % 13) as u8) / 2);
            let b = 140u8.saturating_add(((i % 11) as u8) / 2);
            data.push(r);
            data.push(g);
            data.push(b);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }

    /// Generates a synthetic printed photograph presentation attack frame.
    /// Flat reflectance, paper border artifacts, and low dynamic range.
    pub fn create_printed_photo_frame(width: u32, height: u32) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let mut data = Vec::with_capacity(pixel_count.saturating_mul(3));
        for i in 0..pixel_count {
            let x = (i % (width as usize)) as u32;
            let y = (i / (width as usize)) as u32;

            // White paper border around photo
            if x < 10 || x > width.saturating_sub(10) || y < 10 || y > height.saturating_sub(10) {
                data.push(255);
                data.push(255);
                data.push(255);
            } else {
                // Reduced contrast, paper texture noise
                let r = 180u8.saturating_sub(((i % 5) as u8) * 4);
                let g = 140u8.saturating_sub(((i % 5) as u8) * 3);
                let b = 120u8.saturating_sub(((i % 5) as u8) * 3);
                data.push(r);
                data.push(g);
                data.push(b);
            }
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }

    /// Generates a synthetic digital screen replay presentation attack frame.
    /// High-frequency LCD pixel grid, backlight glare, and moire pattern frequency.
    pub fn create_screen_replay_frame(width: u32, height: u32) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let mut data = Vec::with_capacity(pixel_count.saturating_mul(3));
        for i in 0..pixel_count {
            let x = (i % (width as usize)) as u32;
            let y = (i / (width as usize)) as u32;

            // Periodic subpixel RGB grid / moire interference pattern
            let moire = if (x.wrapping_mul(7).wrapping_add(y.wrapping_mul(13))) % 4 == 0 {
                40u8
            } else {
                0u8
            };
            let r = 160u8.saturating_add(moire);
            let g = 190u8.saturating_add(moire); // Slight blue/green screen backlight tint
            let b = 220u8.saturating_add(moire);
            data.push(r);
            data.push(g);
            data.push(b);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }
}

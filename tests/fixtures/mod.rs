//! Test fixtures for facial verification, synthetic frames, and pre-computed embeddings.

pub mod synthetic {
    use soos_camera_v4l::{Frame, PixelFormat};

    /// Generates a synthetic RGB24 frame with a solid background and an optional colored rectangle.
    pub fn create_synthetic_rgb_frame(
        width: u32,
        height: u32,
        bg_color: [u8; 3],
    ) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let mut data = Vec::with_capacity(pixel_count.saturating_mul(3));
        for _ in 0..pixel_count {
            data.extend_from_slice(&bg_color);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }

    /// Generates a synthetic YUYV frame (4:2:2).
    pub fn create_synthetic_yuyv_frame(
        width: u32,
        height: u32,
        y: u8,
        u: u8,
        v: u8,
    ) -> Frame {
        let num_pairs = (width as usize).saturating_mul(height as usize) / 2;
        let mut data = Vec::with_capacity(num_pairs.saturating_mul(4));
        for _ in 0..num_pairs {
            data.extend_from_slice(&[y, u, y, v]);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Yuyv, 1)
    }

    /// Generates a synthetic Grayscale frame.
    pub fn create_synthetic_grey_frame(
        width: u32,
        height: u32,
        value: u8,
    ) -> Frame {
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

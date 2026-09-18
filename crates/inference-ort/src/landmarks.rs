//! 5-point facial landmark structures and detector trait.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Facial landmark geometry, coordinate scaling, and pixel buffer calculations"
)]

use crate::detector::BoundingBox;
use crate::error::InferenceError;
use zeroize::{Zeroize, Zeroizing};

/// 2D floating-point coordinate representing a landmark point on an image plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2f {
    pub x: f32,
    pub y: f32,
}

impl Point2f {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Computes Euclidean distance to another point.
    pub fn distance_to(&self, other: &Point2f) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// 5-point facial landmarks:
/// 1. Left eye center
/// 2. Right eye center
/// 3. Nose tip
/// 4. Left mouth corner
/// 5. Right mouth corner
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceLandmarks {
    pub left_eye: Point2f,
    pub right_eye: Point2f,
    pub nose: Point2f,
    pub mouth_left: Point2f,
    pub mouth_right: Point2f,
}

impl FaceLandmarks {
    pub fn new(
        left_eye: Point2f,
        right_eye: Point2f,
        nose: Point2f,
        mouth_left: Point2f,
        mouth_right: Point2f,
    ) -> Self {
        Self {
            left_eye,
            right_eye,
            nose,
            mouth_left,
            mouth_right,
        }
    }

    /// Converts the 5 landmark points to an array in fixed canonical order.
    pub fn as_array(&self) -> [Point2f; 5] {
        [
            self.left_eye,
            self.right_eye,
            self.nose,
            self.mouth_left,
            self.mouth_right,
        ]
    }

    /// Constructs `FaceLandmarks` from an array of 5 points.
    pub fn from_array(points: [Point2f; 5]) -> Self {
        let [left_eye, right_eye, nose, mouth_left, mouth_right] = points;
        Self {
            left_eye,
            right_eye,
            nose,
            mouth_left,
            mouth_right,
        }
    }

    /// Returns the inter-pupillary distance between left and right eyes.
    pub fn eye_distance(&self) -> f32 {
        self.left_eye.distance_to(&self.right_eye)
    }

    /// Computes the roll angle in radians between eye centers.
    pub fn roll_angle_rad(&self) -> f32 {
        let dy = self.right_eye.y - self.left_eye.y;
        let dx = self.right_eye.x - self.left_eye.x;
        dy.atan2(dx)
    }
}

/// Trait implemented by 5-point facial landmark detector backends.
pub trait LandmarkDetector: Send + Sync {
    /// Estimates 5-point landmarks for a detected face bounding box.
    fn detect_landmarks(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
        face_box: &BoundingBox,
    ) -> Result<FaceLandmarks, InferenceError>;
}

use ort::session::Session;
use std::sync::{Arc, Mutex};

/// 5-point facial landmark detector backed by an ONNX Runtime session.
pub struct OrtLandmarkDetector {
    session: Arc<Mutex<Session>>,
}

impl OrtLandmarkDetector {
    pub fn new(session: Arc<Mutex<Session>>) -> Self {
        Self { session }
    }

    /// Prepares, crops, and normalizes a facial bounding box to 112x112 NCHW format inside a zeroized container.
    pub fn prepare_input(
        rgb: &[u8],
        width: u32,
        height: u32,
        face_box: &BoundingBox,
    ) -> Result<Zeroizing<Vec<f32>>, InferenceError> {
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(3))
            .ok_or_else(|| InferenceError::InvalidInput("Image dimensions overflow".to_string()))?;

        if rgb.len() != expected_len {
            return Err(InferenceError::InvalidBufferSize {
                expected: expected_len,
                actual: rgb.len(),
            });
        }

        let bw = face_box.width();
        let bh = face_box.height();
        if bw <= 1.0 || bh <= 1.0 {
            return Err(InferenceError::LandmarkFailed(
                "Face bounding box too small for landmark detection".to_string(),
            ));
        }

        // Crop and resize face to 112x112 NCHW [1, 3, 112, 112]
        let target_size = 112usize;
        let mut input_data = Zeroizing::new(vec![0.0f32; 3 * target_size * target_size]);

        let scale_x = bw / target_size as f32;
        let scale_y = bh / target_size as f32;

        for y in 0..target_size {
            let src_y = ((face_box.y1 + y as f32 * scale_y) as usize).min(height as usize - 1);
            for x in 0..target_size {
                let src_x = ((face_box.x1 + x as f32 * scale_x) as usize).min(width as usize - 1);
                let src_idx = (src_y * width as usize + src_x) * 3;

                if let (Some(&r), Some(&g), Some(&b)) =
                    (rgb.get(src_idx), rgb.get(src_idx + 1), rgb.get(src_idx + 2))
                {
                    let norm_r = (r as f32 - 127.5) / 128.0;
                    let norm_g = (g as f32 - 127.5) / 128.0;
                    let norm_b = (b as f32 - 127.5) / 128.0;

                    let r_idx = y * target_size + x;
                    let g_idx = target_size * target_size + y * target_size + x;
                    let b_idx = 2 * target_size * target_size + y * target_size + x;

                    if let Some(slot) = input_data.get_mut(r_idx) {
                        *slot = norm_r;
                    }
                    if let Some(slot) = input_data.get_mut(g_idx) {
                        *slot = norm_g;
                    }
                    if let Some(slot) = input_data.get_mut(b_idx) {
                        *slot = norm_b;
                    }
                }
            }
        }

        Ok(input_data)
    }
}

impl LandmarkDetector for OrtLandmarkDetector {
    fn detect_landmarks(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
        face_box: &BoundingBox,
    ) -> Result<FaceLandmarks, InferenceError> {
        let bw = face_box.width();
        let bh = face_box.height();
        let mut input_data = Self::prepare_input(rgb, width, height, face_box)?;

        let tensor =
            ort::value::TensorRef::from_array_view(([1usize, 3, 112, 112], input_data.as_slice()))
                .map_err(|e| InferenceError::Ort(e.to_string()))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| InferenceError::LandmarkFailed("Session mutex poisoned".to_string()))?;

        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|e| InferenceError::Ort(e.to_string()))?;

        // Zeroize input buffer immediately post-inference
        input_data.zeroize();

        let mut out_iter = outputs.into_iter();
        let (_, lm_tensor) = out_iter.next().ok_or_else(|| {
            InferenceError::LandmarkFailed(
                "Landmark model returned zero output tensors".to_string(),
            )
        })?;

        let lm_data = lm_tensor
            .try_extract_tensor::<f32>()
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .1;

        if lm_data.len() < 10 {
            return Err(InferenceError::LandmarkFailed(format!(
                "Expected at least 10 landmark coordinates, got {}",
                lm_data.len()
            )));
        }

        // Map 5 points back to absolute image plane
        let mut points = [Point2f::new(0.0, 0.0); 5];
        for i in 0..5 {
            let rx = lm_data.get(i * 2).copied().unwrap_or(0.0);
            let ry = lm_data.get(i * 2 + 1).copied().unwrap_or(0.0);

            // Normalized coordinates in [0, 1] or relative [0, 112]
            let norm_rx = if rx > 1.0 { rx / 112.0 } else { rx };
            let norm_ry = if ry > 1.0 { ry / 112.0 } else { ry };

            let abs_x = face_box.x1 + norm_rx * bw;
            let abs_y = face_box.y1 + norm_ry * bh;

            if let Some(p) = points.get_mut(i) {
                *p = Point2f::new(abs_x, abs_y);
            }
        }

        Ok(FaceLandmarks::from_array(points))
    }
}

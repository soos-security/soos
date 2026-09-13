//! Hardware-free deterministic mocks for FaceDetector, LandmarkDetector, and EmbeddingExtractor.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Mock synthetic frame generation, coordinate interpolation, and mock vector generation"
)]

use std::sync::RwLock;

use crate::detector::{BoundingBox, FaceDetection, FaceDetector};
use crate::embedding::{BiometricEmbedding, EmbeddingExtractor};
use crate::error::InferenceError;
use crate::landmarks::{FaceLandmarks, LandmarkDetector, Point2f};

/// Mock face detector for automated tests and headless environments.
pub struct MockFaceDetector {
    detections: RwLock<Vec<FaceDetection>>,
    fail_next: RwLock<bool>,
}

impl MockFaceDetector {
    /// Creates a mock detector that returns no detected faces.
    pub fn new_empty() -> Self {
        Self {
            detections: RwLock::new(Vec::new()),
            fail_next: RwLock::new(false),
        }
    }

    /// Creates a mock detector with pre-configured detections.
    pub fn new_with_detections(detections: Vec<FaceDetection>) -> Self {
        Self {
            detections: RwLock::new(detections),
            fail_next: RwLock::new(false),
        }
    }

    /// Creates a mock detector returning a centered face bounding box.
    pub fn new_centered_face(width: u32, height: u32, score: f32) -> Self {
        let w = width as f32;
        let h = height as f32;
        let box_ = BoundingBox::new(w * 0.25, h * 0.2, w * 0.75, h * 0.8);
        Self::new_with_detections(vec![FaceDetection::new(box_, score)])
    }

    /// Updates the mock detections to return.
    pub fn set_detections(&self, detections: Vec<FaceDetection>) {
        if let Ok(mut guard) = self.detections.write() {
            *guard = detections;
        }
    }

    /// Injects a fault on the next detection call.
    pub fn set_fail_next(&self, fail: bool) {
        if let Ok(mut guard) = self.fail_next.write() {
            *guard = fail;
        }
    }
}

impl FaceDetector for MockFaceDetector {
    fn detect(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<FaceDetection>, InferenceError> {
        if let Ok(mut guard) = self.fail_next.write() {
            if *guard {
                *guard = false;
                return Err(InferenceError::DetectionFailed(
                    "Simulated detector failure".to_string(),
                ));
            }
        }

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

        let guard = self
            .detections
            .read()
            .map_err(|_| InferenceError::DetectionFailed("Lock poisoned".to_string()))?;

        Ok(guard.clone())
    }
}

/// Mock 5-point landmark detector.
pub struct MockLandmarkDetector {
    landmarks: RwLock<FaceLandmarks>,
    fail_next: RwLock<bool>,
}

impl MockLandmarkDetector {
    /// Creates a mock landmark detector with canonical normalized 5-point layout.
    pub fn new_canonical() -> Self {
        let lm = FaceLandmarks::new(
            Point2f::new(38.29, 51.69),
            Point2f::new(73.53, 51.50),
            Point2f::new(56.02, 71.73),
            Point2f::new(41.54, 92.36),
            Point2f::new(70.72, 92.20),
        );
        Self {
            landmarks: RwLock::new(lm),
            fail_next: RwLock::new(false),
        }
    }

    pub fn new_with_landmarks(landmarks: FaceLandmarks) -> Self {
        Self {
            landmarks: RwLock::new(landmarks),
            fail_next: RwLock::new(false),
        }
    }

    pub fn set_fail_next(&self, fail: bool) {
        if let Ok(mut guard) = self.fail_next.write() {
            *guard = fail;
        }
    }
}

impl LandmarkDetector for MockLandmarkDetector {
    fn detect_landmarks(
        &self,
        _rgb: &[u8],
        _width: u32,
        _height: u32,
        face_box: &BoundingBox,
    ) -> Result<FaceLandmarks, InferenceError> {
        if let Ok(mut guard) = self.fail_next.write() {
            if *guard {
                *guard = false;
                return Err(InferenceError::LandmarkFailed(
                    "Simulated landmark detector failure".to_string(),
                ));
            }
        }

        let guard = self
            .landmarks
            .read()
            .map_err(|_| InferenceError::LandmarkFailed("Lock poisoned".to_string()))?;

        // Scale canonical landmarks to the face bounding box
        let bw = face_box.width();
        let bh = face_box.height();
        let scale_x = bw / 112.0;
        let scale_y = bh / 112.0;

        let scaled = FaceLandmarks::new(
            Point2f::new(
                face_box.x1 + guard.left_eye.x * scale_x,
                face_box.y1 + guard.left_eye.y * scale_y,
            ),
            Point2f::new(
                face_box.x1 + guard.right_eye.x * scale_x,
                face_box.y1 + guard.right_eye.y * scale_y,
            ),
            Point2f::new(
                face_box.x1 + guard.nose.x * scale_x,
                face_box.y1 + guard.nose.y * scale_y,
            ),
            Point2f::new(
                face_box.x1 + guard.mouth_left.x * scale_x,
                face_box.y1 + guard.mouth_left.y * scale_y,
            ),
            Point2f::new(
                face_box.x1 + guard.mouth_right.x * scale_x,
                face_box.y1 + guard.mouth_right.y * scale_y,
            ),
        );

        Ok(scaled)
    }
}

/// Mock embedding extractor generating deterministic L2-normalized vectors.
pub struct MockEmbeddingExtractor {
    dim: usize,
    base_seed: f32,
}

impl MockEmbeddingExtractor {
    /// Creates a mock extractor that produces L2-normalized vectors of length `dim`.
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            base_seed: 1.0,
        }
    }

    /// Creates a mock extractor with a custom seed to generate distinct identity embeddings.
    pub fn with_seed(dim: usize, base_seed: f32) -> Self {
        Self { dim, base_seed }
    }
}

impl EmbeddingExtractor for MockEmbeddingExtractor {
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError> {
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(3))
            .ok_or_else(|| InferenceError::InvalidInput("Image dimensions overflow".to_string()))?;

        if aligned_crop_rgb.len() != expected_len {
            return Err(InferenceError::InvalidBufferSize {
                expected: expected_len,
                actual: aligned_crop_rgb.len(),
            });
        }

        // Compute deterministic non-zero vector
        let mut vec = Vec::with_capacity(self.dim);
        let pixel_sum: f32 = aligned_crop_rgb.iter().take(100).map(|&b| b as f32).sum();

        for i in 0..self.dim {
            let angle = (i as f32 * 0.1) + self.base_seed + (pixel_sum * 0.0001);
            vec.push(angle.sin());
        }

        let mut embedding = BiometricEmbedding::new(vec);
        embedding.normalize()?;
        Ok(embedding)
    }
}

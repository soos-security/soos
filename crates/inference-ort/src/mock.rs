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
use crate::pad::{AttackType, PadDetector, PadResult};

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

impl Default for MockLandmarkDetector {
    fn default() -> Self {
        Self::new_canonical()
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

/// Mock embedding extractor generating deterministic L2-normalized vectors (defaults to 512D for ArcFace w600k).
pub struct MockEmbeddingExtractor {
    dim: usize,
    base_seed: f32,
}

impl MockEmbeddingExtractor {
    /// Default embedding dimensionality matching ArcFace w600k (512 dimensions).
    pub const DEFAULT_DIM: usize = 512;

    /// Creates a mock extractor that produces L2-normalized vectors of length `dim`.
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            base_seed: 1.0,
        }
    }

    /// Creates a mock extractor with default 512-dimensional output for ArcFace w600k.
    pub fn new_default() -> Self {
        Self::new(Self::DEFAULT_DIM)
    }

    /// Returns the embedding dimensionality configured for this mock extractor.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Creates a mock extractor with a custom seed to generate distinct identity embeddings.
    pub fn with_seed(dim: usize, base_seed: f32) -> Self {
        Self { dim, base_seed }
    }
}

impl Default for MockEmbeddingExtractor {
    /// Default constructor returning a 512D mock embedding extractor.
    fn default() -> Self {
        Self::new_default()
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

/// Mock presentation attack detector for automated tests and headless environments.
pub struct MockPadDetector {
    result: RwLock<PadResult>,
    fail_next: RwLock<bool>,
}

impl MockPadDetector {
    /// Creates a mock detector that returns a genuine live face verdict.
    pub fn new_live() -> Self {
        Self {
            result: RwLock::new(PadResult::live(0.98)),
            fail_next: RwLock::new(false),
        }
    }

    /// Creates a mock detector that returns a presentation attack (spoof) verdict.
    pub fn new_spoof(attack_type: AttackType, score: f32) -> Self {
        Self {
            result: RwLock::new(PadResult::spoof(score, attack_type)),
            fail_next: RwLock::new(false),
        }
    }

    /// Creates a mock detector with an explicit initial result.
    pub fn new_with_result(result: PadResult) -> Self {
        Self {
            result: RwLock::new(result),
            fail_next: RwLock::new(false),
        }
    }

    /// Updates the configured mock result.
    pub fn set_result(&self, result: PadResult) {
        if let Ok(mut guard) = self.result.write() {
            *guard = result;
        }
    }

    /// Injects a fault on the next evaluation call.
    pub fn set_fail_next(&self, fail: bool) {
        if let Ok(mut guard) = self.fail_next.write() {
            *guard = fail;
        }
    }
}

impl PadDetector for MockPadDetector {
    fn evaluate_liveness(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError> {
        if let Ok(mut guard) = self.fail_next.write() {
            if *guard {
                *guard = false;
                return Err(InferenceError::PadFailed(
                    "Simulated PAD inference failure".to_string(),
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
            .result
            .read()
            .map_err(|_| InferenceError::PadFailed("Lock poisoned".to_string()))?;

        Ok(guard.clone())
    }
}

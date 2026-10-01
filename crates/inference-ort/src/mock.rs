//! Hardware-free deterministic mocks for FaceDetector, PadDetector, and EmbeddingExtractor.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Mock synthetic frame generation, coordinate interpolation, and mock vector generation"
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

use crate::detector::{BoundingBox, FaceDetection, FaceDetector};
use crate::embedding::{BiometricEmbedding, EmbeddingExtractor};
use crate::error::InferenceError;
use crate::landmarks::{FaceLandmarks, Point2f};
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

    /// Computes canonical 5-point facial landmarks scaled to the provided bounding box.
    ///
    /// Uses standard ArcFace / InsightFace 112x112 canonical reference points:
    /// - Left eye: (38.2946, 51.6963)
    /// - Right eye: (73.5318, 51.5014)
    /// - Nose: (56.0252, 71.7366)
    /// - Mouth left: (41.5493, 92.3655)
    /// - Mouth right: (70.7299, 92.2041)
    pub fn canonical_landmarks_for_box(face_box: &BoundingBox) -> FaceLandmarks {
        let bw = face_box.width();
        let bh = face_box.height();
        let scale_x = bw / 112.0;
        let scale_y = bh / 112.0;

        FaceLandmarks::new(
            Point2f::new(
                face_box.x1 + 38.2946 * scale_x,
                face_box.y1 + 51.6963 * scale_y,
            ),
            Point2f::new(
                face_box.x1 + 73.5318 * scale_x,
                face_box.y1 + 51.5014 * scale_y,
            ),
            Point2f::new(
                face_box.x1 + 56.0252 * scale_x,
                face_box.y1 + 71.7366 * scale_y,
            ),
            Point2f::new(
                face_box.x1 + 41.5493 * scale_x,
                face_box.y1 + 92.3655 * scale_y,
            ),
            Point2f::new(
                face_box.x1 + 70.7299 * scale_x,
                face_box.y1 + 92.2041 * scale_y,
            ),
        )
    }

    /// Creates a mock detector returning a centered face bounding box with canonical landmarks scaled to the box.
    pub fn new_centered_face(width: u32, height: u32, score: f32) -> Self {
        let w = width as f32;
        let h = height as f32;
        let box_ = BoundingBox::new(w * 0.25, h * 0.2, w * 0.75, h * 0.8);
        let landmarks = Self::canonical_landmarks_for_box(&box_);
        Self::new_with_detections(vec![FaceDetection::with_landmarks(box_, score, landmarks)])
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

/// Mock embedding extractor generating deterministic L2-normalized vectors (defaults to the
/// shipped 128-D SFace dimension, `EMBEDDING_DIMENSION`).
pub struct MockEmbeddingExtractor {
    dim: usize,
    base_seed: f32,
}

impl MockEmbeddingExtractor {
    /// Default embedding dimensionality: the shipped model's (`EMBEDDING_DIMENSION`, 128).
    pub const DEFAULT_DIM: usize = crate::embedding::EMBEDDING_DIMENSION;

    /// Creates a mock extractor that produces L2-normalized vectors of length `dim`.
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            base_seed: 1.0,
        }
    }

    /// Creates a mock extractor with the default (shipped model) output dimension.
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
    /// Default constructor returning a mock extractor of the shipped dimension.
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

    fn output_dimension(&self) -> Option<usize> {
        Some(self.dim)
    }
}

/// Mock presentation attack detector for automated tests and headless environments.
pub struct MockPadDetector {
    result: RwLock<PadResult>,
    /// Optional per-call result sequence cycled round-robin (empty = use `result`).
    sequence: RwLock<Vec<PadResult>>,
    cursor: AtomicUsize,
    calls: AtomicUsize,
    fail_next: RwLock<bool>,
}

impl MockPadDetector {
    /// Creates a mock detector that returns a genuine live face verdict.
    pub fn new_live() -> Self {
        Self::new_with_result(PadResult::live(0.98))
    }

    /// Creates a mock detector that returns a presentation attack (spoof) verdict.
    pub fn new_spoof(attack_type: AttackType, score: f32) -> Self {
        Self::new_with_result(PadResult::spoof(score, attack_type))
    }

    /// Creates a mock detector with an explicit initial result.
    pub fn new_with_result(result: PadResult) -> Self {
        Self {
            result: RwLock::new(result),
            sequence: RwLock::new(Vec::new()),
            cursor: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            fail_next: RwLock::new(false),
        }
    }

    /// Updates the configured mock result and clears any per-call result sequence.
    pub fn set_result(&self, result: PadResult) {
        if let Ok(mut guard) = self.result.write() {
            *guard = result;
        }
        if let Ok(mut seq) = self.sequence.write() {
            seq.clear();
        }
        self.cursor.store(0, Ordering::SeqCst);
    }

    /// Configures a per-call result sequence, cycled round-robin on every evaluation.
    /// An empty sequence restores the single configured result.
    pub fn set_result_sequence(&self, results: Vec<PadResult>) {
        if let Ok(mut seq) = self.sequence.write() {
            *seq = results;
        }
        self.cursor.store(0, Ordering::SeqCst);
    }

    /// Number of `evaluate_liveness` calls observed so far (including injected failures).
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
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
        self.calls.fetch_add(1, Ordering::SeqCst);
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

        if let Ok(seq) = self.sequence.read() {
            if let Some(len) = std::num::NonZeroUsize::new(seq.len()) {
                let idx = self.cursor.fetch_add(1, Ordering::SeqCst) % len.get();
                if let Some(next) = seq.get(idx) {
                    return Ok(next.clone());
                }
            }
        }

        let guard = self
            .result
            .read()
            .map_err(|_| InferenceError::PadFailed("Lock poisoned".to_string()))?;

        Ok(guard.clone())
    }
}

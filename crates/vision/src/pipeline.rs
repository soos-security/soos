//! High-level vision verification pipeline orchestrator.

use std::sync::Arc;

use soos_camera_v4l::Frame;
use soos_inference_ort::{
    BiometricEmbedding, EmbeddingExtractor, FaceDetection, FaceDetector, FaceLandmarks,
    LandmarkDetector,
};

use crate::align::align_face_112;
use crate::color::convert_to_rgb;
use crate::error::VisionError;
use crate::matcher::{match_embeddings, MatchResult};

/// Configuration options for the vision verification pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct VisionPipelineConfig {
    /// Minimum detection confidence required to accept a face candidate.
    pub min_face_confidence: f32,
    /// Cosine similarity threshold required to grant biometric match.
    pub match_threshold: f32,
    /// Target aligned face width in pixels (standard 112).
    pub target_width: u32,
    /// Target aligned face height in pixels (standard 112).
    pub target_height: u32,
}

impl Default for VisionPipelineConfig {
    fn default() -> Self {
        Self {
            min_face_confidence: 0.70,
            match_threshold: 0.45,
            target_width: 112,
            target_height: 112,
        }
    }
}

/// Output of a single processed frame through the vision pipeline.
#[derive(Debug, Clone)]
pub struct PipelineOutput {
    /// Primary bounding box and detection score.
    pub detection: FaceDetection,
    /// 5-point facial landmarks.
    pub landmarks: FaceLandmarks,
    /// 112x112 aligned face crop in RGB24 format.
    pub aligned_crop_rgb: Vec<u8>,
    /// Extracted L2-normalized biometric embedding.
    pub embedding: BiometricEmbedding,
}

/// Verification outcome containing both pipeline output and biometric comparison.
#[derive(Debug, Clone)]
pub struct VerificationOutcome {
    /// Intermediate outputs from frame processing.
    pub output: PipelineOutput,
    /// Biometric match comparison against the enrolled template.
    pub match_result: MatchResult,
}

/// End-to-end vision processing orchestrator.
pub struct VisionPipeline {
    detector: Arc<dyn FaceDetector>,
    landmarks: Arc<dyn LandmarkDetector>,
    extractor: Arc<dyn EmbeddingExtractor>,
    config: VisionPipelineConfig,
}

impl VisionPipeline {
    /// Constructs a new `VisionPipeline` with the provided neural inference backends.
    pub fn new(
        detector: Arc<dyn FaceDetector>,
        landmarks: Arc<dyn LandmarkDetector>,
        extractor: Arc<dyn EmbeddingExtractor>,
        config: VisionPipelineConfig,
    ) -> Self {
        Self {
            detector,
            landmarks,
            extractor,
            config,
        }
    }

    /// Access the pipeline configuration.
    pub fn config(&self) -> &VisionPipelineConfig {
        &self.config
    }

    /// Processes a single camera frame:
    /// 1. Color converts to RGB24
    /// 2. Detects faces; enforces single-face security invariant (rejects 0 or >1 faces)
    /// 3. Detects 5-point facial landmarks
    /// 4. Warps face to normalized 112x112 RGB crop
    /// 5. Extracts L2-normalized biometric embedding
    pub fn process_frame(&self, frame: &Frame) -> Result<PipelineOutput, VisionError> {
        let rgb = convert_to_rgb(&frame.data, frame.width, frame.height, frame.format)?;

        let mut detections = self.detector.detect(&rgb, frame.width, frame.height)?;

        if detections.is_empty() {
            return Err(VisionError::NoFaceDetected);
        }

        if detections.len() > 1 {
            return Err(VisionError::MultipleFacesDetected {
                count: detections.len(),
            });
        }

        // Single face detected: pop it safely without indexing
        let detection = detections.pop().ok_or(VisionError::NoFaceDetected)?;

        if detection.score < self.config.min_face_confidence {
            return Err(VisionError::FaceBelowConfidence {
                confidence: detection.score,
                min_confidence: self.config.min_face_confidence,
            });
        }

        let landmarks =
            self.landmarks
                .detect_landmarks(&rgb, frame.width, frame.height, &detection.box_)?;

        let aligned_crop = align_face_112(&rgb, frame.width, frame.height, &landmarks)?;

        let embedding = self.extractor.extract_embedding(
            &aligned_crop,
            self.config.target_width,
            self.config.target_height,
        )?;

        Ok(PipelineOutput {
            detection,
            landmarks,
            aligned_crop_rgb: aligned_crop,
            embedding,
        })
    }

    /// End-to-end verification against an enrolled biometric template.
    pub fn verify(
        &self,
        frame: &Frame,
        enrolled_template: &BiometricEmbedding,
    ) -> Result<VerificationOutcome, VisionError> {
        let output = self.process_frame(frame)?;
        let match_result = match_embeddings(
            enrolled_template,
            &output.embedding,
            self.config.match_threshold,
        )?;

        Ok(VerificationOutcome {
            output,
            match_result,
        })
    }
}

//! High-level vision verification pipeline orchestrator.

use std::sync::Arc;

use soos_camera_v4l::Frame;
use soos_inference_ort::{
    AttackType, BiometricEmbedding, EmbeddingExtractor, FaceDetection, FaceDetector, FaceLandmarks,
    PadDetector, PadResult,
};

use crate::align::align_face_112;
use crate::color::convert_to_rgb;
use crate::crop::{crop_and_resize, expand_bbox_for_pad};
use crate::error::VisionError;
use crate::ir_liveness::{evaluate_ir_gate, PadInputModality, DEFAULT_IR_PAD_THRESHOLD};
use crate::matcher::{match_embeddings, MatchResult};
use zeroize::{Zeroize, Zeroizing};

/// Configuration options for the vision verification pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct VisionPipelineConfig {
    /// Minimum detection confidence required to accept a face candidate.
    pub min_face_confidence: f32,
    /// Cosine similarity threshold required to grant biometric match.
    pub match_threshold: f32,
    /// Presentation attack detection (liveness) score threshold for colour frames (standard 0.85).
    pub pad_threshold: f32,
    /// Liveness score threshold for monochrome (IR, `PixelFormat::Grey`) frames.
    ///
    /// The effective IR threshold is `max(pad_threshold, ir_pad_threshold)`, so it is never
    /// looser than the colour threshold. Default [`DEFAULT_IR_PAD_THRESHOLD`] is an
    /// uncalibrated conservative floor (GitHub #169; calibration tracked by GitHub #172).
    pub ir_pad_threshold: f32,
    /// Target aligned face width in pixels (standard 112).
    pub target_width: u32,
    /// Target aligned face height in pixels (standard 112).
    pub target_height: u32,
    /// Target expanded face crop width in pixels for PAD (standard 80).
    pub pad_target_width: u32,
    /// Target expanded face crop height in pixels for PAD (standard 80).
    pub pad_target_height: u32,
    /// Bounding box expansion scale factor for PAD context crop (standard 2.7).
    pub pad_bbox_scale: f32,
}

impl Default for VisionPipelineConfig {
    fn default() -> Self {
        Self {
            min_face_confidence: 0.70,
            match_threshold: 0.70,
            pad_threshold: 0.85,
            ir_pad_threshold: DEFAULT_IR_PAD_THRESHOLD,
            target_width: 112,
            target_height: 112,
            pad_target_width: 80,
            pad_target_height: 80,
            pad_bbox_scale: 2.7,
        }
    }
}

impl VisionPipelineConfig {
    /// Liveness threshold applied to a PAD crop of the given modality.
    ///
    /// Colour frames use `pad_threshold`. Monochrome frames use
    /// `max(pad_threshold, ir_pad_threshold)`; if both values are non-finite the result is
    /// NaN, which rejects every score (comparisons are written fail-closed).
    pub fn effective_pad_threshold(&self, modality: PadInputModality) -> f32 {
        match modality {
            PadInputModality::Color => self.pad_threshold,
            PadInputModality::Monochrome => self.pad_threshold.max(self.ir_pad_threshold),
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
    /// Presentation attack detection evaluation result.
    pub pad_result: PadResult,
    /// Extracted L2-normalized biometric embedding.
    pub embedding: BiometricEmbedding,
}

impl zeroize::Zeroize for PipelineOutput {
    fn zeroize(&mut self) {
        self.aligned_crop_rgb.zeroize();
        self.embedding.zeroize();
    }
}

impl Drop for PipelineOutput {
    fn drop(&mut self) {
        self.zeroize();
    }
}

/// Verification outcome containing both pipeline output and biometric comparison.
#[derive(Debug, Clone)]
pub struct VerificationOutcome {
    /// Intermediate outputs from frame processing.
    pub output: PipelineOutput,
    /// Biometric match comparison against the enrolled template.
    pub match_result: MatchResult,
}

impl zeroize::Zeroize for VerificationOutcome {
    fn zeroize(&mut self) {
        self.output.zeroize();
    }
}

impl Drop for VerificationOutcome {
    fn drop(&mut self) {
        self.zeroize();
    }
}

/// Diagnostic and real-time visualization output for GUI and monitoring tools.
#[derive(Debug, Clone)]
pub struct VisionAnalysis {
    /// Frame converted to RGB24.
    pub rgb: Zeroizing<Vec<u8>>,
    /// All raw face detections from the detector.
    pub detections: Vec<FaceDetection>,
    /// Anti-spoofing PAD evaluation result if a primary face is detected.
    pub pad_result: Option<PadResult>,
    /// Estimated head rotation pose in degrees if landmarks are available.
    pub pose: Option<crate::pose::HeadPose>,
    /// 112x112 normalized aligned face crop for visual preview.
    pub aligned_crop: Option<Zeroizing<Vec<u8>>>,
    /// 512D biometric embedding if feature extraction succeeded.
    pub embedding: Option<Zeroizing<Vec<f32>>>,
}

impl zeroize::Zeroize for VisionAnalysis {
    fn zeroize(&mut self) {
        if let Some(crop) = &mut self.aligned_crop {
            crop.zeroize();
        }
        if let Some(emb) = &mut self.embedding {
            emb.zeroize();
        }
    }
}

impl Drop for VisionAnalysis {
    fn drop(&mut self) {
        self.zeroize();
    }
}

/// End-to-end vision processing orchestrator.
pub struct VisionPipeline {
    detector: Arc<dyn FaceDetector>,
    pad: Arc<dyn PadDetector>,
    extractor: Arc<dyn EmbeddingExtractor>,
    config: VisionPipelineConfig,
}

/// RAII guard that deterministically zeroizes the aligned face crop if processing fails before output transfer.
struct AlignedCropGuard {
    crop: Vec<u8>,
    disarmed: bool,
}

impl Drop for AlignedCropGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            self.crop.zeroize();
        }
    }
}

impl VisionPipeline {
    /// Constructs a new `VisionPipeline` with the provided 3 neural inference backends.
    pub fn new(
        detector: Arc<dyn FaceDetector>,
        pad: Arc<dyn PadDetector>,
        extractor: Arc<dyn EmbeddingExtractor>,
        config: VisionPipelineConfig,
    ) -> Self {
        Self {
            detector,
            pad,
            extractor,
            config,
        }
    }

    /// Access the pipeline configuration.
    pub fn config(&self) -> &VisionPipelineConfig {
        &self.config
    }

    /// Access the face detector.
    pub fn detector(&self) -> &Arc<dyn FaceDetector> {
        &self.detector
    }

    /// Access the presentation attack detector.
    pub fn pad(&self) -> &Arc<dyn PadDetector> {
        &self.pad
    }

    /// Access the feature embedding extractor.
    pub fn extractor(&self) -> &Arc<dyn EmbeddingExtractor> {
        &self.extractor
    }

    /// Analyzes a camera frame without fail-closed short circuiting for GUI live inspection.
    pub fn analyze_frame(&self, frame: &Frame) -> Result<VisionAnalysis, VisionError> {
        let rgb = Zeroizing::new(convert_to_rgb(
            &frame.data,
            frame.width,
            frame.height,
            frame.format,
        )?);

        let detections = self.detector.detect(&rgb, frame.width, frame.height)?;

        let primary = detections
            .iter()
            .filter(|d| d.score >= self.config.min_face_confidence)
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        let mut pad_result = None;
        let mut pose = None;
        let mut aligned_crop = None;
        let mut embedding = None;

        if let Some(det) = primary {
            if let Some(landmarks) = &det.landmarks {
                pose = Some(crate::pose::estimate_head_pose(landmarks));

                let expanded_bbox = expand_bbox_for_pad(
                    &det.box_,
                    self.config.pad_bbox_scale,
                    frame.width,
                    frame.height,
                );

                if let Ok(crop) = crop_and_resize(
                    &rgb,
                    frame.width,
                    frame.height,
                    &expanded_bbox,
                    self.config.pad_target_width,
                    self.config.pad_target_height,
                ) {
                    let pad_crop = Zeroizing::new(crop);
                    pad_result =
                        self.analyze_pad_crop(&pad_crop, PadInputModality::for_frame(frame));
                }

                if let Ok(aligned) = align_face_112(&rgb, frame.width, frame.height, landmarks) {
                    if let Ok(emb) = self.extractor.extract_embedding(
                        &aligned,
                        self.config.target_width,
                        self.config.target_height,
                    ) {
                        embedding = Some(Zeroizing::new(emb.as_slice().to_vec()));
                    }
                    aligned_crop = Some(Zeroizing::new(aligned));
                }
            }
        }

        Ok(VisionAnalysis {
            rgb,
            detections,
            pad_result,
            pose,
            aligned_crop,
            embedding,
        })
    }

    /// Processes a single camera frame:
    /// 1. Color converts to RGB24
    /// 2. Detects faces; enforces single-face security invariant (rejects 0 or >1 faces)
    /// 3. Extracts 5-point facial landmarks from detection
    /// 4. Crops and resizes 2.7x expanded bounding box to 80x80 for PAD
    /// 5. Evaluates Presentation Attack Detection (PAD) liveness; short-circuits on spoof.
    ///    `Grey` frames and every frame from an `Infrared` sensor (whatever its pixel format)
    ///    first pass the fail-closed IR gate and are scored against the stricter IR threshold
    ///    (GitHub #169); they never take the colour PAD path.
    /// 6. Warps face to normalized 112x112 RGB crop using landmarks
    /// 7. Extracts L2-normalized biometric embedding
    pub fn process_frame(&self, frame: &Frame) -> Result<PipelineOutput, VisionError> {
        let rgb = Zeroizing::new(convert_to_rgb(
            &frame.data,
            frame.width,
            frame.height,
            frame.format,
        )?);

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

        let landmarks = detection.landmarks.ok_or(VisionError::MissingLandmarks)?;

        // Step 4: Presentation Attack Detection using 2.7x expanded bounding box context crop
        let expanded_bbox = expand_bbox_for_pad(
            &detection.box_,
            self.config.pad_bbox_scale,
            frame.width,
            frame.height,
        );

        let pad_crop = Zeroizing::new(crop_and_resize(
            &rgb,
            frame.width,
            frame.height,
            &expanded_bbox,
            self.config.pad_target_width,
            self.config.pad_target_height,
        )?);

        let pad_result = self.evaluate_pad_crop(&pad_crop, PadInputModality::for_frame(frame))?;

        // Step 5: Affine alignment to 112x112 using landmarks for recognition embedding
        let mut aligned_crop_guard = AlignedCropGuard {
            crop: align_face_112(&rgb, frame.width, frame.height, &landmarks)?,
            disarmed: false,
        };

        // Step 6: Feature extraction only if PAD passed
        let embedding = self.extractor.extract_embedding(
            &aligned_crop_guard.crop,
            self.config.target_width,
            self.config.target_height,
        )?;

        aligned_crop_guard.disarmed = true;
        let aligned_crop = std::mem::take(&mut aligned_crop_guard.crop);

        Ok(PipelineOutput {
            detection,
            landmarks,
            aligned_crop_rgb: aligned_crop,
            pad_result,
            embedding,
        })
    }

    /// Modality-aware PAD decision for a context crop (modality from the frame's pixel format
    /// and sensor type, see [`PadInputModality::for_frame`]).
    ///
    /// Colour frames: model result must be live and `score >= pad_threshold`.
    /// Monochrome frames: the IR gate must pass (otherwise the model is not consulted), then
    /// the model result must be live and reach the effective IR threshold. Every comparison
    /// rejects non-finite scores or thresholds.
    fn evaluate_pad_crop(
        &self,
        pad_crop: &[u8],
        modality: PadInputModality,
    ) -> Result<PadResult, VisionError> {
        if modality.is_monochrome() {
            evaluate_ir_gate(
                pad_crop,
                self.config.pad_target_width,
                self.config.pad_target_height,
            )
            .map_err(|reason| VisionError::IrLivenessGateFailed { reason })?;
        }

        let pad_result = self.pad.evaluate_liveness(
            pad_crop,
            self.config.pad_target_width,
            self.config.pad_target_height,
        )?;

        let threshold = self.config.effective_pad_threshold(modality);
        // Written so that a NaN score or threshold always rejects (fail-closed).
        let passes =
            pad_result.is_live && pad_result.score.is_finite() && pad_result.score >= threshold;
        if !passes {
            return Err(VisionError::PadFailed {
                score: pad_result.score,
                threshold,
            });
        }
        Ok(pad_result)
    }

    /// Non-short-circuiting PAD evaluation for GUI analysis.
    ///
    /// Colour frames report the raw model result. Monochrome frames report a spoof result
    /// (`is_live = false`) when the IR gate rejects or the score is below the IR threshold,
    /// so the GUI never shows an IR capture as live when the daemon would reject it.
    fn analyze_pad_crop(&self, pad_crop: &[u8], modality: PadInputModality) -> Option<PadResult> {
        if !modality.is_monochrome() {
            return self
                .pad
                .evaluate_liveness(
                    pad_crop,
                    self.config.pad_target_width,
                    self.config.pad_target_height,
                )
                .ok();
        }
        match self.evaluate_pad_crop(pad_crop, modality) {
            Ok(result) => Some(result),
            Err(VisionError::IrLivenessGateFailed { .. }) => {
                Some(PadResult::spoof(0.0, AttackType::UnknownSpoof))
            }
            Err(VisionError::PadFailed { score, .. }) => {
                Some(PadResult::spoof(score, AttackType::UnknownSpoof))
            }
            Err(_) => None,
        }
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

//! High-level vision verification pipeline orchestrator.

use std::sync::Arc;

use soos_camera_v4l::Frame;
use soos_inference_ort::{
    AttackType, BiometricEmbedding, EmbeddingExtractor, FaceDetection, FaceDetector, FaceLandmarks,
    PadDetector, PadResult,
};

use crate::align::align_face_112;
use crate::color::convert_to_rgb;
use crate::crop::crop_pad_context;
use crate::error::VisionError;
use crate::ir_liveness::{evaluate_ir_gate, PadInputModality, DEFAULT_IR_PAD_THRESHOLD};
use crate::matcher::{match_embeddings, MatchResult};
use crate::pad_fusion::fuse_pad_results;
use crate::quality::{
    face_size_px, laplacian_variance, passes_min, FaceQualityRejection, DEFAULT_MIN_FACE_WIDTH_PX,
    DEFAULT_MIN_PAD_CROP_SHARPNESS,
};
use zeroize::{Zeroize, Zeroizing};

/// Maximum number of PAD models in a multi-scale ensemble, primary model included
/// (GitHub #212). Bounds the per-frame inference cost and crop allocations.
pub const MAX_PAD_ENSEMBLE_MODELS: usize = 4;

/// Minimum SCRFD detection confidence (GitHub #251, VIS-09).
///
/// Single value for authentication (`soos-daemon`), enrollment (`soos-enroll`) and the GUI
/// preview (`soos-gui`): it is both the detector's candidate threshold and the pipeline's
/// primary-face threshold.
pub const DEFAULT_MIN_FACE_CONFIDENCE: f32 = 0.70;

/// Default cosine similarity threshold; equals `soos_policy::ThresholdConfig::DEFAULT_MATCH_THRESHOLD`
/// (asserted by `crates/daemon/tests/vision_threshold_parity_tests.rs`).
pub const DEFAULT_MATCH_THRESHOLD: f32 = 0.70;

/// Default colour PAD liveness threshold (GitHub #215, PAD-10); equals
/// `soos_policy::ThresholdConfig::DEFAULT_PAD_THRESHOLD`. It is the only colour liveness
/// threshold of the workspace: every `OrtPadDetector` is built with the configured
/// `pad_threshold`, and the GUI displays [`VisionPipelineConfig::pad_passes`].
pub const DEFAULT_PAD_THRESHOLD: f32 = 0.85;

/// Default SCRFD non-maximum-suppression IoU threshold (GitHub #251, VIS-09).
///
/// The authentication value; a lower IoU would merge more overlapping boxes and could hide a
/// second face from the single-face invariant, so enrollment and the GUI use the same value.
pub const DEFAULT_NMS_IOU_THRESHOLD: f32 = 0.45;

/// Configuration options for the vision verification pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct VisionPipelineConfig {
    /// Minimum detection confidence required to accept a face candidate.
    pub min_face_confidence: f32,
    /// SCRFD non-maximum-suppression IoU threshold passed to `OrtScrfdDetector::new`.
    pub nms_iou_threshold: f32,
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
    /// Minimum face size in pixels (smaller bounding-box side) accepted before PAD
    /// (default [`DEFAULT_MIN_FACE_WIDTH_PX`], GitHub #218). Smaller faces are rejected
    /// with [`VisionError::FaceTooSmall`]; a non-finite value rejects every face.
    pub min_face_width_px: f32,
    /// Minimum PAD crop sharpness (variance of the Laplacian of the 80x80 crop luma)
    /// accepted before PAD (default [`DEFAULT_MIN_PAD_CROP_SHARPNESS`] = disabled,
    /// GitHub #218). Blurrier crops are rejected with [`VisionError::FaceBlurred`].
    pub min_pad_crop_sharpness: f32,
}

impl Default for VisionPipelineConfig {
    fn default() -> Self {
        Self {
            min_face_confidence: DEFAULT_MIN_FACE_CONFIDENCE,
            nms_iou_threshold: DEFAULT_NMS_IOU_THRESHOLD,
            match_threshold: DEFAULT_MATCH_THRESHOLD,
            pad_threshold: DEFAULT_PAD_THRESHOLD,
            ir_pad_threshold: DEFAULT_IR_PAD_THRESHOLD,
            target_width: 112,
            target_height: 112,
            pad_target_width: 80,
            pad_target_height: 80,
            pad_bbox_scale: 2.7,
            min_face_width_px: DEFAULT_MIN_FACE_WIDTH_PX,
            min_pad_crop_sharpness: DEFAULT_MIN_PAD_CROP_SHARPNESS,
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

    /// The single liveness decision of the workspace (GitHub #215, PAD-10).
    ///
    /// True only when the model result is live, the score is finite and
    /// `score >= effective_pad_threshold(modality)`. Written so that a NaN score or threshold
    /// always rejects. Used by the pipeline itself and by the GUI liveness display.
    pub fn pad_passes(&self, pad: &PadResult, modality: PadInputModality) -> bool {
        pad.is_live && pad.score.is_finite() && pad.score >= self.effective_pad_threshold(modality)
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
    /// Set when the primary face failed the pre-PAD quality gate (GitHub #218); PAD,
    /// alignment and embedding are then skipped (`pad_result` and `embedding` are `None`).
    pub quality_rejection: Option<FaceQualityRejection>,
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
    /// Additional `(bbox scale, detector)` PAD members fused with the primary model
    /// (GitHub #212). Empty by default: single-model behaviour.
    extra_pads: Vec<(f32, Arc<dyn PadDetector>)>,
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
            extra_pads: Vec::new(),
            extractor,
            config,
        }
    }

    /// Adds a PAD model run on its own context crop at `scale` and fused with the primary
    /// model (mean live probability, see [`fuse_pad_results`]; GitHub #212).
    ///
    /// Upstream Silent-Face-Anti-Spoofing pairs the 2.7-scale MiniFASNetV2 with a 4.0-scale
    /// MiniFASNetV1SE. Fails with [`VisionError::InvalidPadEnsemble`] when `scale` is not a
    /// finite positive number or the ensemble would exceed [`MAX_PAD_ENSEMBLE_MODELS`].
    pub fn with_additional_pad_model(
        mut self,
        scale: f32,
        detector: Arc<dyn PadDetector>,
    ) -> Result<Self, VisionError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(VisionError::InvalidPadEnsemble {
                reason: "PAD crop scale must be finite and positive",
            });
        }
        if self.extra_pads.len().saturating_add(1) >= MAX_PAD_ENSEMBLE_MODELS {
            return Err(VisionError::InvalidPadEnsemble {
                reason: "PAD ensemble exceeds MAX_PAD_ENSEMBLE_MODELS",
            });
        }
        self.extra_pads.push((scale, detector));
        Ok(self)
    }

    /// Context crop scales of every PAD member, primary (`config.pad_bbox_scale`) first.
    pub fn pad_scales(&self) -> Vec<f32> {
        std::iter::once(self.config.pad_bbox_scale)
            .chain(self.extra_pads.iter().map(|(scale, _)| *scale))
            .collect()
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
        let mut quality_rejection = None;

        if let Some(det) = primary {
            if let Some(landmarks) = &det.landmarks {
                pose = Some(crate::pose::estimate_head_pose(landmarks));
                quality_rejection = self.analyze_primary_face(
                    frame,
                    &rgb,
                    det,
                    landmarks,
                    &mut pad_result,
                    &mut aligned_crop,
                    &mut embedding,
                );
            }
        }

        Ok(VisionAnalysis {
            rgb,
            detections,
            pad_result,
            pose,
            aligned_crop,
            embedding,
            quality_rejection,
        })
    }

    /// Quality gate, PAD, alignment and embedding for the primary face of `analyze_frame`.
    ///
    /// Returns the quality-gate rejection, if any; a rejected face is never scored by PAD
    /// and never produces an aligned crop or an embedding (GitHub #218).
    #[allow(
        clippy::too_many_arguments,
        reason = "Out-parameters of the non-short-circuiting GUI analysis"
    )]
    fn analyze_primary_face(
        &self,
        frame: &Frame,
        rgb: &[u8],
        det: &FaceDetection,
        landmarks: &FaceLandmarks,
        pad_result: &mut Option<PadResult>,
        aligned_crop: &mut Option<Zeroizing<Vec<u8>>>,
        embedding: &mut Option<Zeroizing<Vec<f32>>>,
    ) -> Option<FaceQualityRejection> {
        if self.check_face_size(det).is_err() {
            return Some(FaceQualityRejection::TooSmall);
        }

        // The sharpness gate measures the primary PAD crop (upstream geometry, GitHub #213)
        // that the PAD model would score.
        if let Ok(crop) = crop_pad_context(
            rgb,
            frame.width,
            frame.height,
            &det.box_,
            self.config.pad_bbox_scale,
            self.config.pad_target_width,
            self.config.pad_target_height,
        ) {
            let pad_crop = Zeroizing::new(crop);
            if self.check_pad_crop_sharpness(&pad_crop).is_err() {
                return Some(FaceQualityRejection::Blurred);
            }
        }
        *pad_result = self.analyze_pad(
            rgb,
            frame.width,
            frame.height,
            &det.box_,
            PadInputModality::for_frame(frame),
        );

        if let Ok(aligned) = align_face_112(rgb, frame.width, frame.height, landmarks) {
            if let Ok(emb) = self.extractor.extract_embedding(
                &aligned,
                self.config.target_width,
                self.config.target_height,
            ) {
                *embedding = Some(Zeroizing::new(emb.as_slice().to_vec()));
            }
            *aligned_crop = Some(Zeroizing::new(aligned));
        }
        None
    }

    /// Pre-PAD face size gate (GitHub #218): the smaller bounding-box side must reach
    /// `min_face_width_px`. Fail-closed on non-finite sizes or thresholds.
    fn check_face_size(&self, detection: &FaceDetection) -> Result<(), VisionError> {
        let b = &detection.box_;
        let size = face_size_px(b.x1, b.y1, b.x2, b.y2);
        let min = self.config.min_face_width_px;
        if passes_min(size, min) {
            Ok(())
        } else {
            Err(VisionError::FaceTooSmall {
                width_px: size,
                min_width_px: min,
            })
        }
    }

    /// Pre-PAD sharpness gate (GitHub #218): the PAD crop's variance of the Laplacian must
    /// reach `min_pad_crop_sharpness`. Fail-closed on invalid buffers or non-finite values.
    fn check_pad_crop_sharpness(&self, pad_crop: &[u8]) -> Result<(), VisionError> {
        let sharpness = laplacian_variance(
            pad_crop,
            self.config.pad_target_width,
            self.config.pad_target_height,
        )
        .unwrap_or(f32::NAN);
        let min = self.config.min_pad_crop_sharpness;
        if passes_min(sharpness, min) {
            Ok(())
        } else {
            Err(VisionError::FaceBlurred {
                sharpness,
                min_sharpness: min,
            })
        }
    }

    /// Processes a single camera frame:
    /// 1. Color converts to RGB24
    /// 2. Detects faces; enforces single-face security invariant (rejects 0 or >1 faces)
    /// 3. Extracts 5-point facial landmarks from detection
    /// 4. Rejects faces smaller than `min_face_width_px` ([`VisionError::FaceTooSmall`]),
    ///    crops the upstream-geometry 2.7x context window (plus any additional multi-scale
    ///    PAD member scales), resizes it to 80x80 for PAD and rejects a primary crop below
    ///    `min_pad_crop_sharpness` ([`VisionError::FaceBlurred`]) (GitHub #218)
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

        // Pre-PAD quality gate (GitHub #218): tiny faces give unreliable PAD scores.
        self.check_face_size(&detection)?;

        let landmarks = detection.landmarks.ok_or(VisionError::MissingLandmarks)?;

        // Step 4: Presentation Attack Detection on upstream-geometry context crops (2.7x
        // primary, plus any additional multi-scale members), fused and thresholded.
        let pad_result = self.evaluate_pad(
            &rgb,
            frame.width,
            frame.height,
            &detection.box_,
            PadInputModality::for_frame(frame),
        )?;

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

    /// Scores every PAD member on its own context crop and fuses the results.
    ///
    /// Each member gets [`crop_pad_context`] at its scale (upstream geometry, GitHub #213).
    /// Monochrome frames: the primary crop must first pass the fail-closed IR gate, otherwise
    /// no PAD model is consulted. Any crop or inference error fails the whole evaluation.
    fn fused_pad_result(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
        face_box: &soos_inference_ort::BoundingBox,
        modality: PadInputModality,
        threshold: f32,
    ) -> Result<PadResult, VisionError> {
        let target_w = self.config.pad_target_width;
        let target_h = self.config.pad_target_height;
        let members = std::iter::once((self.config.pad_bbox_scale, &self.pad))
            .chain(self.extra_pads.iter().map(|(scale, pad)| (*scale, pad)));

        let mut results = Vec::with_capacity(self.extra_pads.len().saturating_add(1));
        for (index, (scale, pad)) in members.enumerate() {
            let crop = Zeroizing::new(crop_pad_context(
                rgb, width, height, face_box, scale, target_w, target_h,
            )?);
            if index == 0 {
                // Pre-PAD sharpness gate on the primary crop (GitHub #218), before the IR
                // gate and before any PAD model is consulted.
                self.check_pad_crop_sharpness(&crop)?;
                if modality.is_monochrome() {
                    evaluate_ir_gate(&crop, target_w, target_h)
                        .map_err(|reason| VisionError::IrLivenessGateFailed { reason })?;
                }
            }
            results.push(pad.evaluate_liveness(&crop, target_w, target_h)?);
        }

        fuse_pad_results(&results, threshold).ok_or(VisionError::InvalidPadEnsemble {
            reason: "PAD ensemble has no model",
        })
    }

    /// Modality-aware PAD decision (modality from the frame's pixel format and sensor type,
    /// see [`PadInputModality::for_frame`]).
    ///
    /// Colour frames: the fused result must be live and `score >= pad_threshold`.
    /// Monochrome frames: the IR gate must pass (otherwise the model is not consulted), then
    /// the fused result must be live and reach the effective IR threshold. Every comparison
    /// rejects non-finite scores or thresholds.
    fn evaluate_pad(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
        face_box: &soos_inference_ort::BoundingBox,
        modality: PadInputModality,
    ) -> Result<PadResult, VisionError> {
        let threshold = self.config.effective_pad_threshold(modality);
        let pad_result =
            self.fused_pad_result(rgb, width, height, face_box, modality, threshold)?;

        // Single fail-closed liveness decision (NaN score or threshold always rejects).
        if !self.config.pad_passes(&pad_result, modality) {
            return Err(VisionError::PadFailed {
                score: pad_result.score,
                threshold,
            });
        }
        Ok(pad_result)
    }

    /// Non-short-circuiting PAD evaluation for GUI analysis.
    ///
    /// Colour frames report the (fused) model result. Monochrome frames report a spoof result
    /// (`is_live = false`) when the IR gate rejects or the score is below the IR threshold,
    /// so the GUI never shows an IR capture as live when the daemon would reject it.
    fn analyze_pad(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
        face_box: &soos_inference_ort::BoundingBox,
        modality: PadInputModality,
    ) -> Option<PadResult> {
        if !modality.is_monochrome() {
            return self
                .fused_pad_result(
                    rgb,
                    width,
                    height,
                    face_box,
                    modality,
                    self.config.pad_threshold,
                )
                .ok();
        }
        match self.evaluate_pad(rgb, width, height, face_box, modality) {
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

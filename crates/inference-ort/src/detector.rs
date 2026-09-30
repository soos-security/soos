//! Face detection domain structures, deterministic Rust NMS, and detector abstractions.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Geometric coordinate, IoU, stride grid, and pixel tensor calculations"
)]

use crate::error::InferenceError;
use crate::landmarks::{FaceLandmarks, Point2f};
use std::cmp::Ordering;
use zeroize::{Zeroize, Zeroizing};

/// Axis-aligned 2D bounding box in pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl BoundingBox {
    /// Creates a new bounding box ensuring `x1 <= x2` and `y1 <= y2`.
    pub fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        let min_x = x1.min(x2);
        let max_x = x1.max(x2);
        let min_y = y1.min(y2);
        let max_y = y1.max(y2);
        Self {
            x1: min_x,
            y1: min_y,
            x2: max_x,
            y2: max_y,
        }
    }

    /// Returns the box width.
    pub fn width(&self) -> f32 {
        (self.x2 - self.x1).max(0.0)
    }

    /// Returns the box height.
    pub fn height(&self) -> f32 {
        (self.y2 - self.y1).max(0.0)
    }

    /// Returns the box area.
    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }

    /// Computes the intersection area with another bounding box.
    pub fn intersection(&self, other: &BoundingBox) -> f32 {
        let inter_x1 = self.x1.max(other.x1);
        let inter_y1 = self.y1.max(other.y1);
        let inter_x2 = self.x2.min(other.x2);
        let inter_y2 = self.y2.min(other.y2);

        let w = (inter_x2 - inter_x1).max(0.0);
        let h = (inter_y2 - inter_y1).max(0.0);
        w * h
    }

    /// Computes the Intersection over Union (IoU) metric in range `[0.0, 1.0]`.
    pub fn iou(&self, other: &BoundingBox) -> f32 {
        let inter = self.intersection(other);
        let union = self.area() + other.area() - inter;
        if union <= 0.0 {
            0.0
        } else {
            (inter / union).clamp(0.0, 1.0)
        }
    }

    /// Clamps box coordinates within image bounds `[0, max_w]` and `[0, max_h]`.
    pub fn clamp(&self, max_w: f32, max_h: f32) -> Self {
        Self {
            x1: self.x1.clamp(0.0, max_w),
            y1: self.y1.clamp(0.0, max_h),
            x2: self.x2.clamp(0.0, max_w),
            y2: self.y2.clamp(0.0, max_h),
        }
    }
}

/// A detected face candidate with bounding box, confidence score, and optional landmarks.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceDetection {
    pub box_: BoundingBox,
    pub score: f32,
    pub landmarks: Option<FaceLandmarks>,
}

impl FaceDetection {
    /// Creates a detection without landmarks.
    pub fn new(box_: BoundingBox, score: f32) -> Self {
        Self {
            box_,
            score,
            landmarks: None,
        }
    }

    /// Creates a detection with associated 5-point facial landmarks.
    pub fn with_landmarks(box_: BoundingBox, score: f32, landmarks: FaceLandmarks) -> Self {
        Self {
            box_,
            score,
            landmarks: Some(landmarks),
        }
    }
}

/// Deterministic Rust Non-Maximum Suppression (NMS).
///
/// Filters overlapping candidate boxes based on IoU threshold.
/// Sorts candidates by descending confidence score. If scores are equal,
/// tie-breaking is strictly deterministic based on coordinate values.
pub fn nms(candidates: &[FaceDetection], iou_threshold: f32) -> Vec<FaceDetection> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut indexed: Vec<(usize, &FaceDetection)> = candidates.iter().enumerate().collect();

    // Sort descending by score, deterministic secondary sort on coordinates
    indexed.sort_by(|(_, a), (_, b)| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.box_.x1.partial_cmp(&b.box_.x1).unwrap_or(Ordering::Equal))
            .then_with(|| a.box_.y1.partial_cmp(&b.box_.y1).unwrap_or(Ordering::Equal))
            .then_with(|| a.box_.x2.partial_cmp(&b.box_.x2).unwrap_or(Ordering::Equal))
            .then_with(|| a.box_.y2.partial_cmp(&b.box_.y2).unwrap_or(Ordering::Equal))
    });

    let mut suppressed = vec![false; candidates.len()];
    let mut results = Vec::new();

    for (orig_idx, cand) in indexed {
        if let Some(&true) = suppressed.get(orig_idx) {
            continue;
        }
        results.push(cand.clone());

        for (other_idx, other_cand) in candidates.iter().enumerate() {
            if let Some(&false) = suppressed.get(other_idx) {
                if cand.box_.iou(&other_cand.box_) > iou_threshold {
                    if let Some(s) = suppressed.get_mut(other_idx) {
                        *s = true;
                    }
                }
            }
        }
    }

    results
}

/// Trait implemented by face detector backends.
pub trait FaceDetector: Send + Sync {
    /// Detects faces in an RGB image buffer (packed RGB888, 3 bytes per pixel).
    fn detect(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<FaceDetection>, InferenceError>;
}

use ort::session::Session;
use std::sync::{Arc, Mutex};

/// Legacy UltraFace Slim 320 input preprocessing (not a detector).
///
/// The UltraFace inference path (session, anchor priors, box decoding) was removed with the
/// 3-model pipeline (GitHub #249, VIS-07); no production code constructs this type. Only the
/// preprocessing helper remains, because the acceptance test
/// `zeroize_tests::test_inference_input_buffers_zeroized` (matrix NGM8) still exercises it.
/// Its removal is pending re-pointing that test to the SCRFD input path.
pub struct OrtFaceDetector;

impl OrtFaceDetector {
    /// Prepares, resizes, and normalizes an RGB frame to 320x240 NCHW format inside a zeroized container.
    pub fn prepare_input(
        rgb: &[u8],
        width: u32,
        height: u32,
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

        // Resize and normalize RGB buffer to 320x240 NCHW [1, 3, 240, 320]
        let target_w = 320usize;
        let target_h = 240usize;
        let mut input_data = Zeroizing::new(vec![0.0f32; 3 * target_h * target_w]);

        let scale_x = width as f32 / target_w as f32;
        let scale_y = height as f32 / target_h as f32;

        for y in 0..target_h {
            let src_y = ((y as f32 * scale_y) as usize).min(height as usize - 1);
            for x in 0..target_w {
                let src_x = ((x as f32 * scale_x) as usize).min(width as usize - 1);
                let src_idx = (src_y * width as usize + src_x) * 3;

                if let (Some(&r), Some(&g), Some(&b)) =
                    (rgb.get(src_idx), rgb.get(src_idx + 1), rgb.get(src_idx + 2))
                {
                    let norm_r = (r as f32 - 127.0) / 128.0;
                    let norm_g = (g as f32 - 127.0) / 128.0;
                    let norm_b = (b as f32 - 127.0) / 128.0;

                    let r_idx = y * target_w + x;
                    let g_idx = target_h * target_w + y * target_w + x;
                    let b_idx = 2 * target_h * target_w + y * target_w + x;

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

/// Unprojects coordinates from letterbox space back to original image space.
pub fn unproject(x: f32, y: f32, scale: f32, pad_x: f32, pad_y: f32) -> (f32, f32) {
    if scale <= 0.0 {
        return (0.0, 0.0);
    }
    let orig_x = (x - pad_x) / scale;
    let orig_y = (y - pad_y) / scale;
    (orig_x, orig_y)
}

/// Letterbox pads an RGB image buffer to target x target dimensions, maintaining aspect ratio.
///
/// Output tensor is in NCHW format with BGR channel ordering and (pixel - 127.5) / 128.0 normalization.
/// Border padding is filled with 0.0.
pub fn letterbox_pad(
    rgb: &[u8],
    w: u32,
    h: u32,
    target: usize,
) -> (Zeroizing<Vec<f32>>, f32, f32, f32) {
    let target_f = target as f32;
    let mut tensor = Zeroizing::new(vec![0.0f32; 3 * target * target]);

    if w == 0 || h == 0 || rgb.len() != (w as usize).saturating_mul(h as usize).saturating_mul(3) {
        return (tensor, 1.0, 0.0, 0.0);
    }

    let scale = (target_f / w as f32).min(target_f / h as f32);
    let new_w = ((w as f32 * scale).round() as usize).min(target);
    let new_h = ((h as f32 * scale).round() as usize).min(target);
    let pad_x = ((target_f - new_w as f32) / 2.0).max(0.0);
    let pad_y = ((target_f - new_h as f32) / 2.0).max(0.0);

    let pad_x_int = pad_x as usize;
    let pad_y_int = pad_y as usize;

    let b_offset = 0;
    let g_offset = target * target;
    let r_offset = 2 * target * target;

    for y in 0..new_h {
        let src_y = ((y as f32 / scale).floor() as usize).min((h - 1) as usize);
        let dst_y = y + pad_y_int;
        if dst_y >= target {
            continue;
        }

        for x in 0..new_w {
            let src_x = ((x as f32 / scale).floor() as usize).min((w - 1) as usize);
            let dst_x = x + pad_x_int;
            if dst_x >= target {
                continue;
            }

            let src_idx = (src_y * w as usize + src_x) * 3;
            if let (Some(&r_val), Some(&g_val), Some(&b_val)) =
                (rgb.get(src_idx), rgb.get(src_idx + 1), rgb.get(src_idx + 2))
            {
                let norm_b = (b_val as f32 - 127.5) / 128.0;
                let norm_g = (g_val as f32 - 127.5) / 128.0;
                let norm_r = (r_val as f32 - 127.5) / 128.0;

                let dst_idx = dst_y * target + dst_x;
                if let Some(slot) = tensor.get_mut(b_offset + dst_idx) {
                    *slot = norm_b;
                }
                if let Some(slot) = tensor.get_mut(g_offset + dst_idx) {
                    *slot = norm_g;
                }
                if let Some(slot) = tensor.get_mut(r_offset + dst_idx) {
                    *slot = norm_r;
                }
            }
        }
    }

    (tensor, scale, pad_x, pad_y)
}

/// How raw SCRFD score-head values map to face confidences (GitHub #247, VIS-05).
///
/// The activation is a property of the model, decided once for the detector, never per
/// element: a per-element "pass through if in `[0, 1]`, else sigmoid" rule is non-monotonic
/// across the boundary and misranks candidates before NMS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScoreActivation {
    /// Scores are already probabilities (the attested `scrfd_500m_kps` graph ends in
    /// `Sigmoid`). Any non-finite value or value outside `[0, 1]` fails closed.
    #[default]
    Probability,
    /// Scores are logits; the sigmoid is applied to every value. Non-finite values fail closed.
    Logit,
}

impl ScoreActivation {
    /// Infers one activation for a whole score tensor: `Probability` when every value is a
    /// finite number in `[0, 1]`, `Logit` otherwise.
    ///
    /// Only the historical [`OrtScrfdDetector::decode_stride`] entry point uses this; the
    /// production detector uses its configured activation and fails closed instead.
    pub fn infer(scores: &[f32]) -> Self {
        if scores.iter().all(|s| (0.0..=1.0).contains(s)) {
            Self::Probability
        } else {
            Self::Logit
        }
    }

    /// Checks that every raw score is admissible for this activation.
    pub fn validate(self, scores: &[f32]) -> Result<(), InferenceError> {
        let bad = match self {
            Self::Probability => scores.iter().position(|s| !(0.0..=1.0).contains(s)),
            Self::Logit => scores.iter().position(|s| !s.is_finite()),
        };
        match bad {
            None => Ok(()),
            Some(index) => Err(InferenceError::TensorError(format!(
                "SCRFD score {index} is not a valid {self:?} value"
            ))),
        }
    }

    /// Maps one raw score to a confidence (monotonic non-decreasing).
    pub fn apply(self, raw: f32) -> f32 {
        match self {
            Self::Probability => raw,
            Self::Logit => 1.0 / (1.0 + (-raw).exp()),
        }
    }
}

/// SCRFD 500M KPS ONNX Runtime face detector with multi-stride output parsing and 5-point landmarks.
pub struct OrtScrfdDetector {
    session: Arc<Mutex<Session>>,
    pub conf_threshold: f32,
    pub iou_threshold: f32,
    pub input_size: (usize, usize),
    pub strides: [usize; 3],
    pub anchors_per_cell: usize,
    /// Activation of the score heads, fixed at construction (default `Probability`).
    pub score_activation: ScoreActivation,
}

impl OrtScrfdDetector {
    /// Validates that session output count is exactly 9.
    pub fn validate_output_count(count: usize) -> Result<(), InferenceError> {
        if count != 9 {
            return Err(InferenceError::TensorError(format!(
                "SCRFD model must have exactly 9 output tensors, found {count}"
            )));
        }
        Ok(())
    }

    /// Validates output shape patterns across the 3 strides.
    pub fn validate_output_shapes(shapes: &[Vec<usize>]) -> Result<(), InferenceError> {
        Self::validate_output_count(shapes.len())?;
        let strides = [8, 16, 32];
        for &s in &strides {
            let num_anchors = (640 / s) * (640 / s) * 2;
            let has_score = shapes.iter().any(|sh| {
                sh.len() == 3
                    && sh.first() == Some(&1)
                    && sh.get(1) == Some(&num_anchors)
                    && sh.get(2) == Some(&1)
            });
            let has_bbox = shapes.iter().any(|sh| {
                sh.len() == 3
                    && sh.first() == Some(&1)
                    && sh.get(1) == Some(&num_anchors)
                    && sh.get(2) == Some(&4)
            });
            let has_kps = shapes.iter().any(|sh| {
                sh.len() == 3
                    && sh.first() == Some(&1)
                    && sh.get(1) == Some(&num_anchors)
                    && sh.get(2) == Some(&10)
            });

            if !has_score || !has_bbox || !has_kps {
                return Err(InferenceError::TensorError(format!(
                    "SCRFD output shapes missing expected patterns for stride {s} (num_anchors={num_anchors})"
                )));
            }
        }
        Ok(())
    }

    /// Validates session output metadata dims (negative = symbolic) against the SCRFD layout.
    ///
    /// The attested graph reports `[-1, -1, k]` (symbolic batch and anchor dims), so the
    /// metadata cannot prove per-stride anchor counts; those are enforced on every frame by
    /// [`FaceDetector::detect`]. What the metadata can prove is checked here, failing closed:
    /// - exactly 9 outputs, each of rank 3;
    /// - batch dim symbolic or 1;
    /// - channel dim concrete, with exactly 3 score (1), 3 bbox (4) and 3 keypoint (10) heads;
    /// - within each head kind, every concrete anchor dim is one of the stride anchor counts
    ///   (12800, 3200, 800 for a 640x640 input) and no count appears twice.
    pub fn validate_output_dims(dims: &[Vec<i64>]) -> Result<(), InferenceError> {
        Self::validate_output_count(dims.len())?;
        let reject = |shape: &Vec<i64>, why: &str| {
            InferenceError::TensorError(format!("SCRFD output shape {shape:?}: {why}"))
        };
        let expected_anchors: Vec<i64> = [8i64, 16, 32]
            .iter()
            .map(|&s| (640 / s) * (640 / s) * 2)
            .collect();
        for channels in [1i64, 4, 10] {
            let mut count = 0usize;
            let mut seen: Vec<i64> = Vec::with_capacity(3);
            for shape in dims {
                let &[batch, anchors, ch] = shape.as_slice() else {
                    return Err(reject(shape, "expected rank 3 [N, anchors, channels]"));
                };
                if ch < 0 {
                    return Err(reject(shape, "symbolic channel dim"));
                }
                if batch >= 0 && batch != 1 {
                    return Err(reject(shape, "batch dim must be 1 or symbolic"));
                }
                if ch != channels {
                    continue;
                }
                count += 1;
                if anchors >= 0 {
                    if !expected_anchors.contains(&anchors) {
                        return Err(reject(shape, "anchor count matches no stride"));
                    }
                    if seen.contains(&anchors) {
                        return Err(reject(shape, "duplicate anchor count for this head kind"));
                    }
                    seen.push(anchors);
                }
            }
            if count != 3 {
                return Err(InferenceError::TensorError(format!(
                    "SCRFD model must expose 3 output heads with {channels} channel(s), found {count}"
                )));
            }
        }
        Ok(())
    }

    /// Constructs an SCRFD detector wrapping an active ORT session.
    ///
    /// Startup validation: exactly 9 outputs whose metadata shapes match the score/bbox/kps
    /// pattern of every stride ([`Self::validate_output_dims`]). The score activation defaults
    /// to [`ScoreActivation::Probability`] (see [`Self::with_score_activation`]).
    pub fn new(
        session: Arc<Mutex<Session>>,
        conf_threshold: f32,
        iou_threshold: f32,
    ) -> Result<Self, InferenceError> {
        {
            let s = session.lock().map_err(|_| {
                InferenceError::DetectionFailed("Session mutex poisoned".to_string())
            })?;
            Self::validate_output_count(s.outputs().len())?;
            let dims = s
                .outputs()
                .iter()
                .map(|output| {
                    output
                        .dtype()
                        .tensor_shape()
                        .map(|shape| shape.to_vec())
                        .ok_or_else(|| {
                            InferenceError::TensorError(format!(
                                "SCRFD output '{}' is not a dense tensor",
                                output.name()
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Self::validate_output_dims(&dims)?;
        }

        Ok(Self {
            session,
            conf_threshold,
            iou_threshold,
            input_size: (640, 640),
            strides: [8, 16, 32],
            anchors_per_cell: 2,
            score_activation: ScoreActivation::default(),
        })
    }

    /// Returns the detector with an explicit score activation (for a logit-output variant).
    pub fn with_score_activation(mut self, activation: ScoreActivation) -> Self {
        self.score_activation = activation;
        self
    }

    /// Prepares, letterbox-pads, and normalizes an RGB frame to 640x640 NCHW BGR format.
    pub fn prepare_input(
        rgb: &[u8],
        width: u32,
        height: u32,
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

        let (tensor, _scale, _pad_x, _pad_y) = letterbox_pad(rgb, width, height, 640);
        Ok(tensor)
    }

    /// Decodes grid coordinates, distance-to-border boxes, and landmarks for a specific stride.
    ///
    /// Historical entry point: the activation is inferred once for the whole score tensor
    /// ([`ScoreActivation::infer`]), which keeps the mapping monotonic. Production decoding
    /// uses [`Self::decode_stride_checked`] with the detector's configured activation.
    #[allow(
        clippy::too_many_arguments,
        reason = "Stride decoding requires explicit tensor slices, coordinates, grid geometry, and threshold parameters"
    )]
    pub fn decode_stride(
        stride: usize,
        grid_w: usize,
        grid_h: usize,
        anchors_per_cell: usize,
        scores: &[f32],
        bboxes: &[f32],
        kps: &[f32],
        conf_threshold: f32,
        scale: f32,
        pad_x: f32,
        pad_y: f32,
        orig_w: u32,
        orig_h: u32,
    ) -> Vec<FaceDetection> {
        Self::decode_stride_with(
            stride,
            grid_w,
            grid_h,
            anchors_per_cell,
            scores,
            bboxes,
            kps,
            conf_threshold,
            (scale, pad_x, pad_y),
            (orig_w, orig_h),
            ScoreActivation::infer(scores),
        )
    }

    /// Fail-closed stride decoding with an explicit score activation (production path).
    ///
    /// Every raw score is validated first ([`ScoreActivation::validate`]); one non-finite or
    /// out-of-range value rejects the whole frame with `TensorError`.
    #[allow(
        clippy::too_many_arguments,
        reason = "Stride decoding requires explicit tensor slices, coordinates, grid geometry, threshold and activation parameters"
    )]
    pub fn decode_stride_checked(
        stride: usize,
        grid_w: usize,
        grid_h: usize,
        anchors_per_cell: usize,
        scores: &[f32],
        bboxes: &[f32],
        kps: &[f32],
        conf_threshold: f32,
        scale: f32,
        pad_x: f32,
        pad_y: f32,
        orig_w: u32,
        orig_h: u32,
        activation: ScoreActivation,
    ) -> Result<Vec<FaceDetection>, InferenceError> {
        activation.validate(scores)?;
        Ok(Self::decode_stride_with(
            stride,
            grid_w,
            grid_h,
            anchors_per_cell,
            scores,
            bboxes,
            kps,
            conf_threshold,
            (scale, pad_x, pad_y),
            (orig_w, orig_h),
            activation,
        ))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Stride decoding requires explicit tensor slices, coordinates, grid geometry, and threshold parameters"
    )]
    fn decode_stride_with(
        stride: usize,
        grid_w: usize,
        grid_h: usize,
        anchors_per_cell: usize,
        scores: &[f32],
        bboxes: &[f32],
        kps: &[f32],
        conf_threshold: f32,
        (scale, pad_x, pad_y): (f32, f32, f32),
        (orig_w, orig_h): (u32, u32),
        activation: ScoreActivation,
    ) -> Vec<FaceDetection> {
        let mut detections = Vec::new();
        let s = stride as f32;

        for row in 0..grid_h {
            for col in 0..grid_w {
                for anchor in 0..anchors_per_cell {
                    let idx = (row * grid_w + col) * anchors_per_cell + anchor;
                    let raw_score = match scores.get(idx) {
                        Some(&val) => val,
                        None => continue,
                    };
                    let conf = activation.apply(raw_score);
                    if conf > conf_threshold {
                        let bbox_offset = idx * 4;
                        let (l, t, r, b) = match (
                            bboxes.get(bbox_offset),
                            bboxes.get(bbox_offset + 1),
                            bboxes.get(bbox_offset + 2),
                            bboxes.get(bbox_offset + 3),
                        ) {
                            (Some(&l), Some(&t), Some(&r), Some(&b)) => (l, t, r, b),
                            _ => continue,
                        };

                        let x1_let = (col as f32 - l) * s;
                        let y1_let = (row as f32 - t) * s;
                        let x2_let = (col as f32 + r) * s;
                        let y2_let = (row as f32 + b) * s;

                        let (orig_x1, orig_y1) = unproject(x1_let, y1_let, scale, pad_x, pad_y);
                        let (orig_x2, orig_y2) = unproject(x2_let, y2_let, scale, pad_x, pad_y);

                        let bbox = BoundingBox::new(orig_x1, orig_y1, orig_x2, orig_y2)
                            .clamp(orig_w as f32, orig_h as f32);

                        let kps_offset = idx * 10;
                        let mut lm_pts = [Point2f::new(0.0, 0.0); 5];
                        let mut kps_valid = true;
                        for (i, pt) in lm_pts.iter_mut().enumerate() {
                            let (kx, ky) = match (
                                kps.get(kps_offset + i * 2),
                                kps.get(kps_offset + i * 2 + 1),
                            ) {
                                (Some(&kx), Some(&ky)) => (kx, ky),
                                _ => {
                                    kps_valid = false;
                                    break;
                                }
                            };
                            let lm_x_let = (col as f32 + kx) * s;
                            let lm_y_let = (row as f32 + ky) * s;
                            let (orig_lm_x, orig_lm_y) =
                                unproject(lm_x_let, lm_y_let, scale, pad_x, pad_y);
                            *pt = Point2f::new(
                                orig_lm_x.clamp(0.0, orig_w as f32),
                                orig_lm_y.clamp(0.0, orig_h as f32),
                            );
                        }

                        let landmarks = if kps_valid {
                            Some(FaceLandmarks::new(
                                lm_pts[0], lm_pts[1], lm_pts[2], lm_pts[3], lm_pts[4],
                            ))
                        } else {
                            None
                        };

                        let det = if let Some(lm) = landmarks {
                            FaceDetection::with_landmarks(bbox, conf, lm)
                        } else {
                            FaceDetection::new(bbox, conf)
                        };
                        detections.push(det);
                    }
                }
            }
        }

        detections
    }
}

impl FaceDetector for OrtScrfdDetector {
    fn detect(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<FaceDetection>, InferenceError> {
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

        let (mut input_data, scale, pad_x, pad_y) =
            letterbox_pad(rgb, width, height, self.input_size.0);

        let tensor = ort::value::TensorRef::from_array_view((
            [1usize, 3, self.input_size.1, self.input_size.0],
            input_data.as_slice(),
        ))
        .map_err(|e| InferenceError::Ort(e.to_string()))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| InferenceError::DetectionFailed("Session mutex poisoned".to_string()))?;

        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|e| InferenceError::Ort(e.to_string()))?;

        // Zeroize input buffer immediately post-inference
        input_data.zeroize();

        // Extract raw tensor views from outputs
        let mut extracted = Vec::with_capacity(9);
        for (_name, val) in outputs {
            let (shape, slice) = val
                .try_extract_tensor::<f32>()
                .map_err(|e| InferenceError::Ort(e.to_string()))?;
            let shape_vec: Vec<usize> = shape.as_ref().iter().map(|&d| d as usize).collect();
            extracted.push((shape_vec, slice.to_vec()));
        }

        if extracted.len() != 9 {
            return Err(InferenceError::TensorError(format!(
                "SCRFD output count mismatch: expected 9, got {}",
                extracted.len()
            )));
        }

        let mut candidates = Vec::new();

        for &stride in &self.strides {
            let gw = self.input_size.0 / stride;
            let gh = self.input_size.1 / stride;
            let num_anchors = gw * gh * self.anchors_per_cell;

            let score_data = extracted
                .iter()
                .find(|(sh, sl)| {
                    sh.len() == 3
                        && sh.first() == Some(&1)
                        && sh.get(1) == Some(&num_anchors)
                        && sh.get(2) == Some(&1)
                        && sl.len() == num_anchors
                })
                .map(|(_, sl)| sl.as_slice())
                .ok_or_else(|| {
                    InferenceError::TensorError(format!(
                        "Missing score tensor for stride {stride} (anchors={num_anchors})"
                    ))
                })?;

            let bbox_data = extracted
                .iter()
                .find(|(sh, sl)| {
                    sh.len() == 3
                        && sh.first() == Some(&1)
                        && sh.get(1) == Some(&num_anchors)
                        && sh.get(2) == Some(&4)
                        && sl.len() == num_anchors * 4
                })
                .map(|(_, sl)| sl.as_slice())
                .ok_or_else(|| {
                    InferenceError::TensorError(format!(
                        "Missing bbox tensor for stride {stride} (anchors={num_anchors})"
                    ))
                })?;

            let kps_data = extracted
                .iter()
                .find(|(sh, sl)| {
                    sh.len() == 3
                        && sh.first() == Some(&1)
                        && sh.get(1) == Some(&num_anchors)
                        && sh.get(2) == Some(&10)
                        && sl.len() == num_anchors * 10
                })
                .map(|(_, sl)| sl.as_slice())
                .ok_or_else(|| {
                    InferenceError::TensorError(format!(
                        "Missing kps tensor for stride {stride} (anchors={num_anchors})"
                    ))
                })?;

            let detections = Self::decode_stride_checked(
                stride,
                gw,
                gh,
                self.anchors_per_cell,
                score_data,
                bbox_data,
                kps_data,
                self.conf_threshold,
                scale,
                pad_x,
                pad_y,
                width,
                height,
                self.score_activation,
            )?;
            candidates.extend(detections);
        }

        let filtered = nms(&candidates, self.iou_threshold);
        Ok(filtered)
    }
}

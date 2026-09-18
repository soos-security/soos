//! Face detection domain structures, deterministic Rust NMS, and detector abstractions.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Geometric coordinate, IoU, UltraFace anchor prior, and pixel tensor calculations"
)]

use crate::error::InferenceError;
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

/// A detected face candidate with bounding box and confidence score in `[0.0, 1.0]`.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceDetection {
    pub box_: BoundingBox,
    pub score: f32,
}

impl FaceDetection {
    pub fn new(box_: BoundingBox, score: f32) -> Self {
        Self { box_, score }
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

/// UltraFace Slim 320 ONNX Runtime face detector.
pub struct OrtFaceDetector {
    session: Arc<Mutex<Session>>,
    conf_threshold: f32,
    iou_threshold: f32,
    priors: Vec<[f32; 4]>,
}

impl OrtFaceDetector {
    /// Constructs an UltraFace detector wrapping an active ORT session.
    pub fn new(session: Arc<Mutex<Session>>, conf_threshold: f32, iou_threshold: f32) -> Self {
        let priors = Self::generate_priors();
        Self {
            session,
            conf_threshold,
            iou_threshold,
            priors,
        }
    }

    /// Generates canonical UltraFace 320x240 anchor priors (4,420 anchor boxes).
    pub fn generate_priors() -> Vec<[f32; 4]> {
        let feature_map_sizes = [(30, 40), (15, 20), (8, 10), (4, 5)];
        let min_sizes = [
            vec![10.0f32, 16.0, 24.0],
            vec![32.0f32, 48.0],
            vec![64.0f32, 96.0],
            vec![128.0f32, 192.0, 256.0],
        ];
        let steps = [8.0f32, 16.0, 32.0, 64.0];

        let mut priors = Vec::with_capacity(4420);

        for (k, &(h, w)) in feature_map_sizes.iter().enumerate() {
            let step = steps.get(k).copied().unwrap_or(8.0);
            let sizes = min_sizes.get(k).cloned().unwrap_or_default();

            for i in 0..h {
                for j in 0..w {
                    for &min_size in &sizes {
                        let s_kx = min_size / 320.0;
                        let s_ky = min_size / 240.0;
                        let cx = ((j as f32) + 0.5) * step / 320.0;
                        let cy = ((i as f32) + 0.5) * step / 240.0;
                        priors.push([cx, cy, s_kx, s_ky]);
                    }
                }
            }
        }

        priors
    }

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

impl FaceDetector for OrtFaceDetector {
    fn detect(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<FaceDetection>, InferenceError> {
        let mut input_data = Self::prepare_input(rgb, width, height)?;

        let tensor =
            ort::value::TensorRef::from_array_view(([1usize, 3, 240, 320], input_data.as_slice()))
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

        let mut out_iter = outputs.into_iter();
        let (_, conf_tensor) = out_iter.next().ok_or_else(|| {
            InferenceError::DetectionFailed("UltraFace confidence tensor missing".to_string())
        })?;
        let (_, loc_tensor) = out_iter.next().ok_or_else(|| {
            InferenceError::DetectionFailed("UltraFace box regression tensor missing".to_string())
        })?;

        let conf_data = conf_tensor
            .try_extract_tensor::<f32>()
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .1;
        let loc_data = loc_tensor
            .try_extract_tensor::<f32>()
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .1;

        let mut candidates = Vec::new();

        for (idx, prior) in self.priors.iter().enumerate() {
            let conf_offset = idx * 2;
            let loc_offset = idx * 4;

            if let (Some(&_bg_conf), Some(&face_conf)) =
                (conf_data.get(conf_offset), conf_data.get(conf_offset + 1))
            {
                if face_conf > self.conf_threshold {
                    if let (Some(&loc_cx), Some(&loc_cy), Some(&loc_w), Some(&loc_h)) = (
                        loc_data.get(loc_offset),
                        loc_data.get(loc_offset + 1),
                        loc_data.get(loc_offset + 2),
                        loc_data.get(loc_offset + 3),
                    ) {
                        // Decode center, size from priors
                        let center_x = prior[0] + loc_cx * 0.1 * prior[2];
                        let center_y = prior[1] + loc_cy * 0.1 * prior[3];
                        let w = prior[2] * (loc_w * 0.2).exp();
                        let h = prior[3] * (loc_h * 0.2).exp();

                        let x1 = (center_x - w / 2.0) * width as f32;
                        let y1 = (center_y - h / 2.0) * height as f32;
                        let x2 = (center_x + w / 2.0) * width as f32;
                        let y2 = (center_y + h / 2.0) * height as f32;

                        let bbox =
                            BoundingBox::new(x1, y1, x2, y2).clamp(width as f32, height as f32);
                        candidates.push(FaceDetection::new(bbox, face_conf));
                    }
                }
            }
        }

        let filtered = nms(&candidates, self.iou_threshold);
        Ok(filtered)
    }
}

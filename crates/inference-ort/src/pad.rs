//! Presentation Attack Detection (PAD / anti-spoofing) domain structures,
//! inference engine abstraction, and ONNX Runtime implementation.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Image tensor mapping, affine sampling, and softmax probability normalization"
)]

use std::sync::{Arc, Mutex};

use ort::session::Session;

use crate::error::InferenceError;
use zeroize::{Zeroize, Zeroizing};

/// Classification of detected presentation attack vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackType {
    /// 2D printed photograph attack on paper, cardboard, or glossy photo paper.
    PrintPhoto,
    /// 2D digital screen replay attack on smartphone, tablet, monitor, or laptop.
    ScreenReplay,
    /// Unclassified or multi-modal presentation attack.
    UnknownSpoof,
}

/// Result of a Presentation Attack Detection (PAD) evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct PadResult {
    /// True if the face candidate is classified as a genuine/live presentation.
    pub is_live: bool,
    /// Liveness confidence score normalized to `[0.0, 1.0]`.
    pub score: f32,
    /// Specific attack type if classified as a spoof presentation.
    pub attack_type: Option<AttackType>,
}

impl PadResult {
    /// Constructs a new PAD evaluation result.
    pub fn new(is_live: bool, score: f32, attack_type: Option<AttackType>) -> Self {
        Self {
            is_live,
            score,
            attack_type,
        }
    }

    /// Convenience constructor for genuine live faces.
    pub fn live(score: f32) -> Self {
        Self {
            is_live: true,
            score,
            attack_type: None,
        }
    }

    /// Convenience constructor for spoof/attack faces.
    pub fn spoof(score: f32, attack_type: AttackType) -> Self {
        Self {
            is_live: false,
            score,
            attack_type: Some(attack_type),
        }
    }
}

/// Trait implemented by presentation attack detection backends.
pub trait PadDetector: Send + Sync {
    /// Evaluates liveness of a face candidate RGB buffer (typically 112x112 aligned crop).
    fn evaluate_liveness(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError>;
}

/// MiniFASNet ONNX Runtime Presentation Attack Detector.
pub struct OrtPadDetector {
    session: Arc<Mutex<Session>>,
    liveness_threshold: f32,
}

impl OrtPadDetector {
    /// Constructs a new `OrtPadDetector` wrapping an active ORT session.
    pub fn new(session: Arc<Mutex<Session>>, liveness_threshold: f32) -> Self {
        Self {
            session,
            liveness_threshold,
        }
    }

    /// Current liveness decision threshold.
    pub fn liveness_threshold(&self) -> f32 {
        self.liveness_threshold
    }

    /// Computes numerically stable softmax probabilities over a slice of raw logits.
    pub fn softmax(logits: &[f32]) -> Vec<f32> {
        if logits.is_empty() {
            return Vec::new();
        }

        let max_val = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let exps: Vec<f32> = logits.iter().map(|&x| (x - max_val).exp()).collect();
        let sum: f32 = exps.iter().sum();

        if sum <= 0.0 || sum.is_nan() {
            return vec![1.0 / logits.len() as f32; logits.len()];
        }

        exps.iter().map(|&v| v / sum).collect()
    }

    /// Prepares, resizes, and normalizes an RGB image to 112x112 NCHW format inside a zeroized container.
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

        if width == 0 || height == 0 {
            return Err(InferenceError::InvalidDimensions {
                expected: (112, 112),
                actual: (width, height),
            });
        }

        // Standard MiniFASNet expects 112x112 NCHW tensor [1, 3, 112, 112]
        let target_size = 112usize;
        let mut input_data = Zeroizing::new(vec![0.0f32; 3 * target_size * target_size]);

        let scale_x = width as f32 / target_size as f32;
        let scale_y = height as f32 / target_size as f32;

        for y in 0..target_size {
            let src_y = ((y as f32 * scale_y) as usize).min(height as usize - 1);
            for x in 0..target_size {
                let src_x = ((x as f32 * scale_x) as usize).min(width as usize - 1);
                let src_idx = (src_y * width as usize + src_x) * 3;

                if let (Some(&r), Some(&g), Some(&b)) =
                    (rgb.get(src_idx), rgb.get(src_idx + 1), rgb.get(src_idx + 2))
                {
                    // Normalize to [-1.0, 1.0] standard float tensor range
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

impl PadDetector for OrtPadDetector {
    fn evaluate_liveness(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError> {
        let mut input_data = Self::prepare_input(rgb, width, height)?;

        let tensor =
            ort::value::TensorRef::from_array_view(([1usize, 3, 112, 112], input_data.as_slice()))
                .map_err(|e| InferenceError::Ort(e.to_string()))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| InferenceError::PadFailed("Session mutex poisoned".to_string()))?;

        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|e| InferenceError::Ort(e.to_string()))?;

        // Zeroize input buffer immediately post-inference
        input_data.zeroize();

        let mut out_iter = outputs.into_iter();
        let (_, pad_tensor) = out_iter.next().ok_or_else(|| {
            InferenceError::PadFailed("PAD model returned zero output tensors".to_string())
        })?;

        let logits_binding = pad_tensor
            .try_extract_tensor::<f32>()
            .map_err(|e| InferenceError::Ort(e.to_string()))?;
        let logits = logits_binding.1;

        let probs = Self::softmax(logits);

        // Classification interpretation:
        // For MiniFASNet 3-class models:
        //   Class 0: Print Photo attack
        //   Class 1: Real / Live face
        //   Class 2: Screen Replay attack
        // For binary 2-class models:
        //   Class 0: Spoof
        //   Class 1: Real / Live face
        if probs.len() >= 3 {
            let p_print = probs.first().copied().unwrap_or(0.0);
            let p_live = probs.get(1).copied().unwrap_or(0.0);
            let p_replay = probs.get(2).copied().unwrap_or(0.0);

            if p_live >= self.liveness_threshold {
                Ok(PadResult::live(p_live))
            } else {
                let attack = if p_print >= p_replay {
                    AttackType::PrintPhoto
                } else {
                    AttackType::ScreenReplay
                };
                Ok(PadResult::spoof(p_live, attack))
            }
        } else if probs.len() == 2 {
            let p_live = probs.get(1).copied().unwrap_or(0.0);
            if p_live >= self.liveness_threshold {
                Ok(PadResult::live(p_live))
            } else {
                Ok(PadResult::spoof(p_live, AttackType::UnknownSpoof))
            }
        } else if let Some(&single) = probs.first() {
            // Single sigmoid output
            if single >= self.liveness_threshold {
                Ok(PadResult::live(single))
            } else {
                Ok(PadResult::spoof(single, AttackType::UnknownSpoof))
            }
        } else {
            Err(InferenceError::PadFailed(
                "Empty probability distribution returned from PAD model".to_string(),
            ))
        }
    }
}

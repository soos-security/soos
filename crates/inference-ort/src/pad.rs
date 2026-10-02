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
use crate::outputs::ZeroizingOutputs;
use zeroize::Zeroizing;

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
    /// Evaluates liveness of a face candidate RGB buffer (typically 80x80 expanded crop).
    fn evaluate_liveness(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError>;
}

/// Default live class index for MiniFASNetV2 models (Class 0: PrintPhoto, Class 1: Genuine Live, Class 2: ScreenReplay).
///
/// Single source of truth for the whole workspace: every production construction site
/// (`soos-daemon`, `soos-enroll`, `soos-gui`) uses [`OrtPadDetector::new`], which reads this
/// constant. Overriding the index in production is rejected by the repository invariant
/// `test_no_pad_live_class_index_override_outside_tests` (GitHub #146). Changing the value
/// requires a new ADR entry in `AI/DECISIONS.md` backed by a measurement on the real model.
pub const DEFAULT_MINIFASNET_LIVE_CLASS_INDEX: usize = 1;

/// Number of classes of the MiniFASNetV2 PAD head (`[PrintPhoto, Live, ScreenReplay]`).
///
/// The production detector requires exactly this many logits per inference (GitHub #214,
/// PAD-09): any other length is a wrong model or head and is an error, never a verdict.
pub const MINIFASNET_CLASS_COUNT: usize = 3;

/// Edge (pixels) of the fixed synthetic fixture used by [`OrtPadDetector::self_test`].
pub const PAD_SELF_TEST_EDGE: u32 = 80;

/// Grey level of the fixed synthetic (non-biometric) self-test fixture.
const PAD_SELF_TEST_GREY: u8 = 128;

/// Result of a successful [`OrtPadDetector::self_test`] (safe to log: no biometric data).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadSelfTestReport {
    /// Number of logits the loaded model produced for the fixture.
    pub class_count: usize,
    /// Live class index the detector reads `p_live` from.
    pub live_class_index: usize,
    /// Liveness decision threshold of the detector.
    pub liveness_threshold: f32,
}

/// Checks a PAD output vector against the expected class count and the live class index.
///
/// Fails closed with [`InferenceError::PadFailed`] when `output_len != expected_classes` or when
/// `live_class_index` does not address an element of the output.
pub fn validate_pad_output_contract(
    output_len: usize,
    live_class_index: usize,
    expected_classes: usize,
) -> Result<(), InferenceError> {
    if output_len != expected_classes {
        return Err(InferenceError::PadFailed(format!(
            "PAD model produced {output_len} output values, the contract requires {expected_classes}"
        )));
    }
    if live_class_index >= output_len {
        return Err(InferenceError::PadFailed(format!(
            "PAD live class index {live_class_index} is out of range for {output_len} classes"
        )));
    }
    Ok(())
}

/// Derives the PAD class count from a manifest entry's `output_shapes`.
///
/// - No declared output: [`MINIFASNET_CLASS_COUNT`] (legacy manifests).
/// - Exactly one declared output: its last dimension, which must equal
///   [`MINIFASNET_CLASS_COUNT`] (the attack-type mapping is defined for 3 classes only).
/// - Anything else fails closed with [`InferenceError::PadFailed`].
pub fn pad_class_count_from_manifest(
    output_shapes: &[Vec<usize>],
) -> Result<usize, InferenceError> {
    let classes = match output_shapes {
        [] => return Ok(MINIFASNET_CLASS_COUNT),
        [single] => single.last().copied().ok_or_else(|| {
            InferenceError::PadFailed("PAD manifest output shape has rank 0".to_string())
        })?,
        _ => {
            return Err(InferenceError::PadFailed(format!(
                "PAD manifest declares {} outputs, exactly one class vector is required",
                output_shapes.len()
            )))
        }
    };
    if classes != MINIFASNET_CLASS_COUNT {
        return Err(InferenceError::PadFailed(format!(
            "PAD manifest declares {classes} classes, the detector requires {MINIFASNET_CLASS_COUNT}"
        )));
    }
    Ok(classes)
}

/// MiniFASNetV2 ONNX Runtime Presentation Attack Detector.
pub struct OrtPadDetector {
    session: Arc<Mutex<Session>>,
    liveness_threshold: f32,
    live_class_index: usize,
}

impl OrtPadDetector {
    /// Constructs a new `OrtPadDetector` wrapping an active ORT session.
    pub fn new(session: Arc<Mutex<Session>>, liveness_threshold: f32) -> Self {
        Self {
            session,
            liveness_threshold,
            live_class_index: DEFAULT_MINIFASNET_LIVE_CLASS_INDEX,
        }
    }

    /// Constructs a new `OrtPadDetector` with an explicit live class index.
    ///
    /// Test-only: production code must use [`OrtPadDetector::new`] so that
    /// [`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`] stays the single source of truth
    /// (repository invariant, GitHub #146).
    pub fn new_with_class_index(
        session: Arc<Mutex<Session>>,
        liveness_threshold: f32,
        live_class_index: usize,
    ) -> Self {
        Self {
            session,
            liveness_threshold,
            live_class_index,
        }
    }

    /// Configures the live class index using a builder pattern.
    ///
    /// Test-only: production code must never override the index (see
    /// [`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`] and GitHub #146).
    pub fn with_live_class_index(mut self, live_class_index: usize) -> Self {
        self.live_class_index = live_class_index;
        self
    }

    /// Current live class index.
    pub fn live_class_index(&self) -> usize {
        self.live_class_index
    }

    /// Current liveness decision threshold.
    pub fn liveness_threshold(&self) -> f32 {
        self.liveness_threshold
    }

    /// Interprets model output probabilities into a [`PadResult`].
    ///
    /// Reads `p_live` from `probs[live_class_index]`. For 3-class models, non-live classes
    /// are mapped to attack types in ordinal order (first non-live is PrintPhoto, second is ScreenReplay).
    pub fn interpret_probabilities(
        probs: &[f32],
        liveness_threshold: f32,
        live_class_index: usize,
    ) -> Result<PadResult, InferenceError> {
        if probs.is_empty() {
            return Err(InferenceError::PadFailed(
                "Empty probability distribution returned from PAD model".to_string(),
            ));
        }

        // An index outside the output is a configuration or model error, never a spoof
        // verdict (GitHub #214, PAD-09).
        let Some(&p_live) = probs.get(live_class_index) else {
            return Err(InferenceError::PadFailed(format!(
                "PAD live class index {live_class_index} is out of range for {} classes",
                probs.len()
            )));
        };

        if probs.len() >= 3 {
            if p_live >= liveness_threshold {
                Ok(PadResult::live(p_live))
            } else {
                // Collect remaining non-live classes in ordinal index order
                let non_live: Vec<f32> = probs
                    .iter()
                    .enumerate()
                    .filter(|&(idx, _)| idx != live_class_index)
                    .map(|(_, &p)| p)
                    .collect();

                let p_print = non_live.first().copied().unwrap_or(0.0);
                let p_replay = non_live.get(1).copied().unwrap_or(0.0);

                let attack = if p_print >= p_replay {
                    AttackType::PrintPhoto
                } else {
                    AttackType::ScreenReplay
                };
                Ok(PadResult::spoof(p_live, attack))
            }
        } else if probs.len() == 2 {
            if p_live >= liveness_threshold {
                Ok(PadResult::live(p_live))
            } else {
                Ok(PadResult::spoof(p_live, AttackType::UnknownSpoof))
            }
        } else if p_live >= liveness_threshold {
            // Single sigmoid output (index 0, checked above)
            Ok(PadResult::live(p_live))
        } else {
            Ok(PadResult::spoof(p_live, AttackType::UnknownSpoof))
        }
    }

    /// Classifies softmax probabilities according to the detector's configured threshold and live class index.
    pub fn classify_probabilities(&self, probs: &[f32]) -> Result<PadResult, InferenceError> {
        Self::interpret_probabilities(probs, self.liveness_threshold, self.live_class_index)
    }

    /// Computes numerically stable softmax probabilities over a slice of raw logits.
    ///
    /// The intermediate exponentials live in a zeroizing container, and the production caller
    /// ([`PadDetector::evaluate_liveness`]) wraps the returned probabilities in one too: the PAD
    /// scores of a face are wiped on drop (GitHub #313, VIS-NEW-5).
    pub fn softmax(logits: &[f32]) -> Vec<f32> {
        if logits.is_empty() {
            return Vec::new();
        }

        let max_val = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let exps: Zeroizing<Vec<f32>> =
            Zeroizing::new(logits.iter().map(|&x| (x - max_val).exp()).collect());
        let sum: f32 = exps.iter().sum();

        if sum <= 0.0 || sum.is_nan() {
            return vec![1.0 / logits.len() as f32; logits.len()];
        }

        exps.iter().map(|&v| v / sum).collect()
    }

    /// Prepares and resizes an RGB image to an 80x80 NCHW BGR tensor inside a zeroized container.
    ///
    /// Crops that are not 80x80 are resampled bilinearly with half-pixel centres (the
    /// `cv2.resize` `INTER_LINEAR` convention); an 80x80 crop is copied exactly.
    ///
    /// Value range: raw pixel values as `f32` in `[0.0, 255.0]`, **not** divided by 255.
    /// Upstream Silent-Face-Anti-Spoofing `to_tensor` returns `img.float()` without `div(255)`,
    /// and both MiniFASNet checkpoints were trained on that range (ADR 2026-10-01 "PAD Input
    /// Range Matches Upstream (0-255)", GitHub #212). Every PAD member uses this function.
    /// Channel ordering is BGR: channel 0 = Blue, channel 1 = Green, channel 2 = Red.
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
                expected: (80, 80),
                actual: (width, height),
            });
        }

        // MiniFASNetV2 expects 80x80 NCHW BGR tensor [1, 3, 80, 80]
        let target_size = 80usize;
        let mut input_data = Zeroizing::new(vec![0.0f32; 3 * target_size * target_size]);

        // Half-pixel centre bilinear sampling (cv2.resize INTER_LINEAR convention, GitHub
        // #213). An 80x80 crop maps every output pixel exactly onto its source pixel.
        let scale_x = width as f32 / target_size as f32;
        let scale_y = height as f32 / target_size as f32;
        let max_x = (width - 1) as f32;
        let max_y = (height - 1) as f32;
        let stride = width as usize;
        let last_x = width as usize - 1;
        let last_y = height as usize - 1;
        let plane = target_size * target_size;

        for y in 0..target_size {
            let fy = ((y as f32 + 0.5) * scale_y - 0.5).clamp(0.0, max_y);
            let y0 = (fy.floor() as usize).min(last_y);
            let y1 = (y0 + 1).min(last_y);
            let dy = fy - y0 as f32;
            for x in 0..target_size {
                let fx = ((x as f32 + 0.5) * scale_x - 0.5).clamp(0.0, max_x);
                let x0 = (fx.floor() as usize).min(last_x);
                let x1 = (x0 + 1).min(last_x);
                let dx = fx - x0 as f32;

                let w00 = (1.0 - dx) * (1.0 - dy);
                let w10 = dx * (1.0 - dy);
                let w01 = (1.0 - dx) * dy;
                let w11 = dx * dy;

                let i00 = (y0 * stride + x0) * 3;
                let i10 = (y0 * stride + x1) * 3;
                let i01 = (y1 * stride + x0) * 3;
                let i11 = (y1 * stride + x1) * 3;

                let sample = |c: usize| -> Option<f32> {
                    Some(
                        w00 * f32::from(*rgb.get(i00 + c)?)
                            + w10 * f32::from(*rgb.get(i10 + c)?)
                            + w01 * f32::from(*rgb.get(i01 + c)?)
                            + w11 * f32::from(*rgb.get(i11 + c)?),
                    )
                };

                if let (Some(r), Some(g), Some(b)) = (sample(0), sample(1), sample(2)) {
                    // Upstream MiniFASNet range: raw values in [0, 255] (no division).
                    // Channel ordering: BGR (channel 0 = B, channel 1 = G, channel 2 = R).
                    let idx = y * target_size + x;
                    if let Some(slot) = input_data.get_mut(idx) {
                        *slot = b;
                    }
                    if let Some(slot) = input_data.get_mut(plane + idx) {
                        *slot = g;
                    }
                    if let Some(slot) = input_data.get_mut(2 * plane + idx) {
                        *slot = r;
                    }
                }
            }
        }

        Ok(input_data)
    }
}

impl OrtPadDetector {
    /// Runs one inference and returns the raw logits of the first output tensor, after
    /// checking them against the MiniFASNetV2 output contract.
    fn run_logits(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Zeroizing<Vec<f32>>, InferenceError> {
        // `Zeroizing` wipes the input tensor on drop; ORT-owned logits are wiped in place by
        // `ZeroizingOutputs` (GitHub #255).
        let input_data = Self::prepare_input(rgb, width, height)?;

        let tensor =
            ort::value::TensorRef::from_array_view(([1usize, 3, 80, 80], input_data.as_slice()))
                .map_err(|e| InferenceError::Ort(e.to_string()))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| InferenceError::PadFailed("Session mutex poisoned".to_string()))?;

        let outputs = ZeroizingOutputs::new(
            session
                .run(ort::inputs![tensor])
                .map_err(|e| InferenceError::Ort(e.to_string()))?,
        );

        let pad_tensor = outputs.values().next().ok_or_else(|| {
            InferenceError::PadFailed("PAD model returned zero output tensors".to_string())
        })?;

        let logits_binding = pad_tensor
            .try_extract_tensor::<f32>()
            .map_err(|e| InferenceError::Ort(e.to_string()))?;
        let logits = logits_binding.1;
        // Per-inference output contract (GitHub #214): a wrong head is an error, never a
        // verdict. Checked before the copy, so only a 3-element vector is ever materialized.
        validate_pad_output_contract(logits.len(), self.live_class_index, MINIFASNET_CLASS_COUNT)?;
        // The copy stays inside a wipe-on-drop container (GitHub #255).
        Ok(Zeroizing::new(logits.to_vec()))
    }

    /// Startup self-test (GitHub #214, PAD-09): runs one inference on a fixed synthetic
    /// 80x80 grey fixture and checks that the model emits `expected_classes` logits (derived
    /// from the manifest with [`pad_class_count_from_manifest`]) and that the live class index
    /// addresses one of them. Fails closed with [`InferenceError::PadFailed`] otherwise.
    ///
    /// The fixture is not a face; its verdict is not asserted, only the output contract.
    pub fn self_test(&self, expected_classes: usize) -> Result<PadSelfTestReport, InferenceError> {
        validate_pad_output_contract(
            MINIFASNET_CLASS_COUNT,
            self.live_class_index,
            expected_classes,
        )?;
        let edge = PAD_SELF_TEST_EDGE as usize;
        let fixture = vec![PAD_SELF_TEST_GREY; edge * edge * 3];
        let logits = self.run_logits(&fixture, PAD_SELF_TEST_EDGE, PAD_SELF_TEST_EDGE)?;
        validate_pad_output_contract(logits.len(), self.live_class_index, expected_classes)?;
        Ok(PadSelfTestReport {
            class_count: logits.len(),
            live_class_index: self.live_class_index,
            liveness_threshold: self.liveness_threshold,
        })
    }
}

impl PadDetector for OrtPadDetector {
    fn evaluate_liveness(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError> {
        let logits = self.run_logits(rgb, width, height)?;
        let probs = Zeroizing::new(Self::softmax(&logits));
        Self::interpret_probabilities(&probs, self.liveness_threshold, self.live_class_index)
    }
}

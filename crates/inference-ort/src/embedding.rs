//! Biometric embedding representation, L2-normalization, and feature extractor trait.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Vector normalization, cosine similarity dot product, and pixel buffer normalization"
)]

use zeroize::{Zeroize, Zeroizing};

use crate::error::InferenceError;
use crate::outputs::ZeroizingOutputs;

/// High-dimensional facial biometric embedding vector (e.g. 512D ArcFace or 128D) with automatic memory zeroization.
#[derive(Debug, Clone, PartialEq, Zeroize)]
pub struct BiometricEmbedding {
    vector: Zeroizing<Vec<f32>>,
}

impl BiometricEmbedding {
    /// Constructs a biometric embedding from raw floating point features.
    pub fn new(vector: Vec<f32>) -> Self {
        Self {
            vector: Zeroizing::new(vector),
        }
    }

    /// Constructs an embedding asserting that the vector is already L2-normalized.
    pub fn from_normalized(vector: Vec<f32>, epsilon: f32) -> Result<Self, InferenceError> {
        let emb = Self::new(vector);
        if !emb.is_normalized(epsilon) {
            return Err(InferenceError::EmbeddingFailed(format!(
                "Vector is not L2-normalized: L2 norm is {}, expected 1.0 ± {}",
                emb.l2_norm(),
                epsilon
            )));
        }
        Ok(emb)
    }

    /// Borrow embedding components as slice.
    pub fn as_slice(&self) -> &[f32] {
        &self.vector
    }

    /// Dimensionality of the embedding vector.
    pub fn len(&self) -> usize {
        self.vector.len()
    }

    /// Returns true if embedding vector is empty.
    pub fn is_empty(&self) -> bool {
        self.vector.is_empty()
    }

    /// Calculates Euclidean L2 norm: `||v||_2 = sqrt(sum(v_i^2))`.
    pub fn l2_norm(&self) -> f32 {
        let sum_sq: f32 = self.vector.iter().map(|&x| x * x).sum();
        sum_sq.sqrt()
    }

    /// Normalizes vector in-place such that `||v||_2 == 1.0`.
    pub fn normalize(&mut self) -> Result<(), InferenceError> {
        let norm = self.l2_norm();
        if norm <= 1e-12 {
            return Err(InferenceError::EmbeddingFailed(
                "Cannot L2-normalize zero or near-zero embedding vector".to_string(),
            ));
        }

        for x in &mut *self.vector {
            *x /= norm;
        }

        Ok(())
    }

    /// Returns the inner zeroized vector container.
    pub fn into_inner(self) -> Zeroizing<Vec<f32>> {
        self.vector
    }

    /// Clones the inner floats into a raw vector.
    pub fn to_vec(&self) -> Vec<f32> {
        self.vector.to_vec()
    }

    /// Checks whether the embedding vector satisfies the L2-normalization criterion `V2` (norm ≈ 1.0).
    pub fn is_normalized(&self, epsilon: f32) -> bool {
        let norm = self.l2_norm();
        (norm - 1.0).abs() <= epsilon
    }

    /// Computes cosine similarity between two L2-normalized embedding vectors:
    /// `cosine = dot(a, b) / (||a|| * ||b||)`.
    /// When both are unit vectors, this simplifies to `dot(a, b)`.
    pub fn cosine_similarity(&self, other: &BiometricEmbedding) -> Result<f32, InferenceError> {
        if self.len() != other.len() {
            return Err(InferenceError::DimensionMismatch {
                expected: self.len(),
                actual: other.len(),
            });
        }

        let dot: f32 = self
            .vector
            .iter()
            .zip(other.vector.iter())
            .map(|(&a, &b)| a * b)
            .sum();

        let norm_product = self.l2_norm() * other.l2_norm();
        if norm_product <= 1e-12 {
            return Err(InferenceError::EmbeddingFailed(
                "Degenerate zero-norm vector during cosine similarity computation".to_string(),
            ));
        }

        let similarity = (dot / norm_product).clamp(-1.0, 1.0);
        Ok(similarity)
    }
}

/// Trait implemented by facial feature extractor backends.
pub trait EmbeddingExtractor: Send + Sync {
    /// Extracts an L2-normalized biometric embedding from an aligned 112x112 RGB face crop.
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError>;
}

use ort::session::Session;
use std::sync::{Arc, Mutex};

/// ArcFace 512D feature extractor backed by an ONNX Runtime session.
///
/// The attested model (manifest id `arcface_w600k_mbf`, a historical name) is a Keras ArcFace
/// ResNet34 exported with tf2onnx: NHWC input `input_1` `[N, 112, 112, 3]`, output
/// `embedding` `[N, 512]`. The layout is detected from the session input; the registry has
/// already checked it against the manifest `input_layout`. Pixels are fed in B, G, R order
/// normalized as `(x - 127.5) / 127.5`. The upstream model card of the attested file documents
/// RGB and `(x - 127.5) / 128.0`; the divisor is template-neutral, the channel order is not, and
/// switching it is an owner decision tied to re-enrollment (GitHub #278,
/// `tests/embedding_preprocessing_evaluation_tests.rs`).
pub struct OrtEmbeddingExtractor {
    session: Arc<Mutex<Session>>,
    is_nhwc: bool,
}

impl OrtEmbeddingExtractor {
    pub fn new(session: Arc<Mutex<Session>>) -> Self {
        let is_nhwc = if let Ok(guard) = session.lock() {
            if let Some(input) = guard.inputs().first() {
                match input.dtype() {
                    ort::value::ValueType::Tensor { shape, .. } => shape.last().copied() == Some(3),
                    _ => false,
                }
            } else {
                false
            }
        } else {
            false
        };

        Self { session, is_nhwc }
    }

    /// Whether the attached session takes a channels-last `[N, 112, 112, 3]` input.
    pub fn is_nhwc(&self) -> bool {
        self.is_nhwc
    }

    /// Prepares, resizes, and normalizes an aligned face crop inside a zeroized container.
    /// If `is_nhwc` is true, formats as `[1, 112, 112, 3]` (NHWC); otherwise `[1, 3, 112, 112]` (NCHW).
    pub fn prepare_input_layout(
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
        is_nhwc: bool,
    ) -> Result<Zeroizing<Vec<f32>>, InferenceError> {
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

        let target_size = 112usize;
        let mut input_data = Zeroizing::new(vec![0.0f32; 3 * target_size * target_size]);

        let scale_x = width as f32 / target_size as f32;
        let scale_y = height as f32 / target_size as f32;

        for y in 0..target_size {
            let src_y = ((y as f32 * scale_y) as usize).min(height as usize - 1);
            for x in 0..target_size {
                let src_x = ((x as f32 * scale_x) as usize).min(width as usize - 1);
                let src_idx = (src_y * width as usize + src_x) * 3;

                if let (Some(&r), Some(&g), Some(&b)) = (
                    aligned_crop_rgb.get(src_idx),
                    aligned_crop_rgb.get(src_idx + 1),
                    aligned_crop_rgb.get(src_idx + 2),
                ) {
                    let norm_r = (r as f32 - 127.5) / 127.5;
                    let norm_g = (g as f32 - 127.5) / 127.5;
                    let norm_b = (b as f32 - 127.5) / 127.5;

                    if is_nhwc {
                        let idx = (y * target_size + x) * 3;
                        if let Some(slot) = input_data.get_mut(idx) {
                            *slot = norm_b;
                        }
                        if let Some(slot) = input_data.get_mut(idx + 1) {
                            *slot = norm_g;
                        }
                        if let Some(slot) = input_data.get_mut(idx + 2) {
                            *slot = norm_r;
                        }
                    } else {
                        // NCHW format: planes written in B, G, R order (B=0, G=1, R=2)
                        let b_idx = y * target_size + x;
                        let g_idx = target_size * target_size + y * target_size + x;
                        let r_idx = 2 * target_size * target_size + y * target_size + x;

                        if let Some(slot) = input_data.get_mut(b_idx) {
                            *slot = norm_b;
                        }
                        if let Some(slot) = input_data.get_mut(g_idx) {
                            *slot = norm_g;
                        }
                        if let Some(slot) = input_data.get_mut(r_idx) {
                            *slot = norm_r;
                        }
                    }
                }
            }
        }

        Ok(input_data)
    }

    /// Prepares, resizes, and normalizes an aligned face crop to 112x112 NCHW format inside a zeroized container.
    pub fn prepare_input(
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Zeroizing<Vec<f32>>, InferenceError> {
        Self::prepare_input_layout(aligned_crop_rgb, width, height, false)
    }
}

impl EmbeddingExtractor for OrtEmbeddingExtractor {
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError> {
        // `Zeroizing` wipes the input tensor on drop; the ORT-owned output (the raw,
        // unnormalized embedding) is wiped in place by `ZeroizingOutputs` (GitHub #255).
        let input_data = Self::prepare_input_layout(aligned_crop_rgb, width, height, self.is_nhwc)?;

        let shape = if self.is_nhwc {
            [1usize, 112, 112, 3]
        } else {
            [1usize, 3, 112, 112]
        };
        let tensor = ort::value::TensorRef::from_array_view((shape, input_data.as_slice()))
            .map_err(|e| InferenceError::Ort(e.to_string()))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| InferenceError::EmbeddingFailed("Session mutex poisoned".to_string()))?;

        let outputs = ZeroizingOutputs::new(
            session
                .run(ort::inputs![tensor])
                .map_err(|e| InferenceError::Ort(e.to_string()))?,
        );

        let emb_tensor = outputs.values().next().ok_or_else(|| {
            InferenceError::EmbeddingFailed(
                "Embedding model returned zero output tensors".to_string(),
            )
        })?;

        let emb_data = emb_tensor
            .try_extract_tensor::<f32>()
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .1;

        let mut embedding = BiometricEmbedding::new(emb_data.to_vec());
        embedding.normalize()?;

        Ok(embedding)
    }
}

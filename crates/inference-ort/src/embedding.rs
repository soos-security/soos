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
use crate::manifest::TensorLayout;

use crate::outputs::ZeroizingOutputs;

/// Static contract of an attested embedding model: what the extractor feeds and expects
/// (GitHub #278). Templates are bound to `model_id` and `dimension`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingModelSpec {
    /// Manifest id recorded in every template produced with this model.
    pub model_id: &'static str,
    /// Exact length of the output vector (`[1, dimension]`).
    pub dimension: usize,
    /// Physical layout of the graph input; a session with another layout fails closed.
    pub input_layout: TensorLayout,
}

/// OpenCV Zoo SFace 2021dec (`face_recognition_sface_2021dec.onnx`, Apache-2.0): NCHW input
/// `data` `[1, 3, 112, 112]` fed as R, G, B planes of raw `0..255` values (the graph applies
/// `(x - 127.5) / 128` itself, exactly like OpenCV `FaceRecognizerSF::feature`), output `fc1`
/// `[1, 128]`, L2-normalized by the extractor.
pub const SFACE_2021DEC: EmbeddingModelSpec = EmbeddingModelSpec {
    model_id: "sface_2021dec",
    dimension: 128,
    input_layout: TensorLayout::Nchw,
};

/// The embedding model shipped by soos: the single source of the model id and dimension used
/// by `soos-daemon`, `soos-enroll` and `soos-gui` (owner decision 2026-10-01, GitHub #278).
pub const SHIPPED_EMBEDDING_MODEL: EmbeddingModelSpec = SFACE_2021DEC;

// `OrtEmbeddingExtractor::new` binds the shipped spec without the runtime layout check of
// `with_spec`; this compile-time check keeps that sound (GitHub #298).
const _: () = assert!(
    matches!(SHIPPED_EMBEDDING_MODEL.input_layout, TensorLayout::Nchw),
    "the shipped embedding model must take an NCHW input"
);

/// Output length of the shipped embedding model (`[1, 128]`).
///
/// [`OrtEmbeddingExtractor`] rejects any other output length with
/// [`InferenceError::DimensionMismatch`] (GitHub #268, VIS-14) instead of emitting a vector
/// that would only fail later in the matcher.
pub const EMBEDDING_DIMENSION: usize = SHIPPED_EMBEDDING_MODEL.dimension;

/// Whether a stored template (`template_model_id`, `template_dimension`) belongs to the
/// embedding space of the loaded model (`loaded_model_id`, `loaded_dimension`).
///
/// The ids must be equal (no alias) and, when the loaded extractor reports a dimension, the
/// vector length must equal it. Embeddings of two models are never comparable.
#[must_use]
pub fn template_matches_model(
    loaded_model_id: &str,
    loaded_dimension: Option<usize>,
    template_model_id: &str,
    template_dimension: usize,
) -> bool {
    template_model_id == loaded_model_id && loaded_dimension.is_none_or(|d| d == template_dimension)
}

/// Facial biometric embedding vector (128-D for the shipped SFace model) with automatic memory zeroization.
///
/// `Debug` prints the dimension only, never the values (GitHub #313, VIS-NEW-6).
#[derive(Clone, PartialEq, Zeroize)]
pub struct BiometricEmbedding {
    vector: Zeroizing<Vec<f32>>,
}

impl std::fmt::Debug for BiometricEmbedding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BiometricEmbedding")
            .field("dimension", &self.vector.len())
            .finish_non_exhaustive()
    }
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
    ///
    /// Fails closed on a non-finite component or norm (NaN, infinity, or squares overflowing
    /// to an infinite norm): such a model output is never turned into an embedding
    /// (GitHub #313, VIS-NEW-3).
    pub fn normalize(&mut self) -> Result<(), InferenceError> {
        if self.vector.iter().any(|x| !x.is_finite()) {
            return Err(InferenceError::EmbeddingFailed(
                "Cannot L2-normalize an embedding with non-finite components".to_string(),
            ));
        }
        let norm = self.l2_norm();
        if !norm.is_finite() {
            return Err(InferenceError::EmbeddingFailed(
                "Cannot L2-normalize an embedding whose norm is not finite".to_string(),
            ));
        }
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

    /// Copies the inner floats into a new wipe-on-drop vector (GitHub #287): the copy of the
    /// biometric template is zeroized when the caller drops it, like the original.
    pub fn to_vec(&self) -> Zeroizing<Vec<f32>> {
        Zeroizing::new(self.vector.to_vec())
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

    /// Exact length of every embedding this extractor emits, when known.
    ///
    /// Templates of another length are refused before matching (GitHub #278). `None` (the
    /// default) binds templates by model id only.
    fn output_dimension(&self) -> Option<usize> {
        None
    }
}

use ort::session::Session;
use std::sync::{Arc, Mutex};

/// Model-aware feature extractor backed by an ONNX Runtime session (GitHub #278).
///
/// [`OrtEmbeddingExtractor::new`] binds the session to [`SHIPPED_EMBEDDING_MODEL`] (OpenCV Zoo
/// SFace 2021dec: NCHW input `data` `[1, 3, 112, 112]`, output `fc1` `[1, 128]`). Pixels are fed
/// as R, G, B planes of raw `0..255` values (the graph normalizes them itself, OpenCV
/// `blobFromImage(aligned, 1, Size(112, 112), Scalar(0, 0, 0), swapRB = true)`); the output is
/// L2-normalized. The layout is detected from the session input and must equal the spec's
/// layout; the registry has already checked it against the manifest `input_layout`.
pub struct OrtEmbeddingExtractor {
    session: Arc<Mutex<Session>>,
    /// Contract of the model the session was attested as.
    spec: EmbeddingModelSpec,
    /// Physical input layout inferred from the session when it equals `spec.input_layout`;
    /// `None` when it cannot be inferred (poisoned lock, no input, non-tensor input, rank other
    /// than 4, no size-3 channel axis) or differs from the spec. Extraction fails closed on
    /// `None` (GitHub #268, VIS-14; GitHub #278).
    layout: Option<TensorLayout>,
}

/// Infers the physical layout of a rank-4 image input: channels last (`[N, H, W, 3]`) or
/// channels first (`[N, 3, H, W]`). Anything else is not inferable.
fn infer_input_layout(shape: &[i64]) -> Option<TensorLayout> {
    match shape {
        [_, _, _, 3] => Some(TensorLayout::Nhwc),
        [_, 3, _, _] => Some(TensorLayout::Nchw),
        _ => None,
    }
}

impl OrtEmbeddingExtractor {
    /// Binds `session` to the shipped embedding model ([`SHIPPED_EMBEDDING_MODEL`]), whose
    /// NCHW layout is checked at compile time, so this constructor cannot fail.
    pub fn new(session: Arc<Mutex<Session>>) -> Self {
        Self::bind(session, SHIPPED_EMBEDDING_MODEL)
    }

    /// Binds `session` to the embedding model described by `spec`.
    ///
    /// Fails closed with [`InferenceError::InvalidInput`] when `spec.input_layout` is not
    /// [`TensorLayout::Nchw`]: [`EmbeddingExtractor::extract_embedding`] always feeds an NCHW
    /// `[1, 3, 112, 112]` tensor, so any other layout could only fail at run time
    /// (GitHub #298). A session whose own layout differs from the spec is still accepted here
    /// and fails closed at extraction ([`Self::input_layout`] is `None`).
    pub fn with_spec(
        session: Arc<Mutex<Session>>,
        spec: EmbeddingModelSpec,
    ) -> Result<Self, InferenceError> {
        if spec.input_layout != TensorLayout::Nchw {
            return Err(InferenceError::InvalidInput(format!(
                "embedding model '{}' declares a {:?} input layout; the extractor only feeds \
                 NCHW [1, 3, 112, 112] tensors",
                spec.model_id, spec.input_layout
            )));
        }
        Ok(Self::bind(session, spec))
    }

    /// Shared constructor; callers guarantee that `spec.input_layout` is NCHW.
    fn bind(session: Arc<Mutex<Session>>, spec: EmbeddingModelSpec) -> Self {
        let layout = session
            .lock()
            .ok()
            .and_then(|guard| {
                guard
                    .inputs()
                    .first()
                    .and_then(|input| match input.dtype() {
                        ort::value::ValueType::Tensor { shape, .. } => infer_input_layout(shape),
                        _ => None,
                    })
            })
            .filter(|layout| *layout == spec.input_layout);

        Self {
            session,
            spec,
            layout,
        }
    }

    /// Contract of the model this extractor feeds.
    pub fn spec(&self) -> EmbeddingModelSpec {
        self.spec
    }

    /// Whether the attached session takes a channels-last `[N, 112, 112, 3]` input (never the
    /// case for an accepted SFace session).
    pub fn is_nhwc(&self) -> bool {
        self.layout == Some(TensorLayout::Nhwc)
    }

    /// Input layout inferred from the session, or `None` when it could not be inferred or
    /// differs from the spec (every extraction then fails closed with
    /// [`InferenceError::TensorError`]).
    pub fn input_layout(&self) -> Option<TensorLayout> {
        self.layout
    }

    /// Prepares and resizes an aligned face crop to the SFace input inside a zeroized
    /// container: NCHW `[1, 3, 112, 112]`, planes 0, 1, 2 = R, G, B, raw `f32` values in
    /// `[0.0, 255.0]` (no mean, no divisor: the graph normalizes its input itself).
    pub fn prepare_input(
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Zeroizing<Vec<f32>>, InferenceError> {
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(3))
            .ok_or_else(|| InferenceError::InvalidInput("Image dimensions overflow".to_string()))?;

        // A zero dimension would underflow `width - 1` / `height - 1` below (GitHub #313,
        // VIS-NEW-2); same guard as the PAD input preparation.
        if width == 0 || height == 0 {
            return Err(InferenceError::InvalidDimensions {
                expected: (112, 112),
                actual: (width, height),
            });
        }

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
                    // NCHW planes in R, G, B order (R=0, G=1, B=2), raw values.
                    let plane = target_size * target_size;
                    let idx = y * target_size + x;
                    for (offset, value) in [(0, r), (plane, g), (2 * plane, b)] {
                        if let Some(slot) = input_data.get_mut(offset + idx) {
                            *slot = f32::from(value);
                        }
                    }
                }
            }
        }

        Ok(input_data)
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
        if self.layout.is_none() {
            return Err(InferenceError::TensorError(format!(
                "embedding model input layout is not the {:?} layout of '{}' (expected a \
                 rank-4 [N, 3, 112, 112] input)",
                self.spec.input_layout, self.spec.model_id
            )));
        }
        let input_data = Self::prepare_input(aligned_crop_rgb, width, height)?;

        let shape = [1usize, 3, 112, 112];
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

        if emb_data.len() != self.spec.dimension {
            return Err(InferenceError::DimensionMismatch {
                expected: self.spec.dimension,
                actual: emb_data.len(),
            });
        }

        let mut embedding = BiometricEmbedding::new(emb_data.to_vec());
        embedding.normalize()?;

        Ok(embedding)
    }

    fn output_dimension(&self) -> Option<usize> {
        Some(self.spec.dimension)
    }
}

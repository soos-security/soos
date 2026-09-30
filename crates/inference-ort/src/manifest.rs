//! Cryptographic model manifest parsing and SHA-256 verification.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::InferenceError;

/// Manifest file metadata header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestHeader {
    pub version: String,
}

/// Physical memory layout of an image input tensor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TensorLayout {
    /// Channels first: `[N, C, H, W]`.
    #[default]
    #[serde(rename = "NCHW")]
    Nchw,
    /// Channels last: `[N, H, W, C]`.
    #[serde(rename = "NHWC")]
    Nhwc,
}

/// Metadata specification for a single machine learning model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub id: String,
    pub filename: String,
    pub sha256: String,
    pub license: String,
    pub source_url: String,
    pub description: String,
    /// Logical input shape, always written `[N, C, H, W]` for image models.
    pub input_shape: Vec<usize>,
    /// Physical layout of the input tensor (`"NCHW"` when omitted).
    #[serde(default)]
    pub input_layout: TensorLayout,
    #[serde(default)]
    pub output_shapes: Vec<Vec<usize>>,
}

impl ModelMetadata {
    /// Physical input tensor dims implied by the logical `input_shape` and `input_layout`.
    ///
    /// `input_shape` is always written as the logical `[N, C, H, W]` shape. For
    /// [`TensorLayout::Nhwc`] the physical dims are `[N, H, W, C]`, which requires rank 4.
    pub fn expected_input_dims(&self) -> Result<Vec<usize>, InferenceError> {
        match self.input_layout {
            TensorLayout::Nchw => Ok(self.input_shape.clone()),
            TensorLayout::Nhwc => match self.input_shape.as_slice() {
                &[n, c, h, w] => Ok(vec![n, h, w, c]),
                other => Err(self.shape_mismatch(format!(
                    "NHWC layout requires a rank-4 [N, C, H, W] input_shape, got {other:?}"
                ))),
            },
        }
    }

    /// Validates the tensor shapes reported by an ONNX Runtime session against this entry.
    ///
    /// - The session must expose exactly one input whose physical dims equal
    ///   [`Self::expected_input_dims`].
    /// - When `output_shapes` is declared, the session must expose exactly that many outputs,
    ///   each matching in order.
    /// - Negative session dims are symbolic (dynamic) and match any declared value; every other
    ///   dim, including zero, must be equal. Ranks must always be equal.
    pub fn validate_session_shapes(
        &self,
        inputs: &[Vec<i64>],
        outputs: &[Vec<i64>],
    ) -> Result<(), InferenceError> {
        let expected_input = self.expected_input_dims()?;
        let [actual_input] = inputs else {
            return Err(self.shape_mismatch(format!(
                "expected exactly 1 input tensor, session has {}",
                inputs.len()
            )));
        };
        if !dims_match(&expected_input, actual_input) {
            return Err(self.shape_mismatch(format!(
                "input: manifest declares {expected_input:?} ({:?}), session has {actual_input:?}",
                self.input_layout
            )));
        }

        if self.output_shapes.is_empty() {
            return Ok(());
        }
        if outputs.len() != self.output_shapes.len() {
            return Err(self.shape_mismatch(format!(
                "expected {} output tensors, session has {}",
                self.output_shapes.len(),
                outputs.len()
            )));
        }
        for (index, (declared, actual)) in self.output_shapes.iter().zip(outputs).enumerate() {
            if !dims_match(declared, actual) {
                return Err(self.shape_mismatch(format!(
                    "output {index}: manifest declares {declared:?}, session has {actual:?}"
                )));
            }
        }
        Ok(())
    }

    fn shape_mismatch(&self, detail: String) -> InferenceError {
        InferenceError::ModelShapeMismatch {
            id: self.id.clone(),
            detail,
        }
    }
}

/// Compares declared dims with session dims; negative session dims are dynamic wildcards.
fn dims_match(declared: &[usize], actual: &[i64]) -> bool {
    declared.len() == actual.len()
        && declared
            .iter()
            .zip(actual)
            .all(|(&d, &a)| a < 0 || usize::try_from(a).is_ok_and(|a| a == d))
}

/// Parsed model manifest container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    pub manifest: ManifestHeader,
    pub models: HashMap<String, ModelMetadata>,
}

impl ModelManifest {
    /// Loads and parses a `manifest.toml` file from the specified path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, InferenceError> {
        let p = path.as_ref();
        let content = std::fs::read_to_string(p).map_err(|e| InferenceError::ManifestIo {
            path: p.to_path_buf(),
            source: e,
        })?;
        Self::from_toml_str(&content)
    }

    /// Parses a manifest from a TOML formatted string.
    pub fn from_toml_str(content: &str) -> Result<Self, InferenceError> {
        let manifest: ModelManifest = toml::from_str(content)?;
        Ok(manifest)
    }

    /// Looks up a model entry by its identifier.
    pub fn get_model(&self, id: &str) -> Option<&ModelMetadata> {
        self.models.get(id)
    }

    /// Computes the hexadecimal lowercase SHA-256 digest of a target file.
    pub fn compute_sha256<P: AsRef<Path>>(file_path: P) -> Result<String, std::io::Error> {
        let mut file = File::open(file_path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];

        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            if let Some(slice) = buffer.get(..bytes_read) {
                hasher.update(slice);
            }
        }

        let result = hasher.finalize();
        Ok(format!("{:x}", result))
    }

    /// Verifies that a specific model file matches the SHA-256 hash registered in this manifest.
    pub fn verify_model_checksum<P: AsRef<Path>>(
        &self,
        id: &str,
        file_path: P,
    ) -> Result<(), InferenceError> {
        let p = file_path.as_ref();
        let metadata = self
            .get_model(id)
            .ok_or_else(|| InferenceError::ModelNotFound {
                id: id.to_string(),
                path: p.to_path_buf(),
            })?;

        if !p.is_file() {
            return Err(InferenceError::ModelNotFound {
                id: id.to_string(),
                path: p.to_path_buf(),
            });
        }

        let computed = Self::compute_sha256(p).map_err(|e| InferenceError::ModelIo {
            id: id.to_string(),
            path: p.to_path_buf(),
            source: e,
        })?;

        if !computed.eq_ignore_ascii_case(&metadata.sha256) {
            return Err(InferenceError::ChecksumMismatch {
                id: id.to_string(),
                expected: metadata.sha256.clone(),
                actual: computed,
            });
        }

        Ok(())
    }

    /// Verifies all models registered in the manifest against a target directory.
    pub fn verify_directory<P: AsRef<Path>>(&self, dir: P) -> Result<(), InferenceError> {
        let d = dir.as_ref();
        for (id, metadata) in &self.models {
            let model_path = d.join(&metadata.filename);
            self.verify_model_checksum(id, &model_path)?;
        }
        Ok(())
    }
}

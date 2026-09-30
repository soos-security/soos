//! Cryptographic model manifest parsing and SHA-256 verification.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::InferenceError;

/// Upper bound on the size of a model file read into memory for attestation (512 MiB).
///
/// The largest attested model (ArcFace ResNet34) is 136.6 MB; the bound leaves headroom for a
/// future model while keeping the verified in-memory read bounded (GitHub #246).
pub const MAX_MODEL_FILE_BYTES: u64 = 512 * 1024 * 1024;

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
#[serde(from = "RawModelMetadata", into = "RawModelMetadata")]
pub struct ModelMetadata {
    pub id: String,
    pub filename: String,
    pub sha256: String,
    pub license: String,
    pub source_url: String,
    pub description: String,
    /// Logical input shape, always written `[N, C, H, W]` for image models.
    pub input_shape: Vec<usize>,
    /// Physical layout of the input tensor (`NCHW` when the manifest omits `input_layout`).
    pub input_layout: TensorLayout,
    /// Whether the manifest entry declared `input_layout` explicitly. Entries written before
    /// layout attestation existed omit it; their layout is then not asserted (the SHA-256 still
    /// binds the exact file), while an explicit layout is always enforced.
    pub input_layout_declared: bool,
    pub output_shapes: Vec<Vec<usize>>,
}

/// On-disk TOML form of [`ModelMetadata`], keeping `input_layout` optional.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawModelMetadata {
    id: String,
    filename: String,
    sha256: String,
    license: String,
    source_url: String,
    description: String,
    input_shape: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_layout: Option<TensorLayout>,
    #[serde(default)]
    output_shapes: Vec<Vec<usize>>,
}

impl From<RawModelMetadata> for ModelMetadata {
    fn from(raw: RawModelMetadata) -> Self {
        Self {
            id: raw.id,
            filename: raw.filename,
            sha256: raw.sha256,
            license: raw.license,
            source_url: raw.source_url,
            description: raw.description,
            input_shape: raw.input_shape,
            input_layout: raw.input_layout.unwrap_or_default(),
            input_layout_declared: raw.input_layout.is_some(),
            output_shapes: raw.output_shapes,
        }
    }
}

impl From<ModelMetadata> for RawModelMetadata {
    fn from(meta: ModelMetadata) -> Self {
        Self {
            id: meta.id,
            filename: meta.filename,
            sha256: meta.sha256,
            license: meta.license,
            source_url: meta.source_url,
            description: meta.description,
            input_shape: meta.input_shape,
            input_layout: meta.input_layout_declared.then_some(meta.input_layout),
            output_shapes: meta.output_shapes,
        }
    }
}

impl ModelMetadata {
    /// Physical input tensor dims implied by the logical `input_shape` and `input_layout`.
    ///
    /// `input_shape` is always written as the logical `[N, C, H, W]` shape. For
    /// [`TensorLayout::Nhwc`] the physical dims are `[N, H, W, C]`, which requires rank 4.
    pub fn expected_input_dims(&self) -> Result<Vec<usize>, InferenceError> {
        match self.input_layout {
            TensorLayout::Nchw => Ok(self.input_shape.clone()),
            TensorLayout::Nhwc => nhwc_dims(&self.input_shape).ok_or_else(|| {
                self.shape_mismatch(format!(
                    "NHWC layout requires a rank-4 [N, C, H, W] input_shape, got {:?}",
                    self.input_shape
                ))
            }),
        }
    }

    /// Validates the tensor shapes reported by an ONNX Runtime session against this entry.
    ///
    /// - The session must expose exactly one input whose physical dims equal
    ///   [`Self::expected_input_dims`]. When the entry does not declare `input_layout`
    ///   ([`Self::input_layout_declared`] is false), the layout is not asserted: the input may be
    ///   the logical shape in NCHW or NHWC order, with rank and dims still enforced.
    /// - When `output_shapes` is declared, the session must expose exactly that many outputs,
    ///   each matching in order.
    /// - Negative session dims are symbolic (dynamic) and match any declared value; every other
    ///   dim, including zero, must be equal. Ranks must always be equal.
    pub fn validate_session_shapes(
        &self,
        inputs: &[Vec<i64>],
        outputs: &[Vec<i64>],
    ) -> Result<(), InferenceError> {
        let [actual_input] = inputs else {
            return Err(self.shape_mismatch(format!(
                "expected exactly 1 input tensor, session has {}",
                inputs.len()
            )));
        };
        let input_ok = if self.input_layout_declared {
            dims_match(&self.expected_input_dims()?, actual_input)
        } else {
            // Layout unspecified (manifest predates `input_layout`): accept the logical shape in
            // either NCHW or NHWC order. Rank and every other dim are still enforced.
            dims_match(&self.input_shape, actual_input)
                || nhwc_dims(&self.input_shape).is_some_and(|nhwc| dims_match(&nhwc, actual_input))
        };
        if !input_ok {
            let layout = if self.input_layout_declared {
                format!("{:?}", self.input_layout)
            } else {
                "layout unspecified".to_string()
            };
            return Err(self.shape_mismatch(format!(
                "input: manifest declares logical {:?} ({layout}), session has {actual_input:?}",
                self.input_shape
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

/// Permutes a logical `[N, C, H, W]` shape to `[N, H, W, C]`; `None` unless rank 4.
fn nhwc_dims(logical: &[usize]) -> Option<Vec<usize>> {
    match logical {
        &[n, c, h, w] => Some(vec![n, h, w, c]),
        _ => None,
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

    /// Computes the hexadecimal lowercase SHA-256 digest of an in-memory buffer.
    pub fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Reads a model file once into memory and verifies its SHA-256 against this manifest.
    ///
    /// The returned bytes are exactly the bytes that were hashed, so a session built from them
    /// (`commit_from_memory`) runs the attested graph even if the file changes afterwards
    /// (no check-then-reopen window, GitHub #246). The read is bounded by
    /// [`MAX_MODEL_FILE_BYTES`].
    pub fn read_verified_model<P: AsRef<Path>>(
        &self,
        id: &str,
        file_path: P,
    ) -> Result<Vec<u8>, InferenceError> {
        self.read_verified_model_with_limit(id, file_path, MAX_MODEL_FILE_BYTES)
    }

    /// [`Self::read_verified_model`] with an explicit size bound in bytes.
    ///
    /// Errors: unknown id or a path that is not a regular file -> `ModelNotFound`; a file
    /// larger than `max_bytes` or any read failure -> `ModelIo`; a digest mismatch ->
    /// `ChecksumMismatch`. No bytes are returned on error.
    pub fn read_verified_model_with_limit<P: AsRef<Path>>(
        &self,
        id: &str,
        file_path: P,
        max_bytes: u64,
    ) -> Result<Vec<u8>, InferenceError> {
        let p = file_path.as_ref();
        let metadata = self
            .get_model(id)
            .ok_or_else(|| InferenceError::ModelNotFound {
                id: id.to_string(),
                path: p.to_path_buf(),
            })?;
        let not_found = || InferenceError::ModelNotFound {
            id: id.to_string(),
            path: p.to_path_buf(),
        };
        let io_error = |source: std::io::Error| InferenceError::ModelIo {
            id: id.to_string(),
            path: p.to_path_buf(),
            source,
        };

        let file = match File::open(p) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(not_found()),
            Err(e) => return Err(io_error(e)),
        };
        // Type and size are checked on the opened handle, not on the path.
        let file_meta = file.metadata().map_err(io_error)?;
        if !file_meta.is_file() {
            return Err(not_found());
        }
        let too_large = || {
            io_error(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("model file exceeds the {max_bytes}-byte attestation bound"),
            ))
        };
        if file_meta.len() > max_bytes {
            return Err(too_large());
        }

        // The file may grow after `metadata()`: read at most `max_bytes + 1` bytes.
        let capacity = usize::try_from(file_meta.len()).map_err(|_| too_large())?;
        let mut bytes = Vec::with_capacity(capacity);
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if u64::try_from(bytes.len()).map_or(true, |len| len > max_bytes) {
            return Err(too_large());
        }

        let computed = Self::sha256_hex(&bytes);
        if !computed.eq_ignore_ascii_case(&metadata.sha256) {
            return Err(InferenceError::ChecksumMismatch {
                id: id.to_string(),
                expected: metadata.sha256.clone(),
                actual: computed,
            });
        }
        Ok(bytes)
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

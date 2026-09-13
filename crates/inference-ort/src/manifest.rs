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

/// Metadata specification for a single machine learning model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub id: String,
    pub filename: String,
    pub sha256: String,
    pub license: String,
    pub source_url: String,
    pub description: String,
    pub input_shape: Vec<usize>,
    #[serde(default)]
    pub output_shapes: Vec<Vec<usize>>,
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

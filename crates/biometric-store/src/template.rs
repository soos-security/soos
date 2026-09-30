//! Biometric template schema, validation, and CBOR representation.

use crate::error::BiometricStoreError;
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::{Zeroize, Zeroizing};

/// Biometric template encapsulating user ID, model metadata, and zeroized embedding.
#[derive(Clone, PartialEq)]
pub struct BiometricTemplate {
    /// Linux User ID (UID) owning this template.
    pub uid: u32,
    /// Identifier of the model that generated the embedding (e.g. "mobilefacenet", "glintr100").
    pub model_id: String,
    /// Semantic version of the biometric model.
    pub model_version: String,
    /// Unix timestamp (seconds) when enrollment was completed.
    pub enrollment_timestamp: u64,
    /// Dimension of the vector (e.g. 128, 512).
    pub embedding_dim: usize,
    /// The biometric feature vector, automatically zeroized when dropped from memory.
    pub embedding: Zeroizing<Vec<f32>>,
}

impl BiometricTemplate {
    /// Creates a validated `BiometricTemplate`.
    ///
    /// # Errors
    /// Returns `BiometricStoreError::InvalidMetadata` if:
    /// - `model_id` or `model_version` is empty or only whitespace.
    /// - `embedding` is empty.
    /// - Any float in `embedding` is NaN or Infinite.
    pub fn new(
        uid: u32,
        model_id: String,
        model_version: String,
        enrollment_timestamp: u64,
        embedding: Zeroizing<Vec<f32>>,
    ) -> Result<Self, BiometricStoreError> {
        if model_id.trim().is_empty() {
            return Err(BiometricStoreError::InvalidMetadata(
                "model_id cannot be empty or whitespace".to_string(),
            ));
        }
        if model_version.trim().is_empty() {
            return Err(BiometricStoreError::InvalidMetadata(
                "model_version cannot be empty or whitespace".to_string(),
            ));
        }
        if embedding.is_empty() {
            return Err(BiometricStoreError::InvalidMetadata(
                "embedding vector cannot be empty".to_string(),
            ));
        }
        if embedding.iter().any(|val| !val.is_finite()) {
            return Err(BiometricStoreError::InvalidMetadata(
                "embedding contains non-finite float (NaN or Inf)".to_string(),
            ));
        }

        let embedding_dim = embedding.len();

        Ok(Self {
            uid,
            model_id,
            model_version,
            enrollment_timestamp,
            embedding_dim,
            embedding,
        })
    }

    /// Serializes the template into canonical CBOR binary format.
    ///
    /// The CBOR bytes contain the embedding in plaintext, so they are returned in a
    /// [`Zeroizing`] buffer. The buffer is reserved up front so that serialization does not
    /// reallocate (a reallocation would leave an unscrubbed copy in freed memory), and the
    /// temporary copy of the embedding held by the wire struct is zeroized before returning.
    pub fn to_cbor(&self) -> Result<Zeroizing<Vec<u8>>, BiometricStoreError> {
        let mut wire = TemplateWire {
            uid: self.uid,
            model_id: self.model_id.clone(),
            model_version: self.model_version.clone(),
            enrollment_timestamp: self.enrollment_timestamp,
            embedding_dim: self.embedding_dim,
            embedding: (*self.embedding).clone(),
        };

        let mut bytes = Zeroizing::new(Vec::new());
        let reserve = self
            .embedding
            .len()
            .checked_mul(CBOR_MAX_BYTES_PER_F32)
            .and_then(|n| n.checked_add(CBOR_METADATA_OVERHEAD))
            .and_then(|n| n.checked_add(self.model_id.len()))
            .and_then(|n| n.checked_add(self.model_version.len()));
        let reserved = match reserve {
            Some(n) => bytes.try_reserve_exact(n).map_err(|e| {
                BiometricStoreError::Serialization(format!("CBOR buffer allocation failed: {e}"))
            }),
            None => Err(BiometricStoreError::Serialization(
                "CBOR buffer size overflow".to_string(),
            )),
        };
        let written = reserved.and_then(|()| {
            ciborium::into_writer(&wire, &mut *bytes)
                .map_err(|e| BiometricStoreError::Serialization(e.to_string()))
        });
        wire.embedding.zeroize();
        written?;
        Ok(bytes)
    }

    /// Deserializes and validates a template from CBOR binary bytes.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, BiometricStoreError> {
        let wire: TemplateWire = ciborium::from_reader(bytes)
            .map_err(|e| BiometricStoreError::Serialization(e.to_string()))?;

        if wire.embedding.len() != wire.embedding_dim {
            return Err(BiometricStoreError::InvalidMetadata(format!(
                "Declared dimension {} does not match payload length {}",
                wire.embedding_dim,
                wire.embedding.len()
            )));
        }

        Self::new(
            wire.uid,
            wire.model_id,
            wire.model_version,
            wire.enrollment_timestamp,
            Zeroizing::new(wire.embedding),
        )
    }
}

impl fmt::Debug for BiometricTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BiometricTemplate")
            .field("uid", &self.uid)
            .field("model_id", &self.model_id)
            .field("model_version", &self.model_version)
            .field("enrollment_timestamp", &self.enrollment_timestamp)
            .field("embedding_dim", &self.embedding_dim)
            .field(
                "embedding",
                &format_args!("<redacted, dim: {}>", self.embedding_dim),
            )
            .finish()
    }
}

/// Upper bound of the CBOR encoding of one `f32` array element (a major-type-7 float is at
/// most 1 header byte + 8 payload bytes).
const CBOR_MAX_BYTES_PER_F32: usize = 9;

/// Upper bound of the CBOR encoding of the fixed fields, map keys and length headers.
const CBOR_METADATA_OVERHEAD: usize = 256;

/// Internal wire format for CBOR serialization.
#[derive(Serialize, Deserialize)]
struct TemplateWire {
    uid: u32,
    model_id: String,
    model_version: String,
    enrollment_timestamp: u64,
    embedding_dim: usize,
    embedding: Vec<f32>,
}

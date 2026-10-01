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
    /// Identifier of the model that generated the embedding (e.g. "sface_2021dec").
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

/// Template metadata read without materialising the embedding vector (GitHub #235, STO-19).
///
/// Produced by [`crate::BiometricStore::get_metadata`] for listing purposes (`soos-enroll
/// list`, the GUI profile refresh). It carries no biometric data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateMetadata {
    /// Linux User ID (UID) owning the template.
    pub uid: u32,
    /// Identifier of the model that generated the embedding.
    pub model_id: String,
    /// Semantic version of the biometric model.
    pub model_version: String,
    /// Unix timestamp (seconds) when enrollment was completed.
    pub enrollment_timestamp: u64,
    /// Dimension of the stored embedding.
    pub embedding_dim: usize,
}

impl TemplateMetadata {
    /// Deserializes and validates template metadata from the CBOR form produced by
    /// [`BiometricTemplate::to_cbor`].
    ///
    /// The embedding array is walked element by element (each value must be a finite `f32`)
    /// and only counted, never stored. The same validation rules as
    /// [`BiometricTemplate::from_cbor`] apply: non-blank model identifiers, a non-empty
    /// embedding, and a declared dimension equal to the array length.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, BiometricStoreError> {
        let wire: MetadataWire = ciborium::from_reader(bytes)
            .map_err(|e| BiometricStoreError::Serialization(e.to_string()))?;

        let shape = wire.embedding;
        if shape.len != wire.embedding_dim {
            return Err(BiometricStoreError::InvalidMetadata(format!(
                "Declared dimension {} does not match payload length {}",
                wire.embedding_dim, shape.len
            )));
        }
        if wire.model_id.trim().is_empty() {
            return Err(BiometricStoreError::InvalidMetadata(
                "model_id cannot be empty or whitespace".to_string(),
            ));
        }
        if wire.model_version.trim().is_empty() {
            return Err(BiometricStoreError::InvalidMetadata(
                "model_version cannot be empty or whitespace".to_string(),
            ));
        }
        if shape.len == 0 {
            return Err(BiometricStoreError::InvalidMetadata(
                "embedding vector cannot be empty".to_string(),
            ));
        }
        if !shape.all_finite {
            return Err(BiometricStoreError::InvalidMetadata(
                "embedding contains non-finite float (NaN or Inf)".to_string(),
            ));
        }

        Ok(Self {
            uid: wire.uid,
            model_id: wire.model_id,
            model_version: wire.model_version,
            enrollment_timestamp: wire.enrollment_timestamp,
            embedding_dim: wire.embedding_dim,
        })
    }
}

/// Metadata view of [`TemplateWire`]: identical fields, embedding reduced to its shape.
#[derive(Deserialize)]
struct MetadataWire {
    uid: u32,
    model_id: String,
    model_version: String,
    enrollment_timestamp: u64,
    embedding_dim: usize,
    embedding: EmbeddingShape,
}

/// Length and finiteness of a serialized embedding array, computed without storing it.
struct EmbeddingShape {
    len: usize,
    all_finite: bool,
}

impl<'de> Deserialize<'de> for EmbeddingShape {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ShapeVisitor;

        impl<'de> serde::de::Visitor<'de> for ShapeVisitor {
            type Value = EmbeddingShape;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array of f32 values")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut shape = EmbeddingShape {
                    len: 0,
                    all_finite: true,
                };
                while let Some(value) = seq.next_element::<f32>()? {
                    shape.len = shape.len.saturating_add(1);
                    shape.all_finite &= value.is_finite();
                }
                Ok(shape)
            }
        }

        deserializer.deserialize_seq(ShapeVisitor)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Unit tests utilize direct assertions"
)]
mod metadata_tests {
    use super::{BiometricTemplate, TemplateMetadata, TemplateWire};
    use crate::error::BiometricStoreError;
    use zeroize::Zeroizing;

    fn wire_cbor(embedding_dim: usize, embedding: Vec<f32>, model_id: &str) -> Vec<u8> {
        let wire = TemplateWire {
            uid: 7,
            model_id: model_id.to_string(),
            model_version: "2.0.0".to_string(),
            enrollment_timestamp: 9,
            embedding_dim,
            embedding,
        };
        let mut out = Vec::new();
        ciborium::into_writer(&wire, &mut out).unwrap();
        out
    }

    #[test]
    fn test_metadata_matches_full_template_fields() {
        let t = BiometricTemplate::new(
            7,
            "arcface_w600k_mbf".to_string(),
            "2.0.0".to_string(),
            9,
            Zeroizing::new(vec![0.1f32; 512]),
        )
        .unwrap();
        let meta = TemplateMetadata::from_cbor(&t.to_cbor().unwrap()).unwrap();
        assert_eq!(meta.uid, t.uid);
        assert_eq!(meta.model_id, t.model_id);
        assert_eq!(meta.model_version, t.model_version);
        assert_eq!(meta.enrollment_timestamp, t.enrollment_timestamp);
        assert_eq!(meta.embedding_dim, t.embedding_dim);
    }

    #[test]
    fn test_metadata_refuses_dimension_mismatch() {
        let cbor = wire_cbor(512, vec![0.1f32; 4], "m");
        assert!(matches!(
            TemplateMetadata::from_cbor(&cbor),
            Err(BiometricStoreError::InvalidMetadata(_))
        ));
        assert!(matches!(
            BiometricTemplate::from_cbor(&cbor),
            Err(BiometricStoreError::InvalidMetadata(_))
        ));
    }

    #[test]
    fn test_metadata_refuses_empty_non_finite_and_blank_model() {
        for cbor in [
            wire_cbor(0, vec![], "m"),
            wire_cbor(2, vec![0.1, f32::NAN], "m"),
            wire_cbor(2, vec![0.1, 0.2], "  "),
        ] {
            assert!(matches!(
                TemplateMetadata::from_cbor(&cbor),
                Err(BiometricStoreError::InvalidMetadata(_))
            ));
            assert!(matches!(
                BiometricTemplate::from_cbor(&cbor),
                Err(BiometricStoreError::InvalidMetadata(_))
            ));
        }
    }
}

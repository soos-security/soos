use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ort::session::Session;

use crate::error::InferenceError;
use crate::manifest::ModelManifest;

/// Thread-safe, shared ONNX Runtime session handle as returned by
/// [`ModelRegistry::get_or_load_session`] and consumed by every detector constructor.
pub type SharedSession = Arc<Mutex<Session>>;

/// Configuration for ONNX Runtime sessions and model storage paths.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub models_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub intra_threads: usize,
    pub inter_threads: usize,
}

impl RegistryConfig {
    pub fn new<P: AsRef<Path>>(models_dir: P) -> Self {
        let dir = models_dir.as_ref().to_path_buf();
        let manifest = dir.join("manifest.toml");
        Self {
            models_dir: dir,
            manifest_path: manifest,
            intra_threads: 1,
            inter_threads: 1,
        }
    }

    pub fn with_manifest<P1: AsRef<Path>, P2: AsRef<Path>>(
        models_dir: P1,
        manifest_path: P2,
    ) -> Self {
        Self {
            models_dir: models_dir.as_ref().to_path_buf(),
            manifest_path: manifest_path.as_ref().to_path_buf(),
            intra_threads: 1,
            inter_threads: 1,
        }
    }
}

/// Attested model registry that validates model files against cryptographic manifests
/// prior to ONNX Runtime session initialization.
pub struct ModelRegistry {
    config: RegistryConfig,
    manifest: ModelManifest,
    sessions: HashMap<String, Arc<Mutex<Session>>>,
}

impl ModelRegistry {
    /// Initializes registry by loading the manifest.
    pub fn new(config: RegistryConfig) -> Result<Self, InferenceError> {
        let manifest = ModelManifest::from_file(&config.manifest_path)?;
        Ok(Self {
            config,
            manifest,
            sessions: HashMap::new(),
        })
    }

    /// Initializes registry directly with an in-memory parsed manifest.
    pub fn with_manifest(config: RegistryConfig, manifest: ModelManifest) -> Self {
        Self {
            config,
            manifest,
            sessions: HashMap::new(),
        }
    }

    /// Read-only access to the loaded manifest.
    pub fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }

    /// Read-only access to the configuration.
    pub fn config(&self) -> &RegistryConfig {
        &self.config
    }

    /// Verifies the SHA-256 integrity of all models registered in the manifest.
    pub fn verify_integrity(&self) -> Result<(), InferenceError> {
        self.manifest.verify_directory(&self.config.models_dir)
    }

    /// Resolves the absolute path for a registered model ID.
    pub fn resolve_model_path(&self, id: &str) -> Result<PathBuf, InferenceError> {
        let meta = self
            .manifest
            .get_model(id)
            .ok_or_else(|| InferenceError::ModelNotFound {
                id: id.to_string(),
                path: self.config.models_dir.clone(),
            })?;
        let path = self.config.models_dir.join(&meta.filename);
        if !path.is_file() {
            return Err(InferenceError::ModelNotFound {
                id: id.to_string(),
                path,
            });
        }
        Ok(path)
    }

    /// Validates model integrity and loads (or returns cached) ONNX Runtime session.
    ///
    /// Attestation is two-fold: the file SHA-256 must match the manifest before the session is
    /// built, and the session's I/O tensor shapes must match the manifest's `input_shape`,
    /// `input_layout` and `output_shapes` before it is cached and returned.
    pub fn get_or_load_session(&mut self, id: &str) -> Result<Arc<Mutex<Session>>, InferenceError> {
        if let Some(session) = self.sessions.get(id) {
            return Ok(Arc::clone(session));
        }

        let path = self.resolve_model_path(id)?;

        // Cryptographic attestation: verify SHA-256 before session instantiation
        self.manifest.verify_model_checksum(id, &path)?;

        let session = Session::builder()
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .with_intra_threads(self.config.intra_threads)
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .with_inter_threads(self.config.inter_threads)
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .commit_from_file(&path)
            .map_err(|e| InferenceError::Ort(format!("Failed loading session {}: {}", id, e)))?;

        // Shape attestation: the manifest's declared I/O shapes must match the graph
        // (symbolic dims are wildcards). A mismatch fails closed before the session is cached.
        self.validate_session_shapes(id, &session)?;

        let arc_session = Arc::new(Mutex::new(session));
        self.sessions
            .insert(id.to_string(), Arc::clone(&arc_session));
        Ok(arc_session)
    }

    /// Compares the session's input/output tensor shapes with the manifest entry for `id`.
    fn validate_session_shapes(&self, id: &str, session: &Session) -> Result<(), InferenceError> {
        let meta = self
            .manifest
            .get_model(id)
            .ok_or_else(|| InferenceError::ModelNotFound {
                id: id.to_string(),
                path: self.config.models_dir.clone(),
            })?;
        let shape_of = |name: &str, dtype: &ort::value::ValueType| {
            dtype
                .tensor_shape()
                .map(|shape| shape.to_vec())
                .ok_or_else(|| InferenceError::ModelShapeMismatch {
                    id: id.to_string(),
                    detail: format!("tensor '{name}' is not a dense tensor"),
                })
        };
        let inputs = session
            .inputs()
            .iter()
            .map(|input| shape_of(input.name(), input.dtype()))
            .collect::<Result<Vec<_>, _>>()?;
        let outputs = session
            .outputs()
            .iter()
            .map(|output| shape_of(output.name(), output.dtype()))
            .collect::<Result<Vec<_>, _>>()?;
        meta.validate_session_shapes(&inputs, &outputs)
    }
}

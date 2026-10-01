use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ort::session::Session;

use crate::error::InferenceError;
use crate::manifest::ModelManifest;

/// Set once the "manifest predates layout attestation" warning has been logged (one-time).
static LEGACY_LAYOUT_WARNED: AtomicBool = AtomicBool::new(false);

/// Thread-safe, shared ONNX Runtime session handle as returned by
/// [`ModelRegistry::get_or_load_session`] and consumed by every detector constructor.
pub type SharedSession = Arc<Mutex<Session>>;

/// Upper bound of the default ORT intra-op thread count (GitHub #252, review finding VIS-10).
pub const DEFAULT_MAX_INTRA_THREADS: usize = 4;

/// Hard upper bound of any configured ORT intra-op thread count.
pub const MAX_INTRA_THREADS: usize = 16;

/// Attested models whose ORT sessions log at `Error` level only (GitHub #278, owner decision
/// 2026-10-01).
///
/// The SFace 2021dec file is an IR 6 export that lists 174 initializers as graph inputs; ONNX
/// Runtime warns once per initializer ("Initializer ... appears in graph inputs") when the
/// session is created, which would flood the daemon journal at every start. Its file cannot be
/// re-exported (the SHA-256 is the attestation), so only that session's warnings are dropped;
/// its errors stay visible and every other model keeps the default `Warning` level.
pub const ERROR_ONLY_LOG_MODELS: [&str; 1] = [crate::embedding::SFACE_2021DEC.model_id];

/// ORT session log level of the attested model `id` (see [`ERROR_ONLY_LOG_MODELS`]).
#[must_use]
pub fn ort_session_log_level(id: &str) -> ort::logging::LogLevel {
    if ERROR_ONLY_LOG_MODELS.contains(&id) {
        ort::logging::LogLevel::Error
    } else {
        ort::logging::LogLevel::Warning
    }
}

/// Default ORT intra-op thread count: `min(DEFAULT_MAX_INTRA_THREADS, available_parallelism)`,
/// and 1 when the parallelism cannot be queried.
pub fn default_intra_threads() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .clamp(1, DEFAULT_MAX_INTRA_THREADS)
}

/// Configuration for ONNX Runtime sessions and model storage paths.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub models_dir: PathBuf,
    pub manifest_path: PathBuf,
    /// ORT intra-op threads per session (default [`default_intra_threads`]).
    pub intra_threads: usize,
    /// ORT inter-op threads per session (the graphs are sequential: 1).
    pub inter_threads: usize,
    /// Whether ORT worker threads may spin-wait between runs. Disabled by default so that
    /// the extra intra-op threads do not burn CPU while the daemon is idle.
    pub allow_spinning: bool,
}

impl RegistryConfig {
    pub fn new<P: AsRef<Path>>(models_dir: P) -> Self {
        let dir = models_dir.as_ref().to_path_buf();
        let manifest = dir.join("manifest.toml");
        Self::with_manifest(dir, manifest)
    }

    pub fn with_manifest<P1: AsRef<Path>, P2: AsRef<Path>>(
        models_dir: P1,
        manifest_path: P2,
    ) -> Self {
        Self {
            models_dir: models_dir.as_ref().to_path_buf(),
            manifest_path: manifest_path.as_ref().to_path_buf(),
            intra_threads: default_intra_threads(),
            inter_threads: 1,
            allow_spinning: false,
        }
    }

    /// Sets the ORT intra-op thread count, clamped to `1..=MAX_INTRA_THREADS`.
    #[must_use]
    pub fn with_intra_threads(mut self, intra_threads: usize) -> Self {
        self.intra_threads = intra_threads.clamp(1, MAX_INTRA_THREADS);
        self
    }
}

/// Attested model registry that validates model files against cryptographic manifests
/// prior to ONNX Runtime session initialization.
pub struct ModelRegistry {
    config: RegistryConfig,
    manifest: ModelManifest,
    sessions: HashMap<String, Arc<Mutex<Session>>>,
    /// Model bytes verified by [`Self::verify_integrity`] and not yet turned into a session.
    /// Each entry is consumed (removed) by the next [`Self::get_or_load_session`] for that id,
    /// so a model is hashed once and ORT loads exactly the hashed bytes (GitHub #246).
    verified_bytes: Mutex<HashMap<String, Vec<u8>>>,
}

impl ModelRegistry {
    /// Initializes registry by loading the manifest.
    pub fn new(config: RegistryConfig) -> Result<Self, InferenceError> {
        let manifest = ModelManifest::from_file(&config.manifest_path)?;
        Ok(Self {
            config,
            manifest,
            sessions: HashMap::new(),
            verified_bytes: Mutex::new(HashMap::new()),
        })
    }

    /// Initializes registry directly with an in-memory parsed manifest.
    pub fn with_manifest(config: RegistryConfig, manifest: ModelManifest) -> Self {
        Self {
            config,
            manifest,
            sessions: HashMap::new(),
            verified_bytes: Mutex::new(HashMap::new()),
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
    ///
    /// Each model file is read once into memory (bounded by
    /// [`crate::manifest::MAX_MODEL_FILE_BYTES`]) and hashed; the verified bytes are retained
    /// until [`Self::get_or_load_session`] consumes them, so the session is built from exactly
    /// the bytes attested here and the file is neither re-opened nor re-hashed. On any failure
    /// every retained buffer is dropped and nothing is cached.
    pub fn verify_integrity(&self) -> Result<(), InferenceError> {
        let mut verified = HashMap::with_capacity(self.manifest.models.len());
        let mut ids: Vec<&String> = self.manifest.models.keys().collect();
        ids.sort();
        for id in ids {
            let path = self.model_path(id)?;
            let bytes = self.manifest.read_verified_model(id, &path)?;
            verified.insert(id.clone(), bytes);
        }
        *self.lock_verified_bytes() = verified;
        Ok(())
    }

    /// Number of models whose verified bytes are retained, awaiting a session load.
    pub fn pending_verified_models(&self) -> usize {
        self.lock_verified_bytes().len()
    }

    /// The retained-bytes map is plain data: a poisoned lock still holds a consistent map.
    fn lock_verified_bytes(&self) -> MutexGuard<'_, HashMap<String, Vec<u8>>> {
        self.verified_bytes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Joins the manifest filename of `id` to the models directory (no filesystem access).
    fn model_path(&self, id: &str) -> Result<PathBuf, InferenceError> {
        let meta = self
            .manifest
            .get_model(id)
            .ok_or_else(|| InferenceError::ModelNotFound {
                id: id.to_string(),
                path: self.config.models_dir.clone(),
            })?;
        Ok(self.config.models_dir.join(&meta.filename))
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
    /// Attestation is two-fold: the SHA-256 of the model bytes must match the manifest before
    /// the session is built, and the session's I/O tensor shapes must match the manifest's
    /// `input_shape`, `input_layout` and `output_shapes` before it is cached and returned.
    ///
    /// The session is built with `commit_from_memory` from the very bytes that were hashed:
    /// those retained by a prior [`Self::verify_integrity`], otherwise a single bounded read of
    /// the file (GitHub #246). The file is never re-opened by path after hashing.
    pub fn get_or_load_session(&mut self, id: &str) -> Result<Arc<Mutex<Session>>, InferenceError> {
        if let Some(session) = self.sessions.get(id) {
            return Ok(Arc::clone(session));
        }

        // Cryptographic attestation: the bytes handed to ORT are the bytes that were hashed.
        let retained = self.lock_verified_bytes().remove(id);
        let bytes = match retained {
            Some(bytes) => bytes,
            None => {
                let path = self.model_path(id)?;
                self.manifest.read_verified_model(id, &path)?
            }
        };

        let session = Session::builder()
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .with_log_level(ort_session_log_level(id))
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .with_intra_threads(self.config.intra_threads)
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .with_inter_threads(self.config.inter_threads)
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .with_intra_op_spinning(self.config.allow_spinning)
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .with_inter_op_spinning(self.config.allow_spinning)
            .map_err(|e| InferenceError::Ort(e.to_string()))?
            .commit_from_memory(&bytes)
            .map_err(|e| InferenceError::Ort(format!("Failed loading session {}: {}", id, e)))?;
        // ORT keeps its own copy of the graph; release the attested buffer immediately.
        drop(bytes);

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
        if !meta.input_layout_declared && !LEGACY_LAYOUT_WARNED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                model_id = id,
                "Model manifest predates input layout attestation (no input_layout); \
                 input layout is not asserted, SHA-256 and dims are still enforced. \
                 Reinstall the manifest shipped with this release."
            );
        }
        meta.validate_session_shapes(&inputs, &outputs)
    }
}

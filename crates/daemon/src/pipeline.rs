//! Full pipeline subsystem integrating camera, neural vision, biometric store,
//! evidence store, and authorization policy engine.

use std::sync::Arc;
use tokio::sync::RwLock;

use soos_biometric_store::BiometricStore;
use soos_camera_v4l::CameraManager;
use soos_evidence_store::EvidenceStore;
use soos_policy::AuthorizationEngine;
use soos_vision::VisionPipeline;

use crate::error::DaemonError;

/// Maximum allowed age for a captured camera frame before it is considered stale (150ms).
pub const MAX_FRAME_AGE_NS: u64 = 150_000_000;

/// Total decision latency budget per PAM authentication request (900ms, within 1000ms PAM deadline).
pub const DECISION_BUDGET_MS: u64 = 900;

/// Polling interval of the multi-frame consensus loop between camera snapshot checks (10ms).
///
/// Short enough to pick up every new capture at 30 fps (33ms interval) so the `k` consecutive
/// passing captures required by `soos_policy::PadAggregator` are reached with minimal latency.
pub const FRAME_POLL_INTERVAL_MS: u64 = 10;

/// Attested manifest identifier of the embedding extractor loaded by the daemon.
///
/// Enrolled templates recorded with a different `model_id` are refused
/// (GitHub #182 / STO-09): their vectors live in another embedding space.
pub const EMBEDDING_MODEL_ID: &str = "arcface_w600k_mbf";

/// Attested manifest identifier of the Presentation Attack Detection model.
pub const PAD_MODEL_ID: &str = "minifasnet_v2_pad";

/// Historical `model_id` accepted as an alias of [`EMBEDDING_MODEL_ID`].
///
/// `soos-enroll enroll` recorded `mobilefacenet` / `1.0.0` by default until GitHub #182
/// although the vectors were produced by the ArcFace extractor. Accepted only together
/// with [`LEGACY_EMBEDDING_MODEL_ALIAS_VERSION`] (ADR 2026-09-30 "Legacy Embedding Model
/// Alias"); scheduled for removal once affected users have re-enrolled.
pub const LEGACY_EMBEDDING_MODEL_ALIAS_ID: &str = "mobilefacenet";

/// Exact `model_version` that must accompany [`LEGACY_EMBEDDING_MODEL_ALIAS_ID`].
pub const LEGACY_EMBEDDING_MODEL_ALIAS_VERSION: &str = "1.0.0";

/// Compatibility of an enrolled template with the loaded embedding model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateModelBinding {
    /// The template records the loaded embedding model.
    Current,
    /// The template records the historical CLI default aliasing the ArcFace extractor.
    LegacyAlias,
    /// The template belongs to another embedding space and must be refused.
    Foreign,
}

/// Classifies a template's recorded model against the loaded embedding model.
///
/// The legacy alias is honoured only when the loaded model is [`EMBEDDING_MODEL_ID`] and
/// both the id and the version match exactly (no trimming, no case folding).
#[must_use]
pub fn classify_template_model(
    loaded_model_id: &str,
    template_model_id: &str,
    template_model_version: &str,
) -> TemplateModelBinding {
    if template_model_id == loaded_model_id {
        TemplateModelBinding::Current
    } else if loaded_model_id == EMBEDDING_MODEL_ID
        && template_model_id == LEGACY_EMBEDDING_MODEL_ALIAS_ID
        && template_model_version == LEGACY_EMBEDDING_MODEL_ALIAS_VERSION
    {
        TemplateModelBinding::LegacyAlias
    } else {
        TemplateModelBinding::Foreign
    }
}

/// Composite runtime container holding all operational pipeline components.
pub struct PipelineComponents {
    /// Warm camera capture manager.
    pub camera: Arc<dyn CameraManager>,
    /// Neural vision preprocessing and verification orchestrator.
    pub vision: Arc<VisionPipeline>,
    /// Secure encrypted biometric template store.
    pub biometric_store: Arc<BiometricStore>,
    /// Anti-intrusion evidence snapshot store.
    pub evidence_store: Arc<EvidenceStore>,
    /// Thread-safe authorization decision engine with per-UID rate limiting.
    pub policy: Arc<RwLock<AuthorizationEngine>>,
}

impl PipelineComponents {
    /// Creates a new `PipelineComponents` container wrapping active subsystem instances.
    pub fn new(
        camera: Arc<dyn CameraManager>,
        vision: Arc<VisionPipeline>,
        biometric_store: Arc<BiometricStore>,
        evidence_store: Arc<EvidenceStore>,
        policy: Arc<RwLock<AuthorizationEngine>>,
    ) -> Self {
        Self {
            camera,
            vision,
            biometric_store,
            evidence_store,
            policy,
        }
    }
}

impl std::fmt::Debug for PipelineComponents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineComponents")
            .field("camera", &"<dyn CameraManager>")
            .field("vision", &"<VisionPipeline>")
            .field("biometric_store", &"<BiometricStore>")
            .field("evidence_store", &"<EvidenceStore>")
            .field("policy", &"<AuthorizationEngine>")
            .finish()
    }
}

/// Retrieves the current monotonic timestamp in nanoseconds safely without `unsafe`.
///
/// Uses kernel `CLOCK_MONOTONIC` via `nix::time::clock_gettime`.
///
/// # Errors
///
/// Returns [`DaemonError::Clock`] if `clock_gettime` fails or reports negative values.
pub fn current_monotonic_nanos() -> Result<u64, DaemonError> {
    current_monotonic_nanos_from_clock(nix::time::ClockId::CLOCK_MONOTONIC)
}

/// Retrieves monotonic nanoseconds from the specified POSIX clock identifier.
///
/// # Errors
///
/// Returns [`DaemonError::Clock`] if `clock_gettime` fails or returns negative components.
pub fn current_monotonic_nanos_from_clock(
    clock_id: nix::time::ClockId,
) -> Result<u64, DaemonError> {
    match nix::time::clock_gettime(clock_id) {
        Ok(ts) => {
            let sec = ts.tv_sec();
            let nsec = ts.tv_nsec();
            if sec >= 0 && nsec >= 0 {
                let sec_ns = u64::try_from(sec)
                    .map_err(|e| DaemonError::Clock(format!("Invalid clock seconds {sec}: {e}")))?
                    .saturating_mul(1_000_000_000);
                let nsec_u64 = u64::try_from(nsec).map_err(|e| {
                    DaemonError::Clock(format!("Invalid clock nanoseconds {nsec}: {e}"))
                })?;
                Ok(sec_ns.saturating_add(nsec_u64))
            } else {
                Err(DaemonError::Clock(format!(
                    "Negative clock timestamp returned: sec={sec}, nsec={nsec}"
                )))
            }
        }
        Err(err) => Err(DaemonError::Clock(format!("clock_gettime failed: {err}"))),
    }
}

/// Builds the daemon's production Presentation Attack Detector.
///
/// Sole PAD construction site of the daemon. The live class index is never overridden here:
/// `soos_inference_ort::pad::DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` is the single source of
/// truth shared with `soos-enroll` and `soos-gui` (GitHub #146, enforced by the
/// `test_no_pad_live_class_index_override_outside_tests` invariant).
pub fn build_pad_detector(
    pad_session: soos_inference_ort::SharedSession,
    pad_threshold: f32,
) -> soos_inference_ort::OrtPadDetector {
    soos_inference_ort::OrtPadDetector::new(pad_session, pad_threshold)
}

/// Startup validation of the daemon's PAD detector (GitHub #214, PAD-09).
///
/// Derives the expected class count from the manifest `output_shapes` of [`PAD_MODEL_ID`],
/// runs [`soos_inference_ort::OrtPadDetector::self_test`] on a fixed synthetic fixture and
/// logs the live class index and threshold at info level. Any mismatch (missing manifest
/// entry, wrong output length, out-of-range live class index) fails closed: the daemon does
/// not start rather than silently denying or accepting every presentation.
pub fn validate_pad_detector(
    pad: &soos_inference_ort::OrtPadDetector,
    manifest: &soos_inference_ort::ModelManifest,
) -> Result<soos_inference_ort::PadSelfTestReport, DaemonError> {
    let meta = manifest.get_model(PAD_MODEL_ID).ok_or_else(|| {
        soos_inference_ort::InferenceError::ModelNotFound {
            id: PAD_MODEL_ID.to_string(),
            path: std::path::PathBuf::from("manifest.toml"),
        }
    })?;
    let expected_classes =
        soos_inference_ort::pad::pad_class_count_from_manifest(&meta.output_shapes)?;
    let report = pad.self_test(expected_classes).inspect_err(|err| {
        tracing::error!(
            model_id = PAD_MODEL_ID,
            error = %err,
            "PAD startup self-test failed; refusing to start"
        );
    })?;
    tracing::info!(
        model_id = PAD_MODEL_ID,
        class_count = report.class_count,
        live_class_index = report.live_class_index,
        liveness_threshold = report.liveness_threshold,
        "PAD startup self-test passed"
    );
    Ok(report)
}

/// Returns the camera configuration with its device resolved by the single shared resolver
/// [`soos_camera_v4l::resolve_camera_device`] (GitHub #152), exactly like `soos-enroll` and
/// `soos-gui`: an explicit `camera_device` is honored verbatim, the auto sentinel triggers
/// sensor-preference auto-detection (stable by-id alias). The mock camera never enumerates.
///
/// This is the daemon's production resolution path ([`initialize_pipeline`] calls it with the
/// system enumerator); it is built on [`plan_camera_device`], so the parity tests and the
/// shipped code exercise the same function.
pub fn resolve_pipeline_camera(
    config: &crate::config::PipelineConfig,
    enumerator: &dyn soos_camera_v4l::CameraEnumerator,
) -> soos_camera_v4l::CameraConfig {
    if config.use_mock_camera {
        return config.camera.clone();
    }
    plan_camera_device(&config.camera, |preference| {
        auto_detect_with(preference, enumerator)
    })
}

/// Runs the shared resolver in auto mode and returns the selected path, if a node was found.
fn auto_detect_with(
    preference: soos_camera_v4l::SensorPreference,
    enumerator: &dyn soos_camera_v4l::CameraEnumerator,
) -> Option<std::path::PathBuf> {
    let resolution = soos_camera_v4l::resolve_camera_device(None, preference, enumerator);
    (resolution.source == soos_camera_v4l::CameraResolutionSource::AutoDetected)
        .then_some(resolution.path)
}

/// Sentinel `device_path` meaning "auto-select a camera matching `sensor_preference`".
///
/// Alias of the shared [`soos_camera_v4l::AUTO_CAMERA_DEVICE`]; any other configured path is
/// explicit.
pub const AUTO_SELECT_DEVICE_SENTINEL: &str = soos_camera_v4l::AUTO_CAMERA_DEVICE;

/// Returns whether the camera configuration requests automatic device selection.
fn is_auto_select(camera: &soos_camera_v4l::CameraConfig) -> bool {
    camera.explicit_device().is_none()
}

/// Enumerates capture devices through the shared resolver (GitHub #152) and returns the one
/// matching `preference`, if any, addressed by its persistent `/dev/v4l/by-id/...` link when
/// udev provides one (GitHub #151).
pub fn auto_select_camera_device(
    preference: soos_camera_v4l::SensorPreference,
) -> Option<std::path::PathBuf> {
    auto_detect_with(
        preference,
        &soos_camera_v4l::SystemCameraEnumerator::default(),
    )
}

/// Computes the startup camera configuration (GitHub #151).
///
/// - Auto-selection mode ([`AUTO_SELECT_DEVICE_SENTINEL`]): the device returned by
///   `auto_select` is used; when none is found the sentinel is kept and the supervisor keeps
///   re-resolving through [`camera_device_resolver`] until a camera appears.
/// - Explicit path: kept as-is even when it does not exist yet (by-id link not yet created by
///   udev at boot). It is never silently replaced by a different camera.
///
/// `auto_select` is the enumeration seam (production: [`auto_select_camera_device`]).
pub fn plan_camera_device<F>(
    camera: &soos_camera_v4l::CameraConfig,
    auto_select: F,
) -> soos_camera_v4l::CameraConfig
where
    F: Fn(soos_camera_v4l::SensorPreference) -> Option<std::path::PathBuf>,
{
    let mut camera_cfg = camera.clone();
    if is_auto_select(&camera_cfg) {
        match auto_select(camera_cfg.sensor_preference) {
            Some(selected) => camera_cfg.device_path = selected,
            None => {
                camera_cfg.device_path = std::path::PathBuf::from(AUTO_SELECT_DEVICE_SENTINEL);
                tracing::warn!(
                    preference = ?camera_cfg.sensor_preference,
                    "No capture device detected at startup; camera reported not ready until one appears"
                );
            }
        }
    }
    camera_cfg
}

/// Returns the device resolver the capture supervisor consults after device loss.
///
/// Only auto-selection mode re-enumerates; an explicitly configured device is retried as-is.
pub fn camera_device_resolver(
    camera: &soos_camera_v4l::CameraConfig,
) -> Option<Arc<dyn soos_camera_v4l::DevicePathResolver>> {
    if !is_auto_select(camera) {
        return None;
    }
    let preference = camera.sensor_preference;
    Some(Arc::new(move || auto_select_camera_device(preference)))
}

/// Model registry configuration derived from the pipeline configuration: models directory,
/// its `manifest.toml`, and the operator-configured ORT intra-op thread count (GitHub #252;
/// ORT spin-waiting stays disabled).
pub fn registry_config_for(
    config: &crate::config::PipelineConfig,
) -> soos_inference_ort::RegistryConfig {
    soos_inference_ort::RegistryConfig::new(&config.models_dir)
        .with_intra_threads(config.inference_intra_threads)
}

/// Initializes all production pipeline components from a strongly-typed [`PipelineConfig`].
///
/// This includes:
/// 1. Spawning the V4L2 camera capture supervisor (or instantiating the mock generator if configured).
/// 2. Loading or creating cryptographic master keys for biometrics and anti-intrusion evidence.
/// 3. Instantiating the encrypted biometric template store and evidence snapshot store.
/// 4. Configuring the authorization policy engine and per-UID rate limiter.
/// 5. Verifying the cryptographic SHA-256 integrity of all local ONNX models via `manifest.toml`.
/// 6. Creating isolated CPU ONNX Runtime sessions for face detection, landmarks, PAD, and feature extraction.
/// 7. Assembling the verified high-level [`VisionPipeline`].
///
/// Fails closed if any single component, key, or model cannot be verified or loaded.
pub fn initialize_pipeline(
    config: &crate::config::PipelineConfig,
) -> Result<PipelineComponents, crate::error::DaemonError> {
    // 1. Camera Manager
    // Single resolution path shared with the parity tests (GitHub #152); the mock camera
    // never enumerates hardware.
    let camera_cfg =
        resolve_pipeline_camera(config, &soos_camera_v4l::SystemCameraEnumerator::default());
    let camera: Arc<dyn CameraManager> = if config.use_mock_camera {
        tracing::info!("Initializing mock camera manager for simulation/testing");
        Arc::new(soos_camera_v4l::MockCameraManager::new(camera_cfg))
    } else {
        tracing::info!(
            device = %camera_cfg.device_path.display(),
            "Spawning production V4L2 camera manager"
        );
        match camera_device_resolver(&config.camera) {
            Some(resolver) => Arc::new(soos_camera_v4l::V4lCameraManager::spawn_with_resolver(
                camera_cfg, resolver,
            )?),
            None => Arc::new(soos_camera_v4l::V4lCameraManager::spawn(camera_cfg)?),
        }
    };

    // 2. Biometric Store & Master Key
    let bio_key = soos_biometric_store::MasterKey::load_or_create(&config.master_key_path)?;
    let biometric_store = Arc::new(soos_biometric_store::BiometricStore::new(
        &config.biometrics_dir,
        bio_key,
    )?);

    // 3. Evidence Store & Master Key
    let ev_key = soos_evidence_store::MasterKey::load_or_create(&config.evidence.key_path)?;
    let evidence_store = Arc::new(soos_evidence_store::EvidenceStore::new(
        config.evidence.clone(),
        ev_key,
    ));

    // 4. Policy Engine & Rate Limiter
    let rate_limiter = soos_policy::RateLimiter::new(config.rate_limit);
    let policy = Arc::new(RwLock::new(
        soos_policy::AuthorizationEngine::with_rate_limiter(config.thresholds, rate_limiter),
    ));

    // 5. Machine Learning Models & Vision Pipeline
    let reg_config = registry_config_for(config);
    let mut registry = soos_inference_ort::ModelRegistry::new(reg_config)?;

    // Cryptographic attestation: hash every model once; the verified in-memory bytes are what
    // get_or_load_session hands to ONNX Runtime (no re-open, no second hash, GitHub #246).
    registry.verify_integrity()?;

    let det_session = registry.get_or_load_session("scrfd_500m_kps")?;
    let pad_session = registry.get_or_load_session(PAD_MODEL_ID)?;
    let ext_session = registry.get_or_load_session(EMBEDDING_MODEL_ID)?;

    let detector = Arc::new(soos_inference_ort::OrtScrfdDetector::new(
        det_session,
        config.vision.min_face_confidence,
        config.vision.nms_iou_threshold,
    )?);
    let pad = build_pad_detector(pad_session, config.vision.pad_threshold);
    // PAD output contract self-test (GitHub #214): fail closed before serving any request.
    validate_pad_detector(&pad, registry.manifest())?;
    let pad = Arc::new(pad);
    let extractor = Arc::new(soos_inference_ort::OrtEmbeddingExtractor::new(ext_session));

    let vision = Arc::new(soos_vision::VisionPipeline::new(
        detector,
        pad,
        extractor,
        config.vision.clone(),
    ));

    Ok(PipelineComponents::new(
        camera,
        vision,
        biometric_store,
        evidence_store,
        policy,
    ))
}

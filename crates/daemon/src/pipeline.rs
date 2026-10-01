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

/// Attested manifest identifier of the embedding extractor loaded by the daemon
/// (`sface_2021dec`, derived from `soos_inference_ort::SHIPPED_EMBEDDING_MODEL`, GitHub #278).
///
/// Enrolled templates recorded with a different `model_id` are refused
/// (GitHub #182 / STO-09): their vectors live in another embedding space.
pub const EMBEDDING_MODEL_ID: &str = soos_inference_ort::SHIPPED_EMBEDDING_MODEL.model_id;

/// Attested manifest identifier of the Presentation Attack Detection model.
pub const PAD_MODEL_ID: &str = "minifasnet_v2_pad";

/// Manifest identifier of the optional second PAD ensemble member (GitHub #212, PAD-07):
/// upstream Silent-Face-Anti-Spoofing's 4.0-scale MiniFASNetV1SE.
///
/// The repository manifest does not attest this model yet, so the daemon is single-model by
/// default; the member is wired only when the deployed manifest declares this id
/// (ADR 2026-09-30 "Optional Manifest-Gated Second PAD Member").
pub const SECONDARY_PAD_MODEL_ID: &str = "minifasnet_v1se_pad";

/// Context crop scale of [`SECONDARY_PAD_MODEL_ID`] (upstream `4_0_0_80x80_MiniFASNetV1SE`).
pub const SECONDARY_PAD_BBOX_SCALE: f32 = 4.0;

/// Optional PAD ensemble members `(manifest id, crop scale)`, in fusion order.
const OPTIONAL_PAD_MEMBERS: [(&str, f32); 1] = [(SECONDARY_PAD_MODEL_ID, SECONDARY_PAD_BBOX_SCALE)];

/// Compatibility of an enrolled template with the loaded embedding model.
///
/// The legacy `mobilefacenet` / `1.0.0` alias of the retired ArcFace extractor was removed with
/// the SFace switch (owner decision 2026-10-01, GitHub #278): such templates are `Foreign`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateModelBinding {
    /// The template records the loaded embedding model.
    Current,
    /// The template belongs to another embedding space and must be refused.
    Foreign,
}

/// Classifies a template's recorded model id against the loaded embedding model: `Current`
/// only for the exact id (no alias, no trimming, no case folding). `template_model_version` is
/// ignored: the binding is by model id (and by dimension in [`classify_template`]).
#[must_use]
pub fn classify_template_model(
    loaded_model_id: &str,
    template_model_id: &str,
    template_model_version: &str,
) -> TemplateModelBinding {
    let _ = template_model_version;
    if template_model_id == loaded_model_id {
        TemplateModelBinding::Current
    } else {
        TemplateModelBinding::Foreign
    }
}

/// Classifies a template by model id **and** vector length (GitHub #278): `Foreign` when the
/// ids differ or when the loaded extractor reports a dimension the template does not have.
#[must_use]
pub fn classify_template(
    loaded_model_id: &str,
    loaded_dimension: Option<usize>,
    template_model_id: &str,
    template_dimension: usize,
) -> TemplateModelBinding {
    if soos_inference_ort::template_matches_model(
        loaded_model_id,
        loaded_dimension,
        template_model_id,
        template_dimension,
    ) {
        TemplateModelBinding::Current
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
    let meta = pad_model_metadata(manifest, PAD_MODEL_ID)?;
    let expected_classes =
        soos_inference_ort::pad::pad_class_count_from_manifest(&meta.output_shapes)?;
    let report = pad
        .self_test(expected_classes)
        .inspect_err(|err| log_pad_self_test_failure(PAD_MODEL_ID, err))?;
    log_pad_self_test_success(PAD_MODEL_ID, &report);
    Ok(report)
}

/// [`validate_pad_detector`] for the PAD model attested under `model_id` (primary or optional
/// ensemble member, GitHub #212). Same fail-closed contract.
pub fn validate_pad_detector_for(
    pad: &soos_inference_ort::OrtPadDetector,
    manifest: &soos_inference_ort::ModelManifest,
    model_id: &str,
) -> Result<soos_inference_ort::PadSelfTestReport, DaemonError> {
    let meta = pad_model_metadata(manifest, model_id)?;
    let expected_classes =
        soos_inference_ort::pad::pad_class_count_from_manifest(&meta.output_shapes)?;
    let report = pad
        .self_test(expected_classes)
        .inspect_err(|err| log_pad_self_test_failure(model_id, err))?;
    log_pad_self_test_success(model_id, &report);
    Ok(report)
}

/// Manifest entry of the PAD model `model_id`; a missing entry fails closed.
fn pad_model_metadata<'a>(
    manifest: &'a soos_inference_ort::ModelManifest,
    model_id: &str,
) -> Result<&'a soos_inference_ort::ModelMetadata, DaemonError> {
    manifest.get_model(model_id).ok_or_else(|| {
        soos_inference_ort::InferenceError::ModelNotFound {
            id: model_id.to_string(),
            path: std::path::PathBuf::from("manifest.toml"),
        }
        .into()
    })
}

fn log_pad_self_test_failure(model_id: &str, err: &soos_inference_ort::InferenceError) {
    tracing::error!(
        model_id,
        error = %err,
        "PAD startup self-test failed; refusing to start"
    );
}

fn log_pad_self_test_success(model_id: &str, report: &soos_inference_ort::PadSelfTestReport) {
    tracing::info!(
        model_id,
        class_count = report.class_count,
        live_class_index = report.live_class_index,
        liveness_threshold = report.liveness_threshold,
        "PAD startup self-test passed"
    );
}

/// Optional PAD ensemble members attested by `manifest`, as `(manifest id, crop scale)`
/// (GitHub #212). Empty for the repository manifest: single-model PAD by default.
pub fn optional_pad_members(
    manifest: &soos_inference_ort::ModelManifest,
) -> Vec<(&'static str, f32)> {
    OPTIONAL_PAD_MEMBERS
        .iter()
        .copied()
        .filter(|(id, _)| manifest.get_model(id).is_some())
        .collect()
}

/// Adds every optional PAD ensemble member attested by the registry's manifest to `vision`
/// (GitHub #212, PAD-07).
///
/// Each member is loaded through the attested registry (SHA-256 and I/O shape checks), built by
/// [`build_pad_detector`] with `pad_threshold` (default live class index, never overridden),
/// self-tested by [`validate_pad_detector_for`] and added at its upstream crop scale. Any
/// failure is returned: the daemon refuses to start rather than silently dropping a member the
/// deployed manifest attests. Without an optional entry `vision` is returned unchanged.
pub fn attach_optional_pad_members(
    mut vision: VisionPipeline,
    registry: &mut soos_inference_ort::ModelRegistry,
    pad_threshold: f32,
) -> Result<VisionPipeline, DaemonError> {
    for (model_id, scale) in optional_pad_members(registry.manifest()) {
        let session = registry.get_or_load_session(model_id)?;
        let member = build_pad_detector(session, pad_threshold);
        validate_pad_detector_for(&member, registry.manifest(), model_id)?;
        vision = vision.with_additional_pad_model(scale, Arc::new(member))?;
        tracing::info!(
            model_id,
            bbox_scale = scale,
            "Optional PAD ensemble member attached"
        );
    }
    Ok(vision)
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

/// Largest synthetic frame side used by [`warm_up_vision_stages`] (1920 pixels).
///
/// Bounds the warm-up allocation (at most 1920 x 1920 x 3 bytes) whatever the configured
/// camera resolution; SCRFD letterboxes every frame to 640 x 640 anyway.
pub const MAX_WARMUP_DIMENSION: u32 = 1920;

/// Runs every vision inference stage once on blank (all-zero) synthetic inputs (GitHub #276).
///
/// A blank frame contains no face, so [`VisionPipeline::process_frame`] would stop after
/// detection; the stages are therefore called directly: the face detector on a
/// `frame_width x frame_height` RGB frame (each side clamped to `1..=`[`MAX_WARMUP_DIMENSION`]),
/// the PAD model on a blank crop of the configured PAD size and the embedding extractor on a
/// blank aligned crop of the configured recognition size. Stage results and errors are
/// discarded unseen: warm-up only measures latency and never influences a decision. The
/// inputs contain no camera data, so nothing sensitive is ever produced or logged.
pub fn warm_up_vision_stages(vision: &VisionPipeline, frame_width: u32, frame_height: u32) {
    let width = frame_width.clamp(1, MAX_WARMUP_DIMENSION);
    let height = frame_height.clamp(1, MAX_WARMUP_DIMENSION);
    let _ = vision
        .detector()
        .detect(&blank_rgb(width, height), width, height);

    let config = vision.config();
    let pad_width = config.pad_target_width.clamp(1, MAX_WARMUP_DIMENSION);
    let pad_height = config.pad_target_height.clamp(1, MAX_WARMUP_DIMENSION);
    let _ =
        vision
            .pad()
            .evaluate_liveness(&blank_rgb(pad_width, pad_height), pad_width, pad_height);

    let aligned_width = config.target_width.clamp(1, MAX_WARMUP_DIMENSION);
    let aligned_height = config.target_height.clamp(1, MAX_WARMUP_DIMENSION);
    let _ = vision.extractor().extract_embedding(
        &blank_rgb(aligned_width, aligned_height),
        aligned_width,
        aligned_height,
    );
}

/// Allocates a blank packed RGB888 buffer; both sides are already clamped by the caller.
fn blank_rgb(width: u32, height: u32) -> Vec<u8> {
    let len = usize::try_from(width)
        .unwrap_or(0)
        .saturating_mul(usize::try_from(height).unwrap_or(0))
        .saturating_mul(3);
    vec![0u8; len]
}

/// Builds the daemon's inference gate with its latency estimate seeded by a warm-up run of
/// every vision stage (GitHub #276, walkthrough 96 follow-up).
///
/// When the warm-up fails (a stage panicked), the gate keeps
/// [`crate::inference::DEFAULT_INFERENCE_ESTIMATE_MS`] and a warning is logged: warm-up is
/// best effort and never prevents the daemon from starting.
pub async fn warmed_inference_gate(
    vision: Arc<VisionPipeline>,
    frame_width: u32,
    frame_height: u32,
) -> crate::inference::InferenceGate {
    let gate = crate::inference::InferenceGate::default();
    match gate
        .warm_up(move || warm_up_vision_stages(&vision, frame_width, frame_height))
        .await
    {
        Ok(measured) => tracing::info!(
            measured_ms = measured.as_millis(),
            estimate_ms = gate.estimate().as_millis(),
            "Vision inference warm-up complete; latency estimate seeded"
        ),
        Err(err) => tracing::warn!(
            error = %err,
            estimate_ms = gate.estimate().as_millis(),
            "Vision inference warm-up failed; keeping the default latency estimate"
        ),
    }
    gate
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

/// Sweeps the temporary files an interrupted store write left behind (GitHub #291), once at
/// startup: a blocking evidence write abandoned at the previous shutdown is never awaited, and
/// an interrupted `soos-enroll` can leave a template temporary file.
///
/// Runs `BiometricStore::sweep_orphaned_temp_files` and
/// `EvidenceStore::sweep_orphaned_temp_files`, which remove only the stores' own temporary
/// file names (regular, single-link files owned by root, older than one minute, bounded per
/// call) and never follow a symlink. The sweep is housekeeping: a failure is logged and never
/// prevents the startup. Only counts are logged, never file content.
pub fn sweep_orphaned_store_temp_files(
    biometric: &soos_biometric_store::BiometricStore,
    evidence: &soos_evidence_store::EvidenceStore,
) {
    match biometric.sweep_orphaned_temp_files() {
        Ok(report) => log_temp_sweep(
            "templates",
            report.removed,
            report.kept_recent,
            report.limit_reached,
            report.lock_busy,
        ),
        Err(err) => {
            tracing::warn!(error = %err, store = "templates", "Orphaned temporary file sweep failed");
        }
    }
    match evidence.sweep_orphaned_temp_files() {
        Ok(report) => log_temp_sweep(
            "evidence",
            report.removed,
            report.kept_recent,
            report.limit_reached,
            report.lock_busy,
        ),
        Err(err) => {
            tracing::warn!(error = %err, store = "evidence", "Orphaned temporary file sweep failed");
        }
    }
}

/// Logs the counts of one store sweep (only when something happened).
fn log_temp_sweep(
    store: &'static str,
    removed: usize,
    kept_recent: usize,
    limit_reached: bool,
    lock_busy: bool,
) {
    if lock_busy {
        tracing::info!(
            store,
            "Store busy at startup; orphaned temporary file sweep skipped"
        );
    } else if removed > 0 || kept_recent > 0 || limit_reached {
        tracing::info!(
            store,
            removed,
            kept_recent,
            limit_reached,
            "Swept orphaned temporary files of an interrupted store write"
        );
    }
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
    let evidence_store = Arc::new(
        soos_evidence_store::EvidenceStore::new(config.evidence.clone(), ev_key)
            .with_daily_cap_total(config.evidence_daily_cap_total),
    );
    sweep_orphaned_store_temp_files(&biometric_store, &evidence_store);

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

    let vision = soos_vision::VisionPipeline::new(detector, pad, extractor, config.vision.clone());
    // Optional multi-scale PAD member (GitHub #212): only when the deployed manifest attests it.
    let vision = Arc::new(attach_optional_pad_members(
        vision,
        &mut registry,
        config.vision.pad_threshold,
    )?);

    Ok(PipelineComponents::new(
        camera,
        vision,
        biometric_store,
        evidence_store,
        policy,
    ))
}

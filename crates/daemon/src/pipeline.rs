//! Full pipeline subsystem integrating camera, neural vision, biometric store,
//! evidence store, and authorization policy engine.

use std::sync::Arc;
use tokio::sync::Mutex;

use soos_biometric_store::BiometricStore;
use soos_camera_v4l::CameraManager;
use soos_evidence_store::EvidenceStore;
use soos_policy::AuthorizationEngine;
use soos_vision::VisionPipeline;

/// Maximum allowed age for a captured camera frame before it is considered stale (150ms).
pub const MAX_FRAME_AGE_NS: u64 = 150_000_000;

/// Total decision latency budget per PAM authentication request (150ms).
pub const DECISION_BUDGET_MS: u64 = 150;

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
    pub policy: Arc<Mutex<AuthorizationEngine>>,
}

impl PipelineComponents {
    /// Creates a new `PipelineComponents` container wrapping active subsystem instances.
    pub fn new(
        camera: Arc<dyn CameraManager>,
        vision: Arc<VisionPipeline>,
        biometric_store: Arc<BiometricStore>,
        evidence_store: Arc<EvidenceStore>,
        policy: Arc<Mutex<AuthorizationEngine>>,
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
pub fn current_monotonic_nanos() -> u64 {
    match nix::time::clock_gettime(nix::time::ClockId::CLOCK_MONOTONIC) {
        Ok(ts) => {
            let sec = ts.tv_sec();
            let nsec = ts.tv_nsec();
            if sec >= 0 && nsec >= 0 {
                let sec_ns = u64::try_from(sec)
                    .unwrap_or(0)
                    .saturating_mul(1_000_000_000);
                let nsec_u64 = u64::try_from(nsec).unwrap_or(0);
                sec_ns.saturating_add(nsec_u64)
            } else {
                0
            }
        }
        Err(_) => 0,
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
    let camera: Arc<dyn CameraManager> = if config.use_mock_camera {
        tracing::info!("Initializing mock camera manager for simulation/testing");
        Arc::new(soos_camera_v4l::MockCameraManager::new(
            config.camera.clone(),
        ))
    } else {
        tracing::info!(
            device = %config.camera.device_path.display(),
            "Spawning production V4L2 camera manager"
        );
        Arc::new(soos_camera_v4l::V4lCameraManager::spawn(
            config.camera.clone(),
        )?)
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
    let policy = Arc::new(Mutex::new(
        soos_policy::AuthorizationEngine::with_rate_limiter(config.thresholds, rate_limiter),
    ));

    // 5. Machine Learning Models & Vision Pipeline
    let reg_config = soos_inference_ort::RegistryConfig::new(&config.models_dir);
    let mut registry = soos_inference_ort::ModelRegistry::new(reg_config)?;

    // Cryptographic attestation: verify all models in directory match manifest checksums
    registry.verify_integrity()?;

    let det_session = registry.get_or_load_session("ultraface_slim_320")?;
    let lmk_session = registry.get_or_load_session("landmark_5point")?;
    let pad_session = registry.get_or_load_session("minifasnet_pad")?;
    let ext_session = registry.get_or_load_session("mobilefacenet_arcface")?;

    let detector = Arc::new(soos_inference_ort::OrtFaceDetector::new(
        det_session,
        config.vision.min_face_confidence,
        0.45,
    ));
    let landmarks = Arc::new(soos_inference_ort::OrtLandmarkDetector::new(lmk_session));
    let pad = Arc::new(soos_inference_ort::OrtPadDetector::new(
        pad_session,
        config.vision.pad_threshold,
    ));
    let extractor = Arc::new(soos_inference_ort::OrtEmbeddingExtractor::new(ext_session));

    let vision = Arc::new(soos_vision::VisionPipeline::new(
        detector,
        landmarks,
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

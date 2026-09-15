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

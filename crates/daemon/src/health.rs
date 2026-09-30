//! Health check and component readiness subsystem.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use soos_camera_v4l::{CameraHealth, CameraManager};

/// Snapshot representation of daemon component health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthStatus {
    /// Whether the Unix domain socket is bound and accepting connections.
    pub socket_ready: bool,
    /// Whether the camera device is initialized and capturing frames.
    pub camera_ready: bool,
    /// Whether all local ONNX models have been verified against manifest checksums.
    pub models_verified: bool,
    /// Aggregated overall health (all components must be ready).
    pub is_healthy: bool,
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HealthStatus [is_healthy: {}, socket_ready: {}, camera_ready: {}, models_verified: {}]",
            self.is_healthy, self.socket_ready, self.camera_ready, self.models_verified
        )
    }
}

/// Internal health state tracking daemon components atomically.
///
/// Once a camera manager is attached with [`HealthState::attach_camera`], `camera_ready` is
/// derived live from its [`CameraHealth`] on every snapshot (GitHub #153): streaming and idle
/// auto-standby are ready; starting, recovering (missing/busy device) and dead (panicked
/// capture thread) are not. Before attachment the explicit flag is reported.
#[derive(Default)]
pub struct HealthState {
    socket_ready: AtomicBool,
    camera_ready: AtomicBool,
    models_verified: AtomicBool,
    camera: OnceLock<Arc<dyn CameraManager>>,
}

impl fmt::Debug for HealthState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HealthState")
            .field("socket_ready", &self.socket_ready)
            .field("camera_ready", &self.camera_ready)
            .field("models_verified", &self.models_verified)
            .field("camera", &self.camera_health())
            .finish()
    }
}

impl HealthState {
    /// Creates a new health state with all components initially unready.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the socket listener readiness status.
    pub fn set_socket_ready(&self, ready: bool) {
        self.socket_ready.store(ready, Ordering::Release);
    }

    /// Sets the camera readiness flag used while no camera manager is attached.
    pub fn set_camera_ready(&self, ready: bool) {
        self.camera_ready.store(ready, Ordering::Release);
    }

    /// Sets the ONNX model verification status.
    pub fn set_models_verified(&self, verified: bool) {
        self.models_verified.store(verified, Ordering::Release);
    }

    /// Attaches the live camera manager whose lifecycle state drives `camera_ready`.
    ///
    /// Only the first attachment is kept; later calls are ignored.
    pub fn attach_camera(&self, camera: Arc<dyn CameraManager>) {
        let _ = self.camera.set(camera);
    }

    /// Returns the lifecycle state of the attached camera, if any.
    pub fn camera_health(&self) -> Option<CameraHealth> {
        self.camera.get().map(|camera| camera.health())
    }

    /// Captures a point-in-time snapshot of the daemon component health.
    pub fn snapshot(&self) -> HealthStatus {
        let socket_ready = self.socket_ready.load(Ordering::Acquire);
        let camera_ready = match self.camera_health() {
            Some(state) => state.is_operational(),
            None => self.camera_ready.load(Ordering::Acquire),
        };
        let models_verified = self.models_verified.load(Ordering::Acquire);
        let is_healthy = socket_ready && camera_ready && models_verified;

        HealthStatus {
            socket_ready,
            camera_ready,
            models_verified,
            is_healthy,
        }
    }
}

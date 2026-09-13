//! Health check and component readiness subsystem.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

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
#[derive(Debug, Default)]
pub struct HealthState {
    socket_ready: AtomicBool,
    camera_ready: AtomicBool,
    models_verified: AtomicBool,
}

impl HealthState {
    /// Creates a new health state with all components initially unready.
    pub fn new() -> Self {
        Self {
            socket_ready: AtomicBool::new(false),
            camera_ready: AtomicBool::new(false),
            models_verified: AtomicBool::new(false),
        }
    }

    /// Sets the socket listener readiness status.
    pub fn set_socket_ready(&self, ready: bool) {
        self.socket_ready.store(ready, Ordering::Release);
    }

    /// Sets the camera capture manager readiness status.
    pub fn set_camera_ready(&self, ready: bool) {
        self.camera_ready.store(ready, Ordering::Release);
    }

    /// Sets the ONNX model verification status.
    pub fn set_models_verified(&self, verified: bool) {
        self.models_verified.store(verified, Ordering::Release);
    }

    /// Captures a point-in-time snapshot of the daemon component health.
    pub fn snapshot(&self) -> HealthStatus {
        let socket_ready = self.socket_ready.load(Ordering::Acquire);
        let camera_ready = self.camera_ready.load(Ordering::Acquire);
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

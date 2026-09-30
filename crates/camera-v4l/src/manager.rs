//! Abstract CameraManager trait contract.

use crate::frame::Frame;
use std::sync::Arc;

/// Lifecycle state of a camera capture manager, used by daemon health reporting.
///
/// Unlike [`CameraManager::is_ready`], which is `false` both during an intentional
/// auto-standby and during a hardware failure, this state distinguishes an idle but
/// healthy camera from a broken one (GitHub #153).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraHealth {
    /// The device is being opened or is discarding warmup frames.
    Starting,
    /// The device is streaming stabilized frames.
    Streaming,
    /// The device handle was released by the idle auto-standby; the next activity resumes it.
    Standby,
    /// The device is missing, busy or failing; the supervisor is backing off and retrying.
    Recovering,
    /// The capture supervisor thread terminated abnormally (panic); no frame will ever be served.
    Dead,
}

impl CameraHealth {
    /// Returns whether this state counts as a healthy camera for diagnostics.
    ///
    /// Streaming and auto-standby are healthy; starting, recovering and dead are not.
    pub fn is_operational(self) -> bool {
        matches!(self, Self::Streaming | Self::Standby)
    }

    /// Returns a stable lowercase label for logs and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Streaming => "streaming",
            Self::Standby => "standby",
            Self::Recovering => "recovering",
            Self::Dead => "dead",
        }
    }
}

impl std::fmt::Display for CameraHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Core interface for warm camera capture managers.
pub trait CameraManager: Send + Sync {
    /// Returns a lock-free reference to the latest captured and stabilized frame,
    /// or `None` if the camera is not ready, warming up, or disconnected.
    fn latest_frame(&self) -> Option<Arc<Frame>>;

    /// Returns whether the camera has completed warmup exposure stabilization
    /// and is actively streaming without hardware errors.
    fn is_ready(&self) -> bool;

    /// Signals an active authentication request or user presence, immediately
    /// restoring full FPS from idle power saving mode.
    fn notify_activity(&self);

    /// Requests graceful shutdown of background capture threads.
    fn stop(&self);

    /// Returns the lifecycle state of the capture manager.
    ///
    /// The default derives the state from [`CameraManager::is_ready`] only, which cannot tell
    /// standby from failure; managers owning a capture supervisor override it.
    fn health(&self) -> CameraHealth {
        if self.is_ready() {
            CameraHealth::Streaming
        } else {
            CameraHealth::Starting
        }
    }
}

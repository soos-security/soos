//! Abstract CameraManager trait contract.

use crate::frame::Frame;
use crate::status::CameraStatus;
use std::sync::Arc;

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

    /// Returns the user-presentable lifecycle state, including the classified last error.
    ///
    /// The default derives the state from [`CameraManager::is_ready`]; implementations that
    /// can observe failures override it to report [`CameraStatus::Error`].
    fn status(&self) -> CameraStatus {
        if self.is_ready() {
            CameraStatus::Ready
        } else {
            CameraStatus::Starting
        }
    }
}

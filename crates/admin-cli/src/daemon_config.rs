//! Camera settings of the `soos-daemon` configuration (`/etc/soos/daemon.toml`, GitHub #287).
//!
//! `soos-admin camera list` resolves with the same `[pipeline] camera_device` and
//! `[pipeline] sensor_preference` as the daemon. The file is read by the single shared reader
//! [`soos_camera_v4l::daemon_config`] (GitHub #289), also used by `soos-enroll` and `soos-gui`:
//! bounded by [`MAX_DAEMON_CONFIG_BYTES`], opened without blocking or following a symbolic link,
//! checked on the open handle.
//!
//! `camera list` keeps its documented contract for a key of the wrong type: soos-daemon refuses
//! to start with such a file, so the diagnostic reports it as [`DaemonConfigError::Malformed`]
//! and shows the defaults (matrix row CVF3). The per-key fallback of the shared reader applies
//! to `soos-enroll` and `soos-gui`.

use std::path::Path;

use soos_camera_v4l::daemon_config::read_daemon_camera_config;
pub use soos_camera_v4l::daemon_config::{
    DaemonCameraSettings, DaemonConfigError, DEFAULT_DAEMON_CONFIG_PATH, MAX_DAEMON_CONFIG_BYTES,
};

/// Reads `[pipeline] camera_device` and `[pipeline] sensor_preference` from `path`.
///
/// # Errors
///
/// Returns a [`DaemonConfigError`] when the file is missing, a symbolic link, not a regular
/// file, unreadable, larger than [`MAX_DAEMON_CONFIG_BYTES`] or malformed, including a camera
/// key of the wrong type (soos-daemon refuses to start with it).
pub fn read_daemon_camera_settings(path: &Path) -> Result<DaemonCameraSettings, DaemonConfigError> {
    let config = read_daemon_camera_config(path)?;
    if config.mistyped_keys.is_empty() {
        Ok(config.settings)
    } else {
        Err(DaemonConfigError::Malformed)
    }
}

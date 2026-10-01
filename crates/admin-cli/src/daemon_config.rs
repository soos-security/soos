//! Camera settings of the `soos-daemon` configuration (`/etc/soos/daemon.toml`, GitHub #287).
//!
//! `soos-admin camera list` resolves with the same `[pipeline] camera_device` and
//! `[pipeline] sensor_preference` as the daemon. The daemon's own loader lives in the
//! `soos-daemon` binary crate (Tokio, ONNX Runtime), which this non-biometric CLI must not link,
//! so only these two keys are read here, with the daemon's field types and the shared
//! vocabulary of `soos-camera-v4l` (`parse_sensor_preference`, `is_auto_camera_device`). The
//! read is bounded by [`MAX_DAEMON_CONFIG_BYTES`].

use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use soos_camera_v4l::{parse_sensor_preference, SensorPreference};
use thiserror::Error;

/// Path of the daemon configuration read by default (`DEFAULT_CONFIG_PATH` of `soos-daemon`).
pub const DEFAULT_DAEMON_CONFIG_PATH: &str = "/etc/soos/daemon.toml";

/// Largest configuration file read (1 MiB); a larger file is refused before parsing.
pub const MAX_DAEMON_CONFIG_BYTES: u64 = 1024 * 1024;

/// The camera keys of `[pipeline]`, as written in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DaemonCameraSettings {
    /// `camera_device`, verbatim (sentinels such as `auto` included).
    pub camera_device: Option<PathBuf>,
    /// `sensor_preference`, when it is part of the shared vocabulary.
    pub sensor_preference: Option<SensorPreference>,
    /// `true` when `sensor_preference` is present but not recognized (soos-daemon ignores it
    /// and keeps its default).
    pub sensor_preference_unrecognized: bool,
}

/// Why the configuration could not be used (the caller falls back to the daemon defaults).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DaemonConfigError {
    /// The file does not exist (soos-daemon then runs with its defaults too).
    #[error("not found")]
    NotFound,
    /// The path exists but is not a regular file.
    #[error("is not a regular file")]
    NotARegularFile,
    /// The file exists but cannot be read (for example permission denied).
    #[error("cannot be read ({0})")]
    Unreadable(std::io::ErrorKind),
    /// The file exceeds [`MAX_DAEMON_CONFIG_BYTES`].
    #[error("is larger than {limit} bytes")]
    TooLarge {
        /// The size limit in bytes.
        limit: u64,
    },
    /// Not valid UTF-8 TOML, or a camera key has the wrong type.
    #[error(
        "cannot be parsed as the soos-daemon configuration (soos-daemon refuses to start with it)"
    )]
    Malformed,
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    pipeline: Option<PipelineSection>,
}

#[derive(Debug, Deserialize)]
struct PipelineSection {
    camera_device: Option<PathBuf>,
    sensor_preference: Option<String>,
}

/// Reads `[pipeline] camera_device` and `[pipeline] sensor_preference` from `path`.
///
/// # Errors
///
/// Returns a [`DaemonConfigError`] when the file is missing, not a regular file, unreadable,
/// larger than [`MAX_DAEMON_CONFIG_BYTES`] or malformed.
pub fn read_daemon_camera_settings(path: &Path) -> Result<DaemonCameraSettings, DaemonConfigError> {
    let metadata = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DaemonConfigError::NotFound,
        kind => DaemonConfigError::Unreadable(kind),
    })?;
    if !metadata.is_file() {
        return Err(DaemonConfigError::NotARegularFile);
    }
    let file = std::fs::File::open(path).map_err(|e| DaemonConfigError::Unreadable(e.kind()))?;
    let mut bytes = Vec::new();
    file.take(MAX_DAEMON_CONFIG_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| DaemonConfigError::Unreadable(e.kind()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_DAEMON_CONFIG_BYTES {
        return Err(DaemonConfigError::TooLarge {
            limit: MAX_DAEMON_CONFIG_BYTES,
        });
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| DaemonConfigError::Malformed)?;
    let file: ConfigFile = toml::from_str(text).map_err(|_| DaemonConfigError::Malformed)?;

    let Some(pipeline) = file.pipeline else {
        return Ok(DaemonCameraSettings::default());
    };
    let sensor_preference = pipeline
        .sensor_preference
        .as_deref()
        .and_then(parse_sensor_preference);
    Ok(DaemonCameraSettings {
        camera_device: pipeline.camera_device,
        sensor_preference_unrecognized: pipeline.sensor_preference.is_some()
            && sensor_preference.is_none(),
        sensor_preference,
    })
}

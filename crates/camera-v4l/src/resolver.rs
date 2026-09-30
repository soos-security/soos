//! Single shared camera device resolver (GitHub #152, review finding CAM-04).
//!
//! `soos-daemon`, `soos-enroll` and `soos-gui` MUST resolve the camera through
//! [`resolve_camera_device`] so that enrollment and authentication always use the same sensor.
//!
//! Resolution order (identical in every binary):
//! 1. An explicit device path (not an auto sentinel, see [`is_auto_camera_device`]) is returned
//!    verbatim. Callers decide the precedence of their explicit sources (CLI flag before
//!    `camera_device` in `/etc/soos/daemon.toml`).
//! 2. Otherwise the enumerated V4L2 *capture* nodes are ranked with the [`SensorPreference`]
//!    (default `PreferIr`) by [`select_camera_device`]. The stable `/dev/v4l/by-id/` alias of the
//!    selected node is returned when one exists (Criterion C4), else the `/dev/videoN` node itself.
//!    Aliases are only ever matched against capture nodes, so a metadata node
//!    (`...-video-index1`) can never be selected.
//! 3. With no capture node, [`AUTO_CAMERA_DEVICE`] is returned (the capture supervisor then
//!    reports the device as missing and backs off).

use crate::sensor::{
    enumerate_capture_devices, select_camera_device, CameraDeviceInfo, SensorPreference, SensorType,
};
use std::path::{Path, PathBuf};

/// Auto-detection sentinel stored in `CameraConfig::device_path` when no explicit device is set.
pub const AUTO_CAMERA_DEVICE: &str = "/dev/v4l/by-id/default-camera";

/// Directory holding the persistent udev camera aliases.
pub const DEFAULT_BY_ID_DIR: &str = "/dev/v4l/by-id";

/// Upper bound on the number of `/dev/v4l/by-id/` entries inspected (bounded directory scan).
pub const MAX_BY_ID_ENTRIES: usize = 64;

/// Returns `true` when `path` means "auto-detect the camera".
///
/// Accepted sentinels (case-insensitive, surrounding whitespace ignored): `""`, `"auto"`,
/// `"default"`, and [`AUTO_CAMERA_DEVICE`].
pub fn is_auto_camera_device(path: &Path) -> bool {
    let Some(raw) = path.to_str() else {
        return false;
    };
    let value = raw.trim();
    value.is_empty()
        || value.eq_ignore_ascii_case("auto")
        || value.eq_ignore_ascii_case("default")
        || value == AUTO_CAMERA_DEVICE
}

/// Parses the shared `sensor_preference` vocabulary of `/etc/soos/daemon.toml`.
///
/// `prefer_ir`/`ir`, `prefer_rgb`/`rgb` and `any` (case-insensitive); anything else is `None`
/// and callers keep their default (`PreferIr`).
pub fn parse_sensor_preference(value: &str) -> Option<SensorPreference> {
    match value.trim().to_ascii_lowercase().as_str() {
        "prefer_ir" | "ir" => Some(SensorPreference::PreferIr),
        "prefer_rgb" | "rgb" => Some(SensorPreference::PreferRgb),
        "any" => Some(SensorPreference::Any),
        _ => None,
    }
}

/// Source of the camera inventory consulted by [`resolve_camera_device`].
///
/// Production code uses [`SystemCameraEnumerator`]; tests inject a hermetic fake.
pub trait CameraEnumerator {
    /// Returns the V4L2 video *capture* nodes with at least one supported pixel format.
    fn capture_devices(&self) -> Vec<CameraDeviceInfo>;

    /// Returns `(alias, canonical target)` pairs for the persistent by-id aliases.
    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)>;
}

/// Enumerator backed by `/sys/class/video4linux` and `/dev/v4l/by-id/`.
#[derive(Debug, Clone)]
pub struct SystemCameraEnumerator {
    by_id_dir: PathBuf,
}

impl Default for SystemCameraEnumerator {
    fn default() -> Self {
        Self {
            by_id_dir: PathBuf::from(DEFAULT_BY_ID_DIR),
        }
    }
}

impl SystemCameraEnumerator {
    /// Creates an enumerator reading the aliases from a custom directory (tests, chroots).
    pub fn with_by_id_dir<P: AsRef<Path>>(by_id_dir: P) -> Self {
        Self {
            by_id_dir: by_id_dir.as_ref().to_path_buf(),
        }
    }
}

impl CameraEnumerator for SystemCameraEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        enumerate_capture_devices()
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(&self.by_id_dir) else {
            return Vec::new();
        };
        let mut aliases: Vec<(PathBuf, PathBuf)> = entries
            .take(MAX_BY_ID_ENTRIES)
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let alias = entry.path();
                // Dangling aliases (unplugged device) fail to canonicalize and are skipped.
                let target = std::fs::canonicalize(&alias).ok()?;
                Some((alias, target))
            })
            .collect();
        aliases.sort();
        aliases
    }
}

/// How [`resolve_camera_device`] obtained its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraResolutionSource {
    /// An explicit, non-sentinel device path was supplied.
    Explicit,
    /// A capture node was auto-selected according to the sensor preference.
    AutoDetected,
    /// No capture node was found; the auto sentinel is returned.
    Fallback,
}

/// Result of a camera resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraResolution {
    /// Device path to open (stable by-id alias whenever available).
    pub path: PathBuf,
    /// How the path was obtained.
    pub source: CameraResolutionSource,
    /// Sensor classification of the auto-selected node (`None` unless auto-detected).
    pub sensor_type: Option<SensorType>,
}

/// Resolves the camera device path. See the module documentation for the exact order.
pub fn resolve_camera_device(
    explicit: Option<&Path>,
    preference: SensorPreference,
    enumerator: &dyn CameraEnumerator,
) -> CameraResolution {
    if let Some(path) = explicit.filter(|p| !is_auto_camera_device(p)) {
        return CameraResolution {
            path: path.to_path_buf(),
            source: CameraResolutionSource::Explicit,
            sensor_type: None,
        };
    }

    let devices = enumerator.capture_devices();
    let Some(selected) = select_camera_device(&devices, preference) else {
        return CameraResolution {
            path: PathBuf::from(AUTO_CAMERA_DEVICE),
            source: CameraResolutionSource::Fallback,
            sensor_type: None,
        };
    };

    let node = std::fs::canonicalize(&selected.path).unwrap_or_else(|_| selected.path.clone());
    let path = enumerator
        .by_id_aliases()
        .into_iter()
        .find(|(_, target)| *target == node || *target == selected.path)
        .map_or_else(|| selected.path.clone(), |(alias, _)| alias);

    let sensor_type = selected.sensor_type();
    tracing::info!(
        selected = %path.display(),
        node = %selected.path.display(),
        sensor_type = ?sensor_type,
        preference = ?preference,
        "Auto-selected camera device matching sensor preference"
    );

    CameraResolution {
        path,
        source: CameraResolutionSource::AutoDetected,
        sensor_type: Some(sensor_type),
    }
}

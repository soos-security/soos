//! Single shared camera device resolver (GitHub #152, review finding CAM-04).
//!
//! `soos-daemon`, `soos-enroll` and `soos-gui` MUST resolve the camera through
//! [`resolve_camera_device`] so that enrollment and authentication always use the same sensor.
//!
//! Resolution order (identical in every binary):
//! 1. An explicit device path (not an auto sentinel, see [`is_auto_camera_device`]) is returned
//!    verbatim. Callers decide the precedence of their explicit sources (CLI flag before
//!    `camera_device` in `/etc/soos/daemon.toml`).
//! 2. Otherwise the enumerated V4L2 *capture* nodes are classified once each (by-id name, card
//!    name, formats, frame sizes; a by-id name whose stem another capture node shares is not
//!    used, see [`by_id_stem`]) and ranked with the [`SensorPreference`] (default `PreferIr`)
//!    in the order of [`select_camera_device`]. The stable `/dev/v4l/by-id/` alias of the
//!    selected node is returned when one exists (Criterion C4), else the `/dev/videoN` node itself.
//!    Aliases are only ever matched against capture nodes, so a metadata node
//!    (`...-video-index1`) can never be selected.
//! 3. With no capture node, [`AUTO_CAMERA_DEVICE`] is returned (the capture supervisor then
//!    reports the device as missing and backs off).
//!
//! [`explain_camera_resolution`] returns the same decision with the classification rule of every
//! candidate and the [`SelectionReason`]; `soos-admin camera list` renders it (GitHub #256).

use crate::sensor::{
    enumerate_capture_devices, explain_sensor_classification, frame_sizes_at,
    select_camera_device_with, CameraDeviceInfo, ClassificationReason, SensorHints,
    SensorPreference, SensorType, MAX_FRAME_SIZE_HINTS,
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

    /// Returns the frame sizes (`width`, `height`) the node advertises, used as a sensor
    /// classification hint (GitHub #195). The default reports none (no hint).
    fn frame_sizes(&self, _device: &CameraDeviceInfo) -> Vec<(u32, u32)> {
        Vec::new()
    }
}

/// Returns the by-id alias of `device` among `aliases` (matched on the canonical node path or
/// the raw path), if any.
fn alias_for(device: &CameraDeviceInfo, aliases: &[(PathBuf, PathBuf)]) -> Option<PathBuf> {
    let node = std::fs::canonicalize(&device.path).unwrap_or_else(|_| device.path.clone());
    aliases
        .iter()
        .find(|(_, target)| *target == node || *target == device.path)
        .map(|(alias, _)| alias.clone())
}

/// Returns the by-id link name of `device`: the file name of its alias, or of its own path when
/// that path is itself a by-id link.
fn by_id_name_for(device: &CameraDeviceInfo, alias: Option<&Path>) -> Option<String> {
    alias
        .and_then(|alias| {
            alias
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .or_else(|| by_id_name_of_path(&device.path))
}

/// Suffix udev appends to every `/dev/v4l/by-id/` link (`-video-index<N>`).
const BY_ID_INDEX_SUFFIX: &str = "-video-index";

/// Returns the by-id link name without its trailing `-video-index<N>` suffix (`N` decimal).
///
/// udev builds the stem from the USB vendor, product and serial strings, so every interface of
/// one composite module shares it; names without that exact suffix are returned unchanged.
pub fn by_id_stem(name: &str) -> &str {
    let Some(pos) = name.rfind(BY_ID_INDEX_SUFFIX) else {
        return name;
    };
    let digits = pos
        .checked_add(BY_ID_INDEX_SUFFIX.len())
        .and_then(|start| name.get(start..))
        .unwrap_or_default();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return name;
    }
    name.get(..pos).unwrap_or(name)
}

/// Classification of one enumerated capture node, as used by the resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateClassification {
    /// The enumerated capture node.
    pub device: CameraDeviceInfo,
    /// Its persistent `/dev/v4l/by-id/` alias, when udev created one.
    pub by_id_alias: Option<PathBuf>,
    /// The hints the classifier received (by-id name, bounded frame sizes).
    pub hints: SensorHints,
    /// The resulting sensor type.
    pub sensor_type: SensorType,
    /// The scorer rule that decided `sensor_type`.
    pub reason: ClassificationReason,
    /// `true` when the by-id name was withheld from the classifier because another capture node
    /// shares its stem (composite RGB+IR module: the product string names both interfaces).
    pub by_id_hint_ignored: bool,
}

/// Why [`resolve_camera_device`] chose its answer (GitHub #256, CAM-17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionReason {
    /// An explicit, non-sentinel device path was supplied.
    ExplicitDevice,
    /// A capture node of the preferred sensor type exists (the first one is chosen).
    PreferredSensorMatched,
    /// No node of the preferred type; the first node of unknown type is chosen.
    FallbackUnknownSensor,
    /// No node of the preferred or unknown type; the first capture node is chosen.
    FallbackFirstCandidate,
    /// `SensorPreference::Any`: the first capture node is chosen.
    AnyPreferenceFirstCandidate,
    /// No capture node: the auto sentinel is returned.
    NoCaptureNode,
}

impl SelectionReason {
    /// Stable snake_case label used by `soos-admin camera` and in logs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitDevice => "explicit_device",
            Self::PreferredSensorMatched => "preferred_sensor_matched",
            Self::FallbackUnknownSensor => "fallback_unknown_sensor",
            Self::FallbackFirstCandidate => "fallback_first_candidate",
            Self::AnyPreferenceFirstCandidate => "any_preference_first_candidate",
            Self::NoCaptureNode => "no_capture_node",
        }
    }
}

/// A [`CameraResolution`] together with the classification of every candidate and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraResolutionReport {
    /// The decision, identical to [`resolve_camera_device`].
    pub resolution: CameraResolution,
    /// Every enumerated capture node in enumeration order (empty for an explicit device: the
    /// inventory is not consulted then).
    pub candidates: Vec<CandidateClassification>,
    /// Index of the selected node in `candidates` (`None` unless auto-detected).
    pub selected: Option<usize>,
    /// Why this answer was chosen.
    pub reason: SelectionReason,
}

/// Classifies every capture node once with its by-id name and frame sizes (GitHub #195).
///
/// A by-id name whose stem ([`by_id_stem`]) is shared by another capture node is withheld from
/// the classifier: on a composite module the USB product string names every interface, so an
/// `IR` token in it cannot tell the RGB node from the IR node.
fn classify_candidates(
    devices: Vec<CameraDeviceInfo>,
    aliases: &[(PathBuf, PathBuf)],
    enumerator: &dyn CameraEnumerator,
) -> Vec<CandidateClassification> {
    let named: Vec<(CameraDeviceInfo, Option<PathBuf>, Option<String>)> = devices
        .into_iter()
        .map(|device| {
            let alias = alias_for(&device, aliases);
            let name = by_id_name_for(&device, alias.as_deref());
            (device, alias, name)
        })
        .collect();
    let stem_count = |stem: &str| {
        named
            .iter()
            .filter_map(|(_, _, name)| name.as_deref())
            .filter(|other| by_id_stem(other) == stem)
            .count()
    };
    let shared: Vec<bool> = named
        .iter()
        .map(|(_, _, name)| {
            name.as_deref()
                .is_some_and(|name| stem_count(by_id_stem(name)) > 1)
        })
        .collect();

    named
        .into_iter()
        .zip(shared)
        .map(|((device, by_id_alias, name), by_id_hint_ignored)| {
            let mut frame_sizes = enumerator.frame_sizes(&device);
            frame_sizes.truncate(MAX_FRAME_SIZE_HINTS);
            let hints = SensorHints {
                by_id_name: if by_id_hint_ignored { None } else { name },
                frame_sizes,
            };
            let (sensor_type, reason) =
                explain_sensor_classification(&device.card_name, &device.supported_formats, &hints);
            CandidateClassification {
                device,
                by_id_alias,
                hints,
                sensor_type,
                reason,
                by_id_hint_ignored,
            }
        })
        .collect()
}

/// Selects among classified candidates with the single shared fallback order
/// ([`select_camera_device_with`]) and names the step that produced the choice.
fn select_candidate(
    candidates: &[CandidateClassification],
    preference: SensorPreference,
) -> (Option<usize>, SelectionReason) {
    let devices: Vec<CameraDeviceInfo> = candidates.iter().map(|c| c.device.clone()).collect();
    let classify = |device: &CameraDeviceInfo| {
        devices
            .iter()
            .position(|d| std::ptr::eq(d, device))
            .and_then(|i| candidates.get(i))
            .map_or(SensorType::Unknown, |c| c.sensor_type)
    };
    let Some(chosen) = select_camera_device_with(&devices, preference, classify)
        .and_then(|chosen| devices.iter().position(|d| std::ptr::eq(d, chosen)))
    else {
        return (None, SelectionReason::NoCaptureNode);
    };
    let sensor = candidates
        .get(chosen)
        .map_or(SensorType::Unknown, |c| c.sensor_type);
    let reason = match preference {
        SensorPreference::Any => SelectionReason::AnyPreferenceFirstCandidate,
        SensorPreference::PreferIr if sensor == SensorType::Infrared => {
            SelectionReason::PreferredSensorMatched
        }
        SensorPreference::PreferRgb if sensor == SensorType::Rgb => {
            SelectionReason::PreferredSensorMatched
        }
        _ if sensor == SensorType::Unknown => SelectionReason::FallbackUnknownSensor,
        _ => SelectionReason::FallbackFirstCandidate,
    };
    (Some(chosen), reason)
}

/// Resolves the camera device like [`resolve_camera_device`] and explains the decision: the
/// classification (with its rule) of every capture node and the [`SelectionReason`].
///
/// [`resolve_camera_device`] is implemented on top of this function, so both always agree.
/// It does not log; `soos-admin camera list` renders the report.
pub fn explain_camera_resolution(
    explicit: Option<&Path>,
    preference: SensorPreference,
    enumerator: &dyn CameraEnumerator,
) -> CameraResolutionReport {
    if let Some(path) = explicit.filter(|p| !is_auto_camera_device(p)) {
        return CameraResolutionReport {
            resolution: CameraResolution {
                path: path.to_path_buf(),
                source: CameraResolutionSource::Explicit,
                sensor_type: None,
            },
            candidates: Vec::new(),
            selected: None,
            reason: SelectionReason::ExplicitDevice,
        };
    }

    let aliases = enumerator.by_id_aliases();
    let candidates = classify_candidates(enumerator.capture_devices(), &aliases, enumerator);
    let (selected, reason) = select_candidate(&candidates, preference);
    let resolution = match selected.and_then(|i| candidates.get(i)) {
        Some(chosen) => CameraResolution {
            path: chosen
                .by_id_alias
                .clone()
                .unwrap_or_else(|| chosen.device.path.clone()),
            source: CameraResolutionSource::AutoDetected,
            sensor_type: Some(chosen.sensor_type),
        },
        None => CameraResolution {
            path: PathBuf::from(AUTO_CAMERA_DEVICE),
            source: CameraResolutionSource::Fallback,
            sensor_type: None,
        },
    };
    CameraResolutionReport {
        resolution,
        candidates,
        selected,
        reason,
    }
}

/// Returns the file name of `path` when it is itself a `/dev/v4l/by-id/` link.
pub(crate) fn by_id_name_of_path(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    if parent.file_name()? != "by-id" {
        return None;
    }
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
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

    fn frame_sizes(&self, device: &CameraDeviceInfo) -> Vec<(u32, u32)> {
        frame_sizes_at(&device.path)
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
    let report = explain_camera_resolution(explicit, preference, enumerator);
    if let Some(chosen) = report.selected.and_then(|i| report.candidates.get(i)) {
        tracing::info!(
            selected = %report.resolution.path.display(),
            node = %chosen.device.path.display(),
            sensor_type = ?chosen.sensor_type,
            classification = chosen.reason.as_str(),
            selection = report.reason.as_str(),
            preference = ?preference,
            "Auto-selected camera device matching sensor preference"
        );
        if chosen.reason == ClassificationReason::IrFrameSizeSignature {
            // The frame-size signature is the weakest IR signal (a low-resolution RGB webcam
            // matches it too); say so when it alone decided (candid review finding 6, #195).
            tracing::info!(
                node = %chosen.device.path.display(),
                "Camera classified Infrared by the frame-size signature only; set camera_device \
                 or sensor_preference in /etc/soos/daemon.toml if this is the wrong sensor"
            );
        }
    }
    report.resolution
}

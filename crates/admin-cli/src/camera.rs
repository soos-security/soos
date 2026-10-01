//! `soos-admin camera list|probe`: metadata-only camera diagnostics (GitHub #256, CAM-17).
//!
//! The reports render [`soos_camera_v4l::diagnostics`], which issues only `VIDIOC_QUERYCAP`,
//! `VIDIOC_ENUM_FMT` and `VIDIOC_ENUM_FRAMESIZES` and reports the decision of the shared camera
//! resolver used by `soos-daemon`, `soos-enroll` and `soos-gui`. No frame is ever captured, so
//! the command stays within the non-biometric scope of `soos-admin`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use soos_camera_v4l::diagnostics::{
    collect_camera_diagnostics, device_caps_flag_names, fourcc_label, probe_camera_node,
    sanitize_display_text, CameraDiagnostics, NodeDiagnostics, NodeStatus, V4lDeviceProbe,
};
use soos_camera_v4l::{
    is_auto_camera_device, CameraEnumerator, CameraResolutionSource, PixelFormat, SensorPreference,
    SensorType, SystemCameraEnumerator, DEFAULT_BY_ID_DIR, SYSFS_VIDEO4LINUX_DIR,
};

use crate::daemon_config::{read_daemon_camera_settings, DaemonCameraSettings, DaemonConfigError};

/// Directories the camera diagnostics read (overridable for tests and chroots).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraEnvironment {
    /// Sysfs directory listing the V4L2 nodes (`/sys/class/video4linux`).
    pub sysfs_dir: PathBuf,
    /// Directory holding the device nodes (`/dev`).
    pub dev_dir: PathBuf,
    /// Directory holding the persistent aliases (`/dev/v4l/by-id`).
    pub by_id_dir: PathBuf,
}

impl Default for CameraEnvironment {
    fn default() -> Self {
        Self {
            sysfs_dir: PathBuf::from(SYSFS_VIDEO4LINUX_DIR),
            dev_dir: PathBuf::from("/dev"),
            by_id_dir: PathBuf::from(DEFAULT_BY_ID_DIR),
        }
    }
}

impl CameraEnvironment {
    /// Reads the by-id aliases through the single bounded scanner of the shared resolver.
    fn aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        SystemCameraEnumerator::with_by_id_dir(&self.by_id_dir).by_id_aliases()
    }
}

/// Collects the `camera list` report for `env`.
pub fn collect_list_report(
    env: &CameraEnvironment,
    probe: &dyn V4lDeviceProbe,
    preference: SensorPreference,
    explicit: Option<&Path>,
) -> CameraListReport {
    let diagnostics = collect_camera_diagnostics(
        &env.sysfs_dir,
        &env.dev_dir,
        &env.aliases(),
        probe,
        preference,
        explicit,
    );
    CameraListReport::from_diagnostics(&diagnostics)
}

/// Where a `camera list` setting came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingOrigin {
    /// `--device` or `--sensor-preference`.
    CommandLine,
    /// `[pipeline]` of the daemon configuration.
    DaemonConfig,
    /// The soos-daemon default (auto-detection, `prefer_ir`).
    Default,
}

/// The device and sensor preference `camera list` resolves with, and where they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraListSettings {
    /// The daemon configuration that was consulted.
    pub config_path: PathBuf,
    /// What reading it produced.
    pub config: Result<DaemonCameraSettings, DaemonConfigError>,
    /// Explicit device handed to the resolver (`None`: auto-detection).
    pub explicit_device: Option<PathBuf>,
    /// Origin of [`Self::explicit_device`].
    pub device_origin: SettingOrigin,
    /// Sensor preference handed to the resolver.
    pub sensor_preference: SensorPreference,
    /// Origin of [`Self::sensor_preference`].
    pub preference_origin: SettingOrigin,
}

/// Combines the command-line overrides with the daemon configuration at `config_path`.
///
/// Precedence, for each setting: the flag, then `[pipeline]` of the daemon configuration, then
/// the soos-daemon default. A sentinel device (`auto`, `default`, empty,
/// `/dev/v4l/by-id/default-camera`) means auto-detection wherever it comes from, so
/// `--device auto` overrides a configured device. A missing, unreadable, oversized or malformed
/// configuration falls back to the defaults; [`CameraListSettings::note`] says so.
pub fn resolve_list_settings(
    cli_device: Option<&Path>,
    cli_preference: Option<SensorPreference>,
    config_path: &Path,
) -> CameraListSettings {
    let config = read_daemon_camera_settings(config_path);
    let loaded = config.as_ref().ok();

    let (explicit_device, device_origin) =
        match (cli_device, loaded.and_then(|c| c.camera_device.as_deref())) {
            (Some(device), _) => (Some(device), SettingOrigin::CommandLine),
            (None, Some(device)) => (Some(device), SettingOrigin::DaemonConfig),
            (None, None) => (None, SettingOrigin::Default),
        };
    let explicit_device = explicit_device
        .filter(|device| !is_auto_camera_device(device))
        .map(Path::to_path_buf);

    let (sensor_preference, preference_origin) =
        match (cli_preference, loaded.and_then(|c| c.sensor_preference)) {
            (Some(preference), _) => (preference, SettingOrigin::CommandLine),
            (None, Some(preference)) => (preference, SettingOrigin::DaemonConfig),
            (None, None) => (SensorPreference::default(), SettingOrigin::Default),
        };

    CameraListSettings {
        config_path: config_path.to_path_buf(),
        config,
        explicit_device,
        device_origin,
        sensor_preference,
        preference_origin,
    }
}

impl CameraListSettings {
    /// Human-readable note: which configuration was used and where each setting came from.
    #[must_use]
    pub fn note(&self) -> String {
        let mut out = String::new();
        let config = path_text(&self.config_path);
        match &self.config {
            Ok(_) => {
                let _ = writeln!(out, "camera settings: {config} (loaded)");
            }
            Err(err) => {
                let _ = writeln!(
                    out,
                    "camera settings: {config} {err}; using the soos-daemon defaults"
                );
            }
        }
        let device = self
            .explicit_device
            .as_deref()
            .map_or_else(|| "auto".to_string(), path_text);
        let _ = writeln!(
            out,
            "  camera_device:      {device} ({})",
            origin_label(self.device_origin, "--device")
        );
        let _ = writeln!(
            out,
            "  sensor_preference:  {} ({})",
            preference_label(self.sensor_preference),
            origin_label(self.preference_origin, "--sensor-preference")
        );
        let unrecognized = self
            .config
            .as_ref()
            .is_ok_and(|c| c.sensor_preference_unrecognized);
        if unrecognized && self.preference_origin != SettingOrigin::CommandLine {
            let _ = writeln!(
                out,
                "  note: sensor_preference in daemon.toml is not recognized; soos-daemon ignores \
                 it and uses {}",
                preference_label(SensorPreference::default())
            );
        }
        out
    }
}

const fn origin_label(origin: SettingOrigin, flag: &'static str) -> &'static str {
    match origin {
        SettingOrigin::CommandLine => flag,
        SettingOrigin::DaemonConfig => "daemon.toml",
        SettingOrigin::Default => "default",
    }
}

/// Collects the `camera probe <device>` report for `env`.
pub fn probe_report(
    env: &CameraEnvironment,
    probe: &dyn V4lDeviceProbe,
    device: &Path,
) -> CameraProbeReport {
    CameraProbeReport::from_node(&probe_camera_node(device, &env.aliases(), probe))
}

/// Label of a sensor preference, in the `daemon.toml` `sensor_preference` vocabulary.
const fn preference_label(preference: SensorPreference) -> &'static str {
    match preference {
        SensorPreference::PreferIr => "prefer_ir",
        SensorPreference::PreferRgb => "prefer_rgb",
        SensorPreference::Any => "any",
    }
}

const fn sensor_label(sensor: SensorType) -> &'static str {
    match sensor {
        SensorType::Rgb => "rgb",
        SensorType::Infrared => "infrared",
        SensorType::Unknown => "unknown",
    }
}

const fn source_label(source: CameraResolutionSource) -> &'static str {
    match source {
        CameraResolutionSource::Explicit => "explicit",
        CameraResolutionSource::AutoDetected => "auto_detected",
        CameraResolutionSource::Fallback => "fallback",
    }
}

/// Displayable path: control characters and bidirectional overrides replaced.
fn path_text(path: &Path) -> String {
    sanitize_display_text(&path.to_string_lossy())
}

/// One node of a report (JSON shape of `camera list` entries and of `camera probe`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeReport {
    pub node: String,
    pub status: &'static str,
    pub probe_error: Option<&'static str>,
    pub by_id: Option<String>,
    pub driver: Option<String>,
    pub card: Option<String>,
    pub bus_info: Option<String>,
    pub device_caps: Option<String>,
    pub device_caps_flags: Vec<&'static str>,
    pub fourccs: Vec<String>,
    pub formats: Vec<&'static str>,
    pub frame_sizes: Vec<String>,
    pub sensor_type: Option<&'static str>,
    pub classification_reason: Option<&'static str>,
    pub by_id_hint_ignored: bool,
}

impl NodeReport {
    fn from_node(node: &NodeDiagnostics) -> Self {
        let details = node.details.as_ref();
        let probe_error = match node.status {
            NodeStatus::ProbeFailed(failure) => Some(failure.as_str()),
            _ => None,
        };
        Self {
            node: path_text(&node.node),
            status: node.status.as_str(),
            probe_error,
            by_id: node.by_id_alias.as_deref().map(path_text),
            driver: details.map(|d| sanitize_display_text(&d.driver)),
            card: details.map(|d| sanitize_display_text(&d.card_name)),
            bus_info: details.map(|d| sanitize_display_text(&d.bus_info)),
            device_caps: details.map(|d| format!("{:#010x}", d.device_caps)),
            device_caps_flags: details
                .map(|d| device_caps_flag_names(d.device_caps))
                .unwrap_or_default(),
            fourccs: details
                .map(|d| d.fourccs.iter().map(|f| fourcc_label(*f)).collect())
                .unwrap_or_default(),
            formats: node
                .supported_formats
                .iter()
                .map(|f| PixelFormat::fourcc_str(*f))
                .collect(),
            frame_sizes: details
                .map(|d| {
                    d.frame_sizes
                        .iter()
                        .map(|(w, h)| format!("{w}x{h}"))
                        .collect()
                })
                .unwrap_or_default(),
            sensor_type: node.sensor_type.map(sensor_label),
            classification_reason: node.classification_reason.map(|r| r.as_str()),
            by_id_hint_ignored: node.by_id_hint_ignored,
        }
    }

    fn list_or_dash(items: &[String]) -> String {
        if items.is_empty() {
            "-".to_string()
        } else {
            items.join(", ")
        }
    }

    /// Appends the aligned text block of this node to `out`.
    fn write_table(&self, out: &mut String) {
        match self.probe_error {
            Some(error) => {
                let _ = writeln!(out, "{}  [{}: {error}]", self.node, self.status);
                return;
            }
            None => {
                let _ = writeln!(out, "{}  [{}]", self.node, self.status);
            }
        }
        let dash = || "-".to_string();
        let _ = writeln!(
            out,
            "  by-id:        {}",
            self.by_id.clone().unwrap_or_else(dash)
        );
        let _ = writeln!(
            out,
            "  driver:       {}",
            self.driver.clone().unwrap_or_else(dash)
        );
        let _ = writeln!(
            out,
            "  card:         {}",
            self.card.clone().unwrap_or_else(dash)
        );
        let _ = writeln!(
            out,
            "  bus_info:     {}",
            self.bus_info.clone().unwrap_or_else(dash)
        );
        let flags: Vec<String> = self
            .device_caps_flags
            .iter()
            .map(|f| (*f).to_string())
            .collect();
        let _ = writeln!(
            out,
            "  device_caps:  {} ({})",
            self.device_caps.clone().unwrap_or_else(dash),
            Self::list_or_dash(&flags)
        );
        let _ = writeln!(out, "  fourccs:      {}", Self::list_or_dash(&self.fourccs));
        let _ = writeln!(
            out,
            "  frame sizes:  {}",
            Self::list_or_dash(&self.frame_sizes)
        );
        if let (Some(sensor), Some(reason)) = (self.sensor_type, self.classification_reason) {
            let _ = writeln!(out, "  sensor:       {sensor} ({reason})");
        }
        if self.by_id_hint_ignored {
            out.push_str(
                "  note:         by-id IR token ignored (stem shared by several capture nodes)\n",
            );
        }
    }
}

/// Report of `soos-admin camera list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CameraListReport {
    pub preference: &'static str,
    pub selected: Option<String>,
    pub selected_node: Option<String>,
    pub sensor_type: Option<&'static str>,
    pub source: &'static str,
    pub reason: &'static str,
    pub nodes: Vec<NodeReport>,
}

impl CameraListReport {
    /// Builds the report from collected diagnostics.
    #[must_use]
    pub fn from_diagnostics(diagnostics: &CameraDiagnostics) -> Self {
        let has_device = diagnostics.resolution.source != CameraResolutionSource::Fallback;
        Self {
            preference: preference_label(diagnostics.preference),
            selected: has_device.then(|| path_text(&diagnostics.resolution.path)),
            selected_node: diagnostics.selected_node.as_deref().map(path_text),
            sensor_type: diagnostics.resolution.sensor_type.map(sensor_label),
            source: source_label(diagnostics.resolution.source),
            reason: diagnostics.reason.as_str(),
            nodes: diagnostics
                .nodes
                .iter()
                .map(NodeReport::from_node)
                .collect(),
        }
    }

    /// Process exit status: 0 when a device was chosen (auto-detected or explicit), 1 otherwise.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        i32::from(self.selected.is_none())
    }

    /// Pretty-printed JSON (produced by `serde_json`, so every string is escaped).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self)
            .unwrap_or_else(|_| "{\"error\": \"report serialization failed\"}".to_string())
    }

    /// Human-readable report: one block per node, then the selection.
    #[must_use]
    pub fn format_table(&self) -> String {
        let mut out = String::new();
        let candidates = self
            .nodes
            .iter()
            .filter(|n| n.status == NodeStatus::Candidate.as_str())
            .count();
        let _ = writeln!(
            out,
            "Camera nodes: {} scanned, {candidates} capture candidates\n",
            self.nodes.len()
        );
        for node in &self.nodes {
            node.write_table(&mut out);
            out.push('\n');
        }
        let none = || "none".to_string();
        let _ = writeln!(out, "Selection (sensor preference: {})", self.preference);
        let _ = writeln!(
            out,
            "  selected:     {}",
            self.selected.clone().unwrap_or_else(none)
        );
        let _ = writeln!(
            out,
            "  node:         {}",
            self.selected_node.clone().unwrap_or_else(none)
        );
        let _ = writeln!(
            out,
            "  sensor:       {}",
            self.sensor_type.unwrap_or("unknown")
        );
        let _ = writeln!(out, "  source:       {}", self.source);
        let _ = writeln!(out, "  reason:       {}", self.reason);
        out
    }
}

/// Report of `soos-admin camera probe <device>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CameraProbeReport {
    pub node: NodeReport,
}

impl CameraProbeReport {
    /// Builds the report from one probed node.
    #[must_use]
    pub fn from_node(node: &NodeDiagnostics) -> Self {
        Self {
            node: NodeReport::from_node(node),
        }
    }

    /// Process exit status: 0 only for a usable capture candidate.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        i32::from(self.node.status != NodeStatus::Candidate.as_str())
    }

    /// Pretty-printed JSON (produced by `serde_json`).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self)
            .unwrap_or_else(|_| "{\"error\": \"report serialization failed\"}".to_string())
    }

    /// Human-readable block of the node.
    #[must_use]
    pub fn format_table(&self) -> String {
        let mut out = String::new();
        self.node.write_table(&mut out);
        out
    }
}

//! Configuration structures for the soos daemon.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::error::DaemonError;

/// Configuration for the Unix domain socket listener.
#[derive(Debug, Clone)]
pub struct SocketConfig {
    /// Full path to the Unix domain socket.
    pub socket_path: PathBuf,
    /// Parent runtime directory (e.g. `/run/soos`).
    pub socket_dir: PathBuf,
    /// File permission mode for the socket (typically `0660`).
    pub socket_mode: u32,
    /// Whether to enforce that the parent directory is owned by root (`uid == 0`).
    pub enforce_root_owner: bool,
    /// Target system group for socket ownership (defaults to `"soos"`).
    pub socket_group: Option<String>,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/run/soos/daemon.sock"),
            socket_dir: PathBuf::from("/run/soos"),
            socket_mode: 0o660,
            enforce_root_owner: true,
            socket_group: Some("soos".to_string()),
        }
    }
}

/// Configuration for connection dispatching and bounded concurrency.
#[derive(Debug, Clone)]
pub struct DispatcherConfig {
    /// Maximum concurrent client connections accepted simultaneously.
    pub max_concurrent_connections: usize,
    /// Per-connection execution timeout deadline.
    pub connection_timeout: Duration,
    /// Whether to enforce that the target UID owns an active logind session.
    pub enforce_active_session: bool,
    /// Directory containing systemd logind runtime session state files.
    pub logind_sessions_dir: PathBuf,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_millis(250),
            enforce_active_session: true,
            logind_sessions_dir: PathBuf::from(crate::session::DEFAULT_LOGIND_SESSIONS_DIR),
        }
    }
}

/// Configuration for the full facial verification and storage pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Camera configuration parameters.
    pub camera: soos_camera_v4l::CameraConfig,
    /// Vision pipeline parameters.
    pub vision: soos_vision::VisionPipelineConfig,
    /// Storage directory containing verified machine learning models and manifest.
    pub models_dir: PathBuf,
    /// Storage directory for biometric templates.
    pub biometrics_dir: PathBuf,
    /// Master encryption key path for biometric templates.
    pub master_key_path: PathBuf,
    /// Evidence capture and retention configuration.
    pub evidence: soos_evidence_store::EvidenceConfig,
    /// Threshold configuration for authorization decisions.
    pub thresholds: soos_policy::ThresholdConfig,
    /// Rate limit configuration per UID.
    pub rate_limit: soos_policy::RateLimitConfig,
    /// Whether to force mock camera simulation rather than hardware device.
    pub use_mock_camera: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            camera: soos_camera_v4l::CameraConfig::default(),
            vision: soos_vision::VisionPipelineConfig::default(),
            models_dir: PathBuf::from("/var/lib/soos/models"),
            biometrics_dir: PathBuf::from(soos_biometric_store::DEFAULT_BIOMETRICS_DIR),
            master_key_path: PathBuf::from("/var/lib/soos/master.key"),
            evidence: soos_evidence_store::EvidenceConfig::default(),
            thresholds: soos_policy::ThresholdConfig::default(),
            rate_limit: soos_policy::RateLimitConfig::default(),
            use_mock_camera: false,
        }
    }
}

/// Global daemon configuration.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Socket listener configuration.
    pub socket: SocketConfig,
    /// Connection dispatcher configuration.
    pub dispatcher: DispatcherConfig,
    /// Pipeline configuration.
    pub pipeline: PipelineConfig,
    /// Logging filter directive (e.g. "info", "debug").
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket: SocketConfig::default(),
            dispatcher: DispatcherConfig::default(),
            pipeline: PipelineConfig::default(),
            log_level: "info".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DaemonConfigFile {
    #[serde(default)]
    socket: Option<SocketConfigFile>,
    #[serde(default)]
    dispatcher: Option<DispatcherConfigFile>,
    #[serde(default)]
    pipeline: Option<PipelineConfigFile>,
    #[serde(default)]
    log_level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SocketConfigFile {
    socket_path: Option<PathBuf>,
    socket_dir: Option<PathBuf>,
    socket_mode: Option<u32>,
    enforce_root_owner: Option<bool>,
    socket_group: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DispatcherConfigFile {
    max_concurrent_connections: Option<usize>,
    connection_timeout_ms: Option<u64>,
    enforce_active_session: Option<bool>,
    logind_sessions_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct PipelineConfigFile {
    camera_device: Option<PathBuf>,
    use_mock_camera: Option<bool>,
    models_dir: Option<PathBuf>,
    biometrics_dir: Option<PathBuf>,
    master_key_path: Option<PathBuf>,
    #[serde(default)]
    evidence: Option<EvidenceConfigFile>,
    #[serde(default)]
    thresholds: Option<ThresholdConfigFile>,
    #[serde(default)]
    rate_limit: Option<RateLimitConfigFile>,
}

#[derive(Debug, Deserialize)]
struct EvidenceConfigFile {
    enabled: Option<bool>,
    base_dir: Option<PathBuf>,
    key_path: Option<PathBuf>,
    retention_days: Option<u32>,
    daily_cap_per_uid: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ThresholdConfigFile {
    match_threshold: Option<f32>,
    pad_threshold: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct RateLimitConfigFile {
    max_attempts: Option<u32>,
    window_duration_secs: Option<u64>,
    max_tracked_uids: Option<usize>,
}

impl DaemonConfig {
    /// Parses a complete daemon configuration from a TOML string.
    pub fn from_toml_str(content: &str) -> Result<Self, DaemonError> {
        let file: DaemonConfigFile = toml::from_str(content)
            .map_err(|e| DaemonError::Config(format!("Failed to parse TOML configuration: {e}")))?;

        let mut config = Self::default();

        if let Some(log_level) = file.log_level {
            config.log_level = log_level;
        }

        if let Some(socket) = file.socket {
            if let Some(path) = socket.socket_path {
                config.socket.socket_path = path;
            }
            if let Some(dir) = socket.socket_dir {
                config.socket.socket_dir = dir;
            }
            if let Some(mode) = socket.socket_mode {
                config.socket.socket_mode = mode;
            }
            if let Some(enforce_root) = socket.enforce_root_owner {
                config.socket.enforce_root_owner = enforce_root;
            }
            if let Some(grp) = socket.socket_group {
                config.socket.socket_group = Some(grp);
            }
        }

        if let Some(dispatcher) = file.dispatcher {
            if let Some(max_conn) = dispatcher.max_concurrent_connections {
                config.dispatcher.max_concurrent_connections = max_conn;
            }
            if let Some(timeout_ms) = dispatcher.connection_timeout_ms {
                config.dispatcher.connection_timeout = Duration::from_millis(timeout_ms);
            }
            if let Some(enforce_session) = dispatcher.enforce_active_session {
                config.dispatcher.enforce_active_session = enforce_session;
            }
            if let Some(sessions_dir) = dispatcher.logind_sessions_dir {
                config.dispatcher.logind_sessions_dir = sessions_dir;
            }
        }

        if let Some(pipe) = file.pipeline {
            if let Some(camera_device) = pipe.camera_device {
                config.pipeline.camera.device_path = camera_device;
            }
            if let Some(use_mock) = pipe.use_mock_camera {
                config.pipeline.use_mock_camera = use_mock;
            }
            if let Some(models_dir) = pipe.models_dir {
                config.pipeline.models_dir = models_dir;
            }
            if let Some(biometrics_dir) = pipe.biometrics_dir {
                config.pipeline.biometrics_dir = biometrics_dir;
            }
            if let Some(master_key_path) = pipe.master_key_path {
                config.pipeline.master_key_path = master_key_path;
            }

            if let Some(ev) = pipe.evidence {
                if let Some(enabled) = ev.enabled {
                    config.pipeline.evidence.enabled = enabled;
                }
                if let Some(base_dir) = ev.base_dir {
                    config.pipeline.evidence.base_dir = base_dir;
                }
                if let Some(key_path) = ev.key_path {
                    config.pipeline.evidence.key_path = key_path;
                }
                if let Some(retention_days) = ev.retention_days {
                    config.pipeline.evidence.retention_days = retention_days;
                }
                if let Some(daily_cap) = ev.daily_cap_per_uid {
                    config.pipeline.evidence.daily_cap_per_uid = daily_cap;
                }
            }

            if let Some(th) = pipe.thresholds {
                let current_match = config.pipeline.thresholds.match_threshold();
                let current_pad = config.pipeline.thresholds.pad_threshold();
                let match_thresh = th.match_threshold.unwrap_or(current_match);
                let pad_thresh = th.pad_threshold.unwrap_or(current_pad);
                config.pipeline.thresholds =
                    soos_policy::ThresholdConfig::new_raw(match_thresh, pad_thresh);
            }

            if let Some(rl) = pipe.rate_limit {
                if let Some(max_att) = rl.max_attempts {
                    config.pipeline.rate_limit.max_attempts = max_att;
                }
                if let Some(secs) = rl.window_duration_secs {
                    config.pipeline.rate_limit.window_duration_ns =
                        secs.saturating_mul(1_000_000_000);
                }
                if let Some(max_uids) = rl.max_tracked_uids {
                    config.pipeline.rate_limit.max_tracked_uids = max_uids;
                }
            }
        }

        Ok(config)
    }

    /// Loads and parses configuration from a specific file path.
    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self, DaemonError> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|e| {
            DaemonError::Config(format!(
                "Failed to read configuration file at '{}': {}",
                path.display(),
                e
            ))
        })?;
        Self::from_toml_str(&content)
    }

    /// Loads configuration from the specified optional path, falling back to `/etc/soos/daemon.toml`
    /// if present on disk, or `DaemonConfig::default()`.
    pub fn load_or_default(path_opt: Option<&Path>) -> Result<Self, DaemonError> {
        if let Some(path) = path_opt {
            return Self::load_from_path(path);
        }

        let default_system_config = Path::new("/etc/soos/daemon.toml");
        if default_system_config.is_file() {
            Self::load_from_path(default_system_config)
        } else {
            Ok(Self::default())
        }
    }
}

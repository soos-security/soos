//! Configuration structures for the soos daemon.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::error::DaemonError;
use crate::limits::PeerLimitsConfig;
use crate::preview::PreviewConfig;

/// The only socket permission modes accepted from configuration (GitHub #199, DMN-08).
///
/// Neither grants any bit to "other", and neither carries setuid/setgid/sticky bits.
pub const ALLOWED_SOCKET_MODES: [u32; 2] = [0o660, 0o600];

/// Lower bound of `[dispatcher] connection_timeout_ms` (a zero or near-zero timeout makes
/// every request time out and leaves the camera wake budget at zero).
pub const MIN_CONNECTION_TIMEOUT_MS: u64 = 100;

/// Upper bound of `[dispatcher] connection_timeout_ms` (a connection permit must never be
/// held for an unbounded time).
pub const MAX_CONNECTION_TIMEOUT_MS: u64 = 10_000;

/// Default `[dispatcher] connection_timeout_ms` (user decision 2026-09-30).
///
/// The daemon's request budget is min(client deadline, request start + this timeout) minus
/// `RESPONSE_WRITE_MARGIN_MS`. 2500 ms matches the GDM line `timeout_ms=2500`, so the greeter
/// really gets that daemon time; console/sudo stacks stay capped by the 1000 ms PAM module
/// default (`crates/pam/src/config.rs` `DEFAULT_TIMEOUT_MS`) through their client deadline.
pub const DEFAULT_CONNECTION_TIMEOUT_MS: u64 = 2500;

/// Maximum length in bytes of the `log_level` filter directive.
pub const MAX_LOG_LEVEL_LEN: usize = 256;

/// Number of warm-up frames `soos-daemon` discards after a camera (re)start, whatever the
/// configuration source (GitHub #205, review finding DMN-16).
///
/// Applied by [`DaemonConfig::from_toml_str`] (with or without a `[pipeline]` table) and by
/// [`DaemonConfig::runtime_default`] (no `/etc/soos/daemon.toml`). `0` keeps the documented
/// instant-wake contract; operators opt into discarding frames with `[pipeline] warmup_frames`.
/// The camera crate default (`soos_camera_v4l::CameraConfig::default()`, 20) is a library
/// default and is not what the daemon runs with.
pub const DAEMON_DEFAULT_WARMUP_FRAMES: usize = 0;

/// System configuration file read by `soos-daemon` when `--config` is not given.
pub const DEFAULT_CONFIG_PATH: &str = "/etc/soos/daemon.toml";

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

impl SocketConfig {
    /// Validates the socket settings fail-closed.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::Config`] when `socket_mode` is not one of
    /// [`ALLOWED_SOCKET_MODES`].
    pub fn validate(&self) -> Result<(), DaemonError> {
        if !ALLOWED_SOCKET_MODES.contains(&self.socket_mode) {
            return Err(DaemonError::Config(format!(
                "[socket] socket_mode {:o} is not allowed (accepted: 660, 600; the socket must \
                 never be accessible to other users)",
                self.socket_mode
            )));
        }
        Ok(())
    }
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

impl DispatcherConfig {
    /// Validates the dispatcher settings fail-closed.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::Config`] when `max_concurrent_connections` is zero or
    /// `connection_timeout` lies outside
    /// [`MIN_CONNECTION_TIMEOUT_MS`]..=[`MAX_CONNECTION_TIMEOUT_MS`].
    pub fn validate(&self) -> Result<(), DaemonError> {
        if self.max_concurrent_connections == 0 {
            return Err(DaemonError::Config(
                "[dispatcher] max_concurrent_connections must be at least 1".into(),
            ));
        }
        let timeout_ms = self.connection_timeout.as_millis();
        if timeout_ms < u128::from(MIN_CONNECTION_TIMEOUT_MS)
            || timeout_ms > u128::from(MAX_CONNECTION_TIMEOUT_MS)
        {
            return Err(DaemonError::Config(format!(
                "[dispatcher] connection_timeout_ms {timeout_ms} is outside \
                 {MIN_CONNECTION_TIMEOUT_MS}..={MAX_CONNECTION_TIMEOUT_MS}"
            )));
        }
        Ok(())
    }
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_millis(DEFAULT_CONNECTION_TIMEOUT_MS),
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
    /// Global daily evidence snapshot cap across all UIDs
    /// (`[pipeline.evidence] daily_cap_total`, GitHub #276).
    pub evidence_daily_cap_total: u32,
    /// Threshold configuration for authorization decisions.
    pub thresholds: soos_policy::ThresholdConfig,
    /// Rate limit configuration per UID.
    pub rate_limit: soos_policy::RateLimitConfig,
    /// Whether to force mock camera simulation rather than hardware device.
    pub use_mock_camera: bool,
    /// ONNX Runtime intra-op threads per model session (`[pipeline] inference_intra_threads`,
    /// GitHub #252). Defaults to `min(4, available_parallelism)`; validated to
    /// `1..=soos_inference_ort::MAX_INTRA_THREADS` at load.
    pub inference_intra_threads: usize,
}

impl PipelineConfig {
    /// Validates the pipeline settings that can silently disable a security control or
    /// reject every request.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::Config`] when the thresholds fail the policy security floor,
    /// the vision thresholds differ from the policy thresholds, a rate-limit bound is zero
    /// (window shorter than one second) or `retention_days` is zero.
    pub fn validate(&self) -> Result<(), DaemonError> {
        let match_thresh = self.thresholds.match_threshold();
        let pad_thresh = self.thresholds.pad_threshold();
        soos_policy::ThresholdConfig::builder()
            .match_threshold(match_thresh)
            .pad_threshold(pad_thresh)
            .build_with_security_floor()
            .map_err(|e| DaemonError::Config(format!("Invalid [pipeline.thresholds]: {e}")))?;
        // Bitwise comparison: NaN never equals itself, and the mirror must be an exact copy.
        if self.vision.match_threshold.to_bits() != match_thresh.to_bits() {
            return Err(DaemonError::Config(
                "[pipeline.thresholds] match_threshold differs between the policy and the \
                 vision pipeline"
                    .into(),
            ));
        }
        if self.vision.pad_threshold.to_bits() != pad_thresh.to_bits() {
            return Err(DaemonError::Config(
                "[pipeline.thresholds] pad_threshold differs between the policy and the \
                 vision pipeline"
                    .into(),
            ));
        }

        let rl = &self.rate_limit;
        if rl.max_attempts == 0 {
            return Err(DaemonError::Config(
                "[pipeline.rate_limit] max_attempts must be at least 1".into(),
            ));
        }
        if rl.window_duration_ns < 1_000_000_000 {
            return Err(DaemonError::Config(
                "[pipeline.rate_limit] window_duration_secs must be at least 1".into(),
            ));
        }
        if rl.max_tracked_uids == 0 {
            return Err(DaemonError::Config(
                "[pipeline.rate_limit] max_tracked_uids must be at least 1".into(),
            ));
        }

        if self.evidence.retention_days == 0 {
            return Err(DaemonError::Config(
                "[pipeline.evidence] retention_days must be at least 1".into(),
            ));
        }
        Ok(())
    }
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
            evidence_daily_cap_total: soos_evidence_store::DEFAULT_DAILY_CAP_TOTAL,
            thresholds: soos_policy::ThresholdConfig::default(),
            rate_limit: soos_policy::RateLimitConfig::default(),
            use_mock_camera: false,
            inference_intra_threads: soos_inference_ort::default_intra_threads(),
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
    /// GUI preview stream authorization (`[preview]`, disabled by default).
    pub preview: PreviewConfig,
    /// Per-peer connection and event limits (`[peer_limits]`, GitHub #157 / #175).
    pub peer_limits: PeerLimitsConfig,
    /// Logging filter directive (e.g. "info", "debug").
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket: SocketConfig::default(),
            dispatcher: DispatcherConfig::default(),
            pipeline: PipelineConfig::default(),
            preview: PreviewConfig::default(),
            peer_limits: PeerLimitsConfig::default(),
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
    preview: Option<PreviewConfigFile>,
    #[serde(default)]
    peer_limits: Option<PeerLimitsConfigFile>,
    #[serde(default)]
    log_level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PreviewConfigFile {
    enabled: Option<bool>,
    allowed_uids: Option<Vec<u32>>,
    max_requests_per_sec: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct PeerLimitsConfigFile {
    max_connections_per_uid: Option<usize>,
    reserved_root_connections: Option<usize>,
    max_requests_per_connection: Option<usize>,
    max_connection_lifetime_ms: Option<u64>,
    max_events_per_window: Option<u32>,
    event_window_ms: Option<u64>,
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
    sensor_preference: Option<String>,
    idle_timeout_secs: Option<u64>,
    warmup_frames: Option<usize>,
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
    inference_intra_threads: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct EvidenceConfigFile {
    enabled: Option<bool>,
    base_dir: Option<PathBuf>,
    key_path: Option<PathBuf>,
    retention_days: Option<u32>,
    daily_cap_per_uid: Option<u32>,
    daily_cap_total: Option<u32>,
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

/// Validates a `tracing` filter directive without installing it.
fn validate_log_level(level: &str) -> Result<(), DaemonError> {
    if level.len() > MAX_LOG_LEVEL_LEN {
        return Err(DaemonError::Config(format!(
            "log_level exceeds {MAX_LOG_LEVEL_LEN} bytes"
        )));
    }
    if level.trim().is_empty() {
        return Err(DaemonError::Config("log_level must not be empty".into()));
    }
    tracing_subscriber::EnvFilter::try_new(level)
        .map(drop)
        .map_err(|e| DaemonError::Config(format!("log_level is not a valid filter: {e}")))
}

impl DaemonConfig {
    /// Validates the complete configuration fail-closed (GitHub #199, DMN-08).
    ///
    /// Called by [`DaemonConfig::from_toml_str`] (hence by `load_from_path` and
    /// `load_or_default`), so the daemon refuses to start on any invalid value.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::Config`] naming the first invalid setting.
    pub fn validate(&self) -> Result<(), DaemonError> {
        validate_log_level(&self.log_level)?;
        self.socket.validate()?;
        self.dispatcher.validate()?;
        self.pipeline.validate()?;
        self.preview.validate()?;
        self.peer_limits
            .validate(self.dispatcher.max_concurrent_connections)?;
        Ok(())
    }

    /// Configuration the running daemon uses when no key overrides a value.
    ///
    /// Identical to [`DaemonConfig::default`] except for daemon-specific runtime defaults that
    /// differ from the library defaults of the component crates (currently
    /// [`DAEMON_DEFAULT_WARMUP_FRAMES`]). Every load path (`from_toml_str`, `load_from_path`,
    /// `load_or_default` without a file) starts from it, so the same binary behaves the same
    /// with or without `/etc/soos/daemon.toml`.
    #[must_use]
    pub fn runtime_default() -> Self {
        let mut config = Self::default();
        config.pipeline.camera.warmup_frames = DAEMON_DEFAULT_WARMUP_FRAMES;
        config
    }

    /// Parses a complete daemon configuration from a TOML string.
    pub fn from_toml_str(content: &str) -> Result<Self, DaemonError> {
        let file: DaemonConfigFile = toml::from_str(content)
            .map_err(|e| DaemonError::Config(format!("Failed to parse TOML configuration: {e}")))?;

        let mut config = Self::runtime_default();

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
                // Shared sentinel vocabulary with soos-enroll / soos-gui (GitHub #152):
                // "", "auto" and "default" keep the auto-detection sentinel.
                if !soos_camera_v4l::is_auto_camera_device(&camera_device) {
                    config.pipeline.camera.device_path = camera_device;
                }
            }
            if let Some(warmup_frames) = pipe.warmup_frames {
                config.pipeline.camera.warmup_frames = warmup_frames;
            }
            if let Some(preference) = pipe
                .sensor_preference
                .as_deref()
                .and_then(soos_camera_v4l::parse_sensor_preference)
            {
                config.pipeline.camera.sensor_preference = preference;
            }
            if let Some(idle_secs) = pipe.idle_timeout_secs {
                config.pipeline.camera.idle_timeout = Duration::from_secs(idle_secs);
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
            if let Some(threads) = pipe.inference_intra_threads {
                let max = soos_inference_ort::MAX_INTRA_THREADS;
                if !(1..=max).contains(&threads) {
                    return Err(DaemonError::Config(format!(
                        "Invalid [pipeline] inference_intra_threads = {threads}: expected 1..={max}"
                    )));
                }
                config.pipeline.inference_intra_threads = threads;
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
                if let Some(daily_cap_total) = ev.daily_cap_total {
                    config.pipeline.evidence_daily_cap_total = daily_cap_total;
                }
            }

            if let Some(th) = pipe.thresholds {
                let current_match = config.pipeline.thresholds.match_threshold();
                let current_pad = config.pipeline.thresholds.pad_threshold();
                let match_thresh = th.match_threshold.unwrap_or(current_match);
                let pad_thresh = th.pad_threshold.unwrap_or(current_pad);
                // Operator-editable thresholds are validated and floored (GitHub #170,
                // PAD-04): `pad_threshold = 0` would otherwise disable anti-spoofing silently.
                let validated = soos_policy::ThresholdConfig::builder()
                    .match_threshold(match_thresh)
                    .pad_threshold(pad_thresh)
                    .build_with_security_floor()
                    .map_err(|e| {
                        DaemonError::Config(format!("Invalid [pipeline.thresholds]: {e}"))
                    })?;
                config.pipeline.thresholds = validated;
                config.pipeline.vision.match_threshold = validated.match_threshold();
                config.pipeline.vision.pad_threshold = validated.pad_threshold();
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

        if let Some(preview) = file.preview {
            if let Some(enabled) = preview.enabled {
                config.preview.enabled = enabled;
            }
            if let Some(uids) = preview.allowed_uids {
                config.preview.allowed_uids = uids;
            }
            if let Some(max_per_sec) = preview.max_requests_per_sec {
                config.preview.max_requests_per_sec = max_per_sec;
            }
        }
        config.preview.validate()?;

        if let Some(limits) = file.peer_limits {
            let target = &mut config.peer_limits;
            if let Some(v) = limits.max_connections_per_uid {
                target.max_connections_per_uid = v;
            }
            if let Some(v) = limits.reserved_root_connections {
                target.reserved_root_connections = v;
            }
            if let Some(v) = limits.max_requests_per_connection {
                target.max_requests_per_connection = v;
            }
            if let Some(v) = limits.max_connection_lifetime_ms {
                target.max_connection_lifetime = Duration::from_millis(v);
            }
            if let Some(v) = limits.max_events_per_window {
                target.max_events_per_window = v;
            }
            if let Some(v) = limits.event_window_ms {
                target.event_window = Duration::from_millis(v);
            }
        }
        config
            .peer_limits
            .validate(config.dispatcher.max_concurrent_connections)?;
        config.validate()?;

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

    /// Loads configuration from the specified optional path, falling back to
    /// [`DEFAULT_CONFIG_PATH`] if present on disk, or [`DaemonConfig::runtime_default`].
    pub fn load_or_default(path_opt: Option<&Path>) -> Result<Self, DaemonError> {
        Self::load_or_default_with_system_path(path_opt, Path::new(DEFAULT_CONFIG_PATH))
    }

    /// [`DaemonConfig::load_or_default`] with an injectable system configuration path.
    ///
    /// An explicit `path_opt` is always read (a missing file is an error). Otherwise
    /// `system_path` is read when it is a regular file, and
    /// [`DaemonConfig::runtime_default`] is returned when it is absent.
    pub fn load_or_default_with_system_path(
        path_opt: Option<&Path>,
        system_path: &Path,
    ) -> Result<Self, DaemonError> {
        if let Some(path) = path_opt {
            return Self::load_from_path(path);
        }

        if system_path.is_file() {
            Self::load_from_path(system_path)
        } else {
            Ok(Self::runtime_default())
        }
    }
}

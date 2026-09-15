//! Configuration structures for the soos daemon.

use std::path::PathBuf;
use std::time::Duration;

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
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/run/soos/daemon.sock"),
            socket_dir: PathBuf::from("/run/soos"),
            socket_mode: 0o660,
            enforce_root_owner: true,
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
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_millis(250),
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
#[derive(Debug, Clone, Default)]
pub struct DaemonConfig {
    /// Socket listener configuration.
    pub socket: SocketConfig,
    /// Connection dispatcher configuration.
    pub dispatcher: DispatcherConfig,
    /// Optional pipeline configuration (if None, operates in basic skeleton mode).
    pub pipeline: Option<PipelineConfig>,
    /// Logging filter directive (e.g. "info", "debug").
    pub log_level: String,
}

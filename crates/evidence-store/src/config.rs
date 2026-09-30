//! Configuration for the evidence snapshot storage.

use std::path::PathBuf;

/// Default retention duration in days.
pub const DEFAULT_RETENTION_DAYS: u32 = 7;

/// Default maximum snapshot captures per UID per day.
pub const DEFAULT_DAILY_CAP_PER_UID: u32 = 10;

/// Default maximum snapshot captures per day across all UIDs (GitHub #276, DMN-03 remainder).
///
/// Bounds the disk usage of one day whatever the number of distinct UIDs a caller names:
/// the per-UID cap alone lets `DEFAULT_DAILY_CAP_PER_UID` snapshots per UID accumulate for
/// every UID in `0..=MAX_VALID_UID`.
pub const DEFAULT_DAILY_CAP_TOTAL: u32 = 100;

/// Default evidence directory path.
pub const DEFAULT_EVIDENCE_DIR: &str = "/var/lib/soos/evidence";

/// Default evidence master key file path.
pub const DEFAULT_KEY_PATH: &str = "/var/lib/soos/evidence.key";

/// Strongly-typed configuration for evidence capture and persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceConfig {
    /// Whether evidence snapshot persistence is enabled (strictly opt-in, default false).
    pub enabled: bool,
    /// Base directory for stored evidence snapshots.
    pub base_dir: PathBuf,
    /// File path to the master encryption key.
    pub key_path: PathBuf,
    /// Maximum retention period in days before automated purging.
    pub retention_days: u32,
    /// Maximum number of evidence snapshots captured per UID per day.
    pub daily_cap_per_uid: u32,
}

impl Default for EvidenceConfig {
    /// Constructs default configuration with evidence storage disabled (strictly opt-in).
    fn default() -> Self {
        Self {
            enabled: false,
            base_dir: PathBuf::from(DEFAULT_EVIDENCE_DIR),
            key_path: PathBuf::from(DEFAULT_KEY_PATH),
            retention_days: DEFAULT_RETENTION_DAYS,
            daily_cap_per_uid: DEFAULT_DAILY_CAP_PER_UID,
        }
    }
}

impl EvidenceConfig {
    /// Constructs a new builder or customized config enabled for testing.
    pub fn enabled_with_dir(base_dir: PathBuf, key_path: PathBuf) -> Self {
        Self {
            enabled: true,
            base_dir,
            key_path,
            retention_days: DEFAULT_RETENTION_DAYS,
            daily_cap_per_uid: DEFAULT_DAILY_CAP_PER_UID,
        }
    }
}

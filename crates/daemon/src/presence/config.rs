//! `[presence]` configuration (GitHub #323).

use std::time::Duration;

use crate::error::DaemonError;

/// Presence auto-unlock is enabled by default (owner decision 2026-10-02).
pub const DEFAULT_PRESENCE_ENABLED: bool = true;
/// Default minimum time between the starts of two scans of one session.
pub const DEFAULT_SCAN_INTERVAL_MS: u64 = 2000;
/// Lower bound of `[presence] scan_interval_ms`.
pub const MIN_SCAN_INTERVAL_MS: u64 = 1000;
/// Upper bound of `[presence] scan_interval_ms`.
pub const MAX_SCAN_INTERVAL_MS: u64 = 60_000;
/// Default time a session must have been observed locked before any scan.
pub const DEFAULT_LOCK_GRACE_MS: u64 = 3000;
/// Lower bound of `[presence] lock_grace_ms`.
pub const MIN_LOCK_GRACE_MS: u64 = 1000;
/// Upper bound of `[presence] lock_grace_ms`.
pub const MAX_LOCK_GRACE_MS: u64 = 60_000;

/// Validated `[presence]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceConfig {
    /// Whether the presence worker runs.
    pub enabled: bool,
    /// Minimum time between the starts of two scans of the same session.
    pub scan_interval: Duration,
    /// Time a session must have been observed locked before any scan or unlock.
    pub lock_grace: Duration,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_PRESENCE_ENABLED,
            scan_interval: Duration::from_millis(DEFAULT_SCAN_INTERVAL_MS),
            lock_grace: Duration::from_millis(DEFAULT_LOCK_GRACE_MS),
        }
    }
}

/// Checks that `value` lies within `[min_ms, max_ms]`.
fn check_range(key: &str, value: Duration, min_ms: u64, max_ms: u64) -> Result<(), DaemonError> {
    let min = Duration::from_millis(min_ms);
    let max = Duration::from_millis(max_ms);
    if value < min || value > max {
        return Err(DaemonError::Config(format!(
            "[presence] {key} must be within {min_ms}..={max_ms} ms (0 is never a sentinel; \
             disable presence with enabled = false)"
        )));
    }
    Ok(())
}

impl PresenceConfig {
    /// Validates both intervals, whether or not the feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::Config`] naming `[presence] scan_interval_ms` or
    /// `lock_grace_ms` when it is outside its range (`0` included).
    pub fn validate(&self) -> Result<(), DaemonError> {
        check_range(
            "scan_interval_ms",
            self.scan_interval,
            MIN_SCAN_INTERVAL_MS,
            MAX_SCAN_INTERVAL_MS,
        )?;
        check_range(
            "lock_grace_ms",
            self.lock_grace,
            MIN_LOCK_GRACE_MS,
            MAX_LOCK_GRACE_MS,
        )
    }
}

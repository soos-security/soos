//! Contract tests of GitHub #323 for the `[presence]` table of `daemon.toml` and the presence
//! constants (matrix PAU1, PAU2 daemon default, PAU14 bounds, PAU17 bus address, C6).
//!
//! The feature is enabled by default (owner decision 2026-10-02): an absent file, an empty
//! file and an empty `[presence]` table give the same configuration; `0` is never
//! "immediately" nor "disabled" (disabling is `enabled = false`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::time::Duration;

use soos_daemon::config::DaemonConfig;
use soos_daemon::error::DaemonError;
use soos_daemon::presence;
use soos_daemon::presence::config::{
    PresenceConfig, DEFAULT_LOCK_GRACE_MS, DEFAULT_PRESENCE_ENABLED, DEFAULT_SCAN_INTERVAL_MS,
    MAX_LOCK_GRACE_MS, MAX_SCAN_INTERVAL_MS, MIN_LOCK_GRACE_MS, MIN_SCAN_INTERVAL_MS,
};

fn assert_toml_rejected(toml: &str, expected_setting: &str) {
    match DaemonConfig::from_toml_str(toml) {
        Err(DaemonError::Config(msg)) => assert!(
            msg.contains(expected_setting),
            "error for `{toml}` must name `{expected_setting}`: {msg}"
        ),
        Err(other) => panic!("`{toml}` must yield DaemonError::Config, got {other:?}"),
        Ok(_) => panic!("`{toml}` must be refused, but was accepted"),
    }
}

fn accepted(toml: &str) -> DaemonConfig {
    DaemonConfig::from_toml_str(toml)
        .unwrap_or_else(|e| panic!("`{toml}` must be accepted, got {e:?}"))
}

fn presence_warnings(config: &DaemonConfig) -> Vec<&String> {
    config
        .warnings
        .iter()
        .filter(|w| w.contains("presence"))
        .collect()
}

// ---------------------------------------------------------------------------------------
// PAU1 — defaults and table equivalence
// ---------------------------------------------------------------------------------------

/// PAU1: the documented defaults (enabled, 2000 ms interval, 3000 ms grace) and bounds.
#[test]
fn test_pau_presence_defaults_and_bounds_match_spec() {
    assert_eq!(
        DEFAULT_PRESENCE_ENABLED,
        PresenceConfig::default().enabled,
        "the default is the single source"
    );
    assert!(
        PresenceConfig::default().enabled,
        "enabled by default (owner decision)"
    );
    assert_eq!(DEFAULT_SCAN_INTERVAL_MS, 2000);
    assert_eq!(MIN_SCAN_INTERVAL_MS, 1000);
    assert_eq!(MAX_SCAN_INTERVAL_MS, 60_000);
    assert_eq!(DEFAULT_LOCK_GRACE_MS, 3000);
    assert_eq!(MIN_LOCK_GRACE_MS, 1000);
    assert_eq!(MAX_LOCK_GRACE_MS, 60_000);

    let config = PresenceConfig::default();
    assert_eq!(
        config,
        PresenceConfig {
            enabled: true,
            scan_interval: Duration::from_millis(2000),
            lock_grace: Duration::from_millis(3000),
        }
    );
    config.validate().expect("the default must be valid");
}

/// PAU1: absent file (built-in default), empty file and empty table are the same.
#[test]
fn test_pau_absent_empty_file_and_empty_table_give_the_same_presence_config() {
    let expected = PresenceConfig::default();
    assert_eq!(DaemonConfig::default().presence, expected);
    assert_eq!(DaemonConfig::runtime_default().presence, expected);
    let temp = tempfile::tempdir().unwrap();
    let absent = temp.path().join("daemon.toml");
    assert_eq!(
        DaemonConfig::load_or_default_with_system_path(None, &absent)
            .unwrap()
            .presence,
        expected,
        "an absent daemon.toml must give the default presence configuration"
    );
    assert_eq!(accepted("").presence, expected);
    assert_eq!(accepted("[presence]\n").presence, expected);
    assert_eq!(accepted("log_level = \"info\"\n").presence, expected);
}

/// PAU1: every key is read from `[presence]`.
#[test]
fn test_pau_presence_keys_are_parsed() {
    let config =
        accepted("[presence]\nenabled = false\nscan_interval_ms = 5000\nlock_grace_ms = 10000\n");
    assert_eq!(
        config.presence,
        PresenceConfig {
            enabled: false,
            scan_interval: Duration::from_millis(5000),
            lock_grace: Duration::from_millis(10_000),
        }
    );
    // A missing key keeps its default.
    let partial = accepted("[presence]\nlock_grace_ms = 4000\n");
    assert!(partial.presence.enabled);
    assert_eq!(partial.presence.scan_interval, Duration::from_millis(2000));
    assert_eq!(partial.presence.lock_grace, Duration::from_millis(4000));
}

/// PAU1: `0`, `999` and `60001` are startup errors naming the key, for both intervals.
#[test]
fn test_pau_out_of_range_presence_intervals_are_startup_errors() {
    for value in [0u64, 999, 60_001, u64::from(u32::MAX)] {
        assert_toml_rejected(
            &format!("[presence]\nscan_interval_ms = {value}\n"),
            "scan_interval_ms",
        );
        assert_toml_rejected(
            &format!("[presence]\nlock_grace_ms = {value}\n"),
            "lock_grace_ms",
        );
    }
}

/// PAU1: `0` is rejected even when the feature is disabled (never a sentinel).
#[test]
fn test_pau_zero_interval_is_rejected_even_when_disabled() {
    assert_toml_rejected(
        "[presence]\nenabled = false\nscan_interval_ms = 0\n",
        "scan_interval_ms",
    );
    assert_toml_rejected(
        "[presence]\nenabled = false\nlock_grace_ms = 0\n",
        "lock_grace_ms",
    );
}

/// PAU1: the bounds themselves (`1000`, `60000`) are accepted.
#[test]
fn test_pau_presence_interval_bounds_are_accepted() {
    for value in [1000u64, 60_000] {
        let config = accepted(&format!(
            "[presence]\nscan_interval_ms = {value}\nlock_grace_ms = {value}\n"
        ));
        assert_eq!(config.presence.scan_interval, Duration::from_millis(value));
        assert_eq!(config.presence.lock_grace, Duration::from_millis(value));
    }
}

/// PAU1: a wrong type is a TOML parse error (startup error), like every other key.
#[test]
fn test_pau_presence_wrong_type_is_a_startup_error() {
    for toml in [
        "[presence]\nenabled = \"yes\"\n",
        "[presence]\nscan_interval_ms = \"2000\"\n",
        "[presence]\nlock_grace_ms = -1\n",
        "presence = 1\n",
    ] {
        assert!(
            matches!(
                DaemonConfig::from_toml_str(toml),
                Err(DaemonError::Config(_))
            ),
            "`{toml}` must be refused"
        );
    }
}

/// PAU1: `PresenceConfig::validate` names the key and `DaemonConfig::validate` calls it.
#[test]
fn test_pau_presence_validate_names_the_key_and_is_called_by_daemon_validate() {
    let bad_interval = PresenceConfig {
        scan_interval: Duration::from_millis(999),
        ..PresenceConfig::default()
    };
    match bad_interval.validate() {
        Err(DaemonError::Config(msg)) => assert!(msg.contains("scan_interval_ms"), "{msg}"),
        other => panic!("expected a Config error naming scan_interval_ms, got {other:?}"),
    }
    let bad_grace = PresenceConfig {
        lock_grace: Duration::ZERO,
        ..PresenceConfig::default()
    };
    match bad_grace.validate() {
        Err(DaemonError::Config(msg)) => assert!(msg.contains("lock_grace_ms"), "{msg}"),
        other => panic!("expected a Config error naming lock_grace_ms, got {other:?}"),
    }
    let config = DaemonConfig {
        presence: bad_grace,
        ..DaemonConfig::default()
    };
    assert!(
        matches!(config.validate(), Err(DaemonError::Config(msg)) if msg.contains("lock_grace_ms")),
        "DaemonConfig::validate must validate [presence]"
    );
}

// ---------------------------------------------------------------------------------------
// Rate-limit interaction warnings (spec §2.6, never errors)
// ---------------------------------------------------------------------------------------

/// A pinned `max_attempts <= 5` with presence enabled starts with a warning (never scans).
#[test]
fn test_pau_presence_warns_when_max_attempts_leaves_no_room_for_scans() {
    for max in [1u32, 5] {
        let config = accepted(&format!("[pipeline.rate_limit]\nmax_attempts = {max}\n"));
        let warnings = presence_warnings(&config);
        assert!(
            warnings.iter().any(|w| w.contains("max_attempts")),
            "max_attempts = {max} with presence enabled must warn naming max_attempts: {:?}",
            config.warnings
        );
    }
}

/// A budget below `ceil(window / scan_interval) + 5` warns that scans are throttled.
#[test]
fn test_pau_presence_warns_when_scans_will_be_throttled() {
    // 60 s / 1 s = 60 scans > 20 - 5.
    let config =
        accepted("[pipeline.rate_limit]\nmax_attempts = 20\n[presence]\nscan_interval_ms = 1000\n");
    assert!(
        presence_warnings(&config)
            .iter()
            .any(|w| w.contains("throttled")),
        "a throttled presence scan rate must warn: {:?}",
        config.warnings
    );
}

/// The defaults (30 scans/min, 40 attempts, reserve 5) produce no presence warning, and a
/// disabled feature never warns.
#[test]
fn test_pau_presence_defaults_and_disabled_feature_do_not_warn() {
    assert!(presence_warnings(&accepted("")).is_empty());
    assert!(presence_warnings(&accepted("[presence]\n")).is_empty());
    let disabled =
        accepted("[presence]\nenabled = false\n[pipeline.rate_limit]\nmax_attempts = 5\n");
    assert!(
        presence_warnings(&disabled).is_empty(),
        "a disabled presence feature must not warn: {:?}",
        disabled.warnings
    );
}

/// The warnings are not errors: the configuration still validates.
#[test]
fn test_pau_presence_rate_limit_warning_is_not_an_error() {
    let config = accepted("[pipeline.rate_limit]\nmax_attempts = 5\n");
    config.validate().expect("a warning never fails validation");
    assert_eq!(config.pipeline.rate_limit.max_attempts, 5);
    assert!(config.presence.enabled);
}

// ---------------------------------------------------------------------------------------
// PAU2 — daemon default rate limit
// ---------------------------------------------------------------------------------------

/// PAU2: the daemon's default `[pipeline.rate_limit]` is 40 attempts per 60 s.
#[test]
fn test_pau_daemon_default_rate_limit_is_forty_per_minute() {
    for config in [
        DaemonConfig::default(),
        DaemonConfig::runtime_default(),
        accepted(""),
    ] {
        assert_eq!(config.pipeline.rate_limit.max_attempts, 40);
        assert_eq!(
            config.pipeline.rate_limit.window_duration_ns,
            60_000_000_000
        );
    }
}

// ---------------------------------------------------------------------------------------
// Presence constants (spec §2.6 single source)
// ---------------------------------------------------------------------------------------

/// PAU14 / §2.6: every presence bound has the value the spec fixes.
#[test]
fn test_pau_presence_constants_match_spec() {
    assert_eq!(presence::LOCK_POLL_INTERVAL_MS, 1000);
    assert_eq!(presence::PRESENCE_RESERVED_ATTEMPTS, 5);
    assert_eq!(presence::DBUS_CALL_TIMEOUT_MS, 500);
    assert_eq!(presence::DBUS_CONNECT_TIMEOUT_MS, 1000);
    assert_eq!(presence::DBUS_RECONNECT_BACKOFF_MIN_MS, 1000);
    assert_eq!(presence::DBUS_RECONNECT_BACKOFF_MAX_MS, 30_000);
    assert_eq!(presence::MAX_PRESENCE_SEAT_SESSIONS, 16);
    assert_eq!(presence::MAX_TRACKED_LOCKED_SESSIONS, 16);
    assert_eq!(presence::MAX_ALLOW_TO_UNLOCK_MS, 1000);
    assert_eq!(presence::UNLOCK_CONFIRM_TIMEOUT_MS, 5000);
    assert_eq!(presence::MAX_LOGIND_ERROR_LEN, 256);
    assert_eq!(presence::MAX_DRM_CONNECTORS, 64);
    assert_eq!(presence::MAX_SYSFS_ATTR_BYTES, 64);
    assert_eq!(presence::ACCOUNT_CHECK_TIMEOUT_MS, 500);
    assert_eq!(presence::MAX_USER_NAME_LEN, 256);
    assert_eq!(
        presence::DEFAULT_FAILLOCK_CONF,
        "/etc/security/faillock.conf"
    );
    assert_eq!(
        presence::VENDOR_FAILLOCK_CONF,
        "/usr/etc/security/faillock.conf"
    );
    assert_eq!(presence::DEFAULT_FAILLOCK_DIR, "/run/faillock");
    assert_eq!(presence::DEFAULT_FAILLOCK_DENY, 3);
    assert_eq!(presence::DEFAULT_FAILLOCK_FAIL_INTERVAL_S, 900);
    assert_eq!(presence::DEFAULT_FAILLOCK_UNLOCK_TIME_S, 600);
    assert_eq!(presence::MAX_FAILLOCK_TIME_INTERVAL, 604_800);
    assert_eq!(presence::TALLY_RECORD_BYTES, 64);
    assert_eq!(presence::TALLY_STATUS_VALID, 0x1);
    assert_eq!(presence::MAX_TALLY_BYTES, 69_632);
    assert_eq!(
        presence::MAX_TALLY_BYTES,
        1088 * presence::TALLY_RECORD_BYTES
    );
    assert_eq!(presence::MAX_FAILLOCK_CONF_BYTES, 65_536);
    assert_eq!(presence::MAX_PAM_FILE_BYTES, 65_536);
    assert_eq!(
        presence::DEFAULT_PAM_DIRS,
        ["/etc/pam.d", "/usr/lib/pam.d", "/usr/etc/pam.d"]
    );
    assert_eq!(presence::MAX_PAM_DIR_ENTRIES, 512);
    assert_eq!(presence::DEFAULT_SHADOW_PATH, "/etc/shadow");
    assert_eq!(presence::MAX_SHADOW_BYTES, 4 * 1024 * 1024);
    assert_eq!(presence::MAX_SHADOW_LINES, 65_536);
    assert_eq!(
        presence::SYSTEM_BUS_ADDRESS,
        "unix:path=/run/dbus/system_bus_socket"
    );
    assert_eq!(presence::DEFAULT_DRM_SYSFS_DIR, "/sys/class/drm");
    assert_eq!(presence::GLOBAL_DISABLE_FLAG, "disabled");
    assert_eq!(presence::PRESENCE_DISABLE_FLAG, "presence.disable");
}

/// C6 (runtime half): the presence kill-switch directory is the PAM flag directory.
#[test]
fn test_pau_kill_switch_dir_is_the_pam_flag_dir() {
    assert_eq!(presence::DEFAULT_KILL_SWITCH_DIR, "/etc/soos");
}

/// PAU14: the reconnect backoff doubles from 1 s and saturates at 30 s.
#[test]
fn test_pau_reconnect_backoff_doubles_and_saturates() {
    let expected_ms = [1000u64, 2000, 4000, 8000, 16_000, 30_000, 30_000];
    for (i, ms) in expected_ms.iter().enumerate() {
        let failures = u32::try_from(i).unwrap() + 1;
        assert_eq!(
            presence::reconnect_backoff(failures),
            Duration::from_millis(*ms),
            "backoff after {failures} consecutive failures"
        );
    }
    assert_eq!(
        presence::reconnect_backoff(u32::MAX),
        Duration::from_millis(30_000),
        "the backoff saturates and never overflows"
    );
    assert_eq!(
        presence::reconnect_backoff(0),
        Duration::ZERO,
        "no failure, no backoff (reset on success)"
    );
}

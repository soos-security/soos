//! Contractual tests: `daemon.toml` is validated fail-closed at load time (GitHub #199, DMN-08).
//!
//! A single mistyped line used to turn the daemon into a world-writable socket
//! (`socket_mode = 438`), a service that rejects or times out every request
//! (`connection_timeout_ms = 0`, `rate_limit.max_attempts = 0`) or a service whose
//! logging filter is silently dropped. Every such value must now be refused with
//! `DaemonError::Config` naming the offending setting, before any socket is bound.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use std::io::Write;
use std::time::Duration;

use soos_daemon::config::{
    DaemonConfig, ALLOWED_SOCKET_MODES, MAX_CONNECTION_TIMEOUT_MS, MAX_LOG_LEVEL_LEN,
    MIN_CONNECTION_TIMEOUT_MS,
};
use soos_daemon::error::DaemonError;

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

fn assert_toml_accepted(toml: &str) -> DaemonConfig {
    DaemonConfig::from_toml_str(toml)
        .unwrap_or_else(|e| panic!("`{toml}` must be accepted, got {e:?}"))
}

fn assert_validate_rejected(config: &DaemonConfig, expected_setting: &str) {
    match config.validate() {
        Err(DaemonError::Config(msg)) => assert!(
            msg.contains(expected_setting),
            "validate() error must name `{expected_setting}`: {msg}"
        ),
        Err(other) => panic!("validate() must yield DaemonError::Config, got {other:?}"),
        Ok(()) => panic!("validate() must refuse an invalid `{expected_setting}`"),
    }
}

#[test]
fn test_default_daemon_config_passes_validation() {
    DaemonConfig::default()
        .validate()
        .expect("the built-in default configuration must be valid");
}

#[test]
fn test_validation_bounds_constants_match_spec() {
    assert_eq!(ALLOWED_SOCKET_MODES, [0o660, 0o600]);
    assert_eq!(MIN_CONNECTION_TIMEOUT_MS, 100);
    assert_eq!(MAX_CONNECTION_TIMEOUT_MS, 10_000);
    assert_eq!(MAX_LOG_LEVEL_LEN, 256);
}

#[test]
fn test_daemon_toml_rejects_world_writable_socket_mode_438() {
    // 438 == 0o666: the exact line quoted by the DMN-08 review.
    assert_toml_rejected("[socket]\nsocket_mode = 438\n", "socket_mode");
}

#[test]
fn test_daemon_toml_rejects_socket_modes_outside_allowlist() {
    for mode in [
        0o777u32, 0o666, 0o664, 0o662, 0o661, 0o640, 0o700, 0o000, 0o4660, 0o2660, 0o1660, 0o10660,
    ] {
        assert_toml_rejected(&format!("[socket]\nsocket_mode = {mode}\n"), "socket_mode");
    }
}

#[test]
fn test_daemon_toml_accepts_allowlisted_socket_modes() {
    for mode in ALLOWED_SOCKET_MODES {
        let cfg = assert_toml_accepted(&format!("[socket]\nsocket_mode = {mode}\n"));
        assert_eq!(cfg.socket.socket_mode, mode);
    }
}

#[test]
fn test_programmatic_world_writable_socket_mode_fails_validate() {
    let mut cfg = DaemonConfig::default();
    cfg.socket.socket_mode = 0o666;
    assert_validate_rejected(&cfg, "socket_mode");
    assert!(cfg.socket.validate().is_err());
}

#[test]
fn test_daemon_toml_rejects_connection_timeout_out_of_bounds() {
    for ms in [0u64, 1, 99, 10_001, u64::MAX] {
        assert_toml_rejected(
            &format!(
                "[dispatcher]\nconnection_timeout_ms = {}\n",
                ms.min(i64::MAX as u64)
            ),
            "connection_timeout_ms",
        );
    }
}

#[test]
fn test_daemon_toml_accepts_connection_timeout_bounds() {
    for ms in [MIN_CONNECTION_TIMEOUT_MS, 1000, MAX_CONNECTION_TIMEOUT_MS] {
        let cfg = assert_toml_accepted(&format!("[dispatcher]\nconnection_timeout_ms = {ms}\n"));
        assert_eq!(cfg.dispatcher.connection_timeout, Duration::from_millis(ms));
    }
}

#[test]
fn test_daemon_toml_rejects_zero_max_concurrent_connections() {
    assert_toml_rejected(
        "[dispatcher]\nmax_concurrent_connections = 0\n",
        "max_concurrent_connections",
    );
}

#[test]
fn test_programmatic_zero_timeout_and_permits_fail_validate() {
    let mut cfg = DaemonConfig::default();
    cfg.dispatcher.connection_timeout = Duration::ZERO;
    assert_validate_rejected(&cfg, "connection_timeout_ms");

    let mut cfg = DaemonConfig::default();
    cfg.dispatcher.max_concurrent_connections = 0;
    assert_validate_rejected(&cfg, "max_concurrent_connections");
}

#[test]
fn test_daemon_toml_rejects_zero_rate_limit_max_attempts() {
    assert_toml_rejected(
        "[pipeline]\nuse_mock_camera = true\n\n[pipeline.rate_limit]\nmax_attempts = 0\n",
        "max_attempts",
    );
}

#[test]
fn test_daemon_toml_rejects_zero_rate_limit_window() {
    assert_toml_rejected(
        "[pipeline]\nuse_mock_camera = true\n\n[pipeline.rate_limit]\nwindow_duration_secs = 0\n",
        "window_duration_secs",
    );
}

#[test]
fn test_daemon_toml_rejects_zero_rate_limit_tracked_uids() {
    assert_toml_rejected(
        "[pipeline]\nuse_mock_camera = true\n\n[pipeline.rate_limit]\nmax_tracked_uids = 0\n",
        "max_tracked_uids",
    );
}

#[test]
fn test_daemon_toml_accepts_minimal_rate_limit() {
    let cfg = assert_toml_accepted(
        "[pipeline]\nuse_mock_camera = true\n\n[pipeline.rate_limit]\n\
         max_attempts = 1\nwindow_duration_secs = 1\nmax_tracked_uids = 1\n",
    );
    assert_eq!(cfg.pipeline.rate_limit.max_attempts, 1);
    assert_eq!(cfg.pipeline.rate_limit.window_duration_ns, 1_000_000_000);
    assert_eq!(cfg.pipeline.rate_limit.max_tracked_uids, 1);
}

#[test]
fn test_daemon_toml_rejects_zero_evidence_retention_days() {
    assert_toml_rejected(
        "[pipeline]\nuse_mock_camera = true\n\n[pipeline.evidence]\nretention_days = 0\n",
        "retention_days",
    );
}

#[test]
fn test_daemon_toml_rejects_unparseable_log_level() {
    assert_toml_rejected("log_level = \"\"\n", "log_level");
    assert_toml_rejected("log_level = \"   \"\n", "log_level");
    assert_toml_rejected("log_level = \"debug,soos=notalevel\"\n", "log_level");
    let too_long = "a".repeat(MAX_LOG_LEVEL_LEN + 1);
    assert_toml_rejected(&format!("log_level = \"{too_long}\"\n"), "log_level");
}

#[test]
fn test_daemon_toml_accepts_valid_log_directives() {
    for level in ["info", "debug", "warn", "soos_daemon=debug,info"] {
        let cfg = assert_toml_accepted(&format!("log_level = \"{level}\"\n"));
        assert_eq!(cfg.log_level, level);
    }
}

#[test]
fn test_programmatic_unchecked_thresholds_fail_validate() {
    let mut cfg = DaemonConfig::default();
    cfg.pipeline.thresholds = soos_policy::ThresholdConfig::new_raw(0.0, 0.85);
    cfg.pipeline.vision.match_threshold = 0.0;
    assert_validate_rejected(&cfg, "match_threshold");

    let mut cfg = DaemonConfig::default();
    cfg.pipeline.thresholds = soos_policy::ThresholdConfig::new_raw(0.70, f32::NAN);
    cfg.pipeline.vision.pad_threshold = f32::NAN;
    assert_validate_rejected(&cfg, "pad_threshold");
}

#[test]
fn test_programmatic_vision_threshold_drift_fails_validate() {
    // The policy thresholds and the vision mirror must never disagree: the vision
    // pipeline would otherwise gate PAD on a value the operator never validated.
    let mut cfg = DaemonConfig::default();
    cfg.pipeline.vision.pad_threshold = 0.0;
    assert_validate_rejected(&cfg, "pad_threshold");
}

#[test]
fn test_invalid_socket_mode_fails_load_from_path_and_load_or_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("daemon.toml");
    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(b"[socket]\nsocket_mode = 438\n")
        .expect("write");
    drop(file);

    assert!(matches!(
        DaemonConfig::load_from_path(&path),
        Err(DaemonError::Config(_))
    ));
    assert!(matches!(
        DaemonConfig::load_or_default(Some(&path)),
        Err(DaemonError::Config(_))
    ));
}

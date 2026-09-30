//! Single effective `warmup_frames` default for the daemon (GitHub #205, review finding DMN-16).
//!
//! The running daemon must discard the same number of warm-up frames whether or not
//! `/etc/soos/daemon.toml` exists and whether or not it contains a `[pipeline]` table.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Test suite assertions"
)]

use soos_daemon::config::DAEMON_DEFAULT_WARMUP_FRAMES;
use soos_daemon::DaemonConfig;

#[test]
fn test_daemon_default_warmup_frames_constant_is_zero() {
    assert_eq!(
        DAEMON_DEFAULT_WARMUP_FRAMES, 0,
        "The daemon default keeps the file-based contract (instant wake)"
    );
}

#[test]
fn test_warmup_frames_identical_with_or_without_pipeline_table() {
    let without_table = DaemonConfig::from_toml_str("").unwrap();
    let with_table = DaemonConfig::from_toml_str("[pipeline]\n").unwrap();
    let other_table = DaemonConfig::from_toml_str("log_level = \"debug\"\n[socket]\n").unwrap();
    assert_eq!(
        without_table.pipeline.camera.warmup_frames, with_table.pipeline.camera.warmup_frames,
        "An empty daemon.toml and one with an empty [pipeline] table must agree"
    );
    assert_eq!(
        other_table.pipeline.camera.warmup_frames,
        DAEMON_DEFAULT_WARMUP_FRAMES
    );
}

#[test]
fn test_runtime_default_matches_file_based_config() {
    let runtime = DaemonConfig::runtime_default();
    let file_based = DaemonConfig::from_toml_str("[pipeline]\n").unwrap();
    assert_eq!(
        runtime.pipeline.camera.warmup_frames, file_based.pipeline.camera.warmup_frames,
        "No-config startup and file-based startup must use the same warmup_frames"
    );
    assert_eq!(
        runtime.pipeline.camera.warmup_frames,
        DAEMON_DEFAULT_WARMUP_FRAMES
    );
}

#[test]
fn test_load_or_default_without_system_file_uses_runtime_default() {
    let tmp = tempfile::tempdir().unwrap();
    let absent = tmp.path().join("daemon.toml");
    let config = DaemonConfig::load_or_default_with_system_path(None, &absent).unwrap();
    assert_eq!(
        config.pipeline.camera.warmup_frames, DAEMON_DEFAULT_WARMUP_FRAMES,
        "Startup without /etc/soos/daemon.toml must use the daemon runtime default"
    );
}

#[test]
fn test_load_or_default_reads_present_system_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("daemon.toml");
    std::fs::write(&path, "[pipeline]\nwarmup_frames = 7\n").unwrap();
    let config = DaemonConfig::load_or_default_with_system_path(None, &path).unwrap();
    assert_eq!(config.pipeline.camera.warmup_frames, 7);
}

#[test]
fn test_explicit_warmup_frames_override_is_kept() {
    let config = DaemonConfig::from_toml_str("[pipeline]\nwarmup_frames = 20\n").unwrap();
    assert_eq!(config.pipeline.camera.warmup_frames, 20);
}

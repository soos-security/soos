//! Contract tests of the `soos-lock-widget` binary without a compositor (GitHub #401; matrix
//! LKA13): a broken setup exits 1 after `SETUP_FAILURE_EXIT_DELAY_MS` with one stderr line and no
//! panic, so swaylock-plugin (which restarts its background program) cannot spin.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use soos_lock_widget::wayland::SETUP_FAILURE_EXIT_DELAY_MS;

const BIN: &str = env!("CARGO_BIN_EXE_soos-lock-widget");

fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

#[test]
fn test_lka13_exit_delay_constant() {
    assert_eq!(SETUP_FAILURE_EXIT_DELAY_MS, 1000);
}

#[test]
fn test_lka13_no_compositor_exits_1_after_the_delay_with_one_line() {
    let runtime = private_dir();
    let start = Instant::now();
    let out = Command::new(BIN)
        .env_clear()
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", "soos-test-no-such-compositor")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.starts_with("soos-lock-widget: "), "{stderr:?}");
    assert!(!stderr.contains("panicked"), "{stderr:?}");
    for state in ["Scanning", "Not recognized", "scanning", "not_recognized"] {
        assert!(
            !stderr.contains(state),
            "the state is never logged: {stderr:?}"
        );
    }
    assert!(
        elapsed >= Duration::from_millis(SETUP_FAILURE_EXIT_DELAY_MS),
        "{elapsed:?}"
    );
    assert!(elapsed < Duration::from_millis(SETUP_FAILURE_EXIT_DELAY_MS + 4000));
}

#[test]
fn test_lka13_invalid_wayland_socket_fd_exits_1_without_panic() {
    let runtime = private_dir();
    let out = Command::new(BIN)
        .env_clear()
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_SOCKET", "987")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("panicked"), "{stderr:?}");
}

#[test]
fn test_lka13_usage_error_is_reported() {
    let out = Command::new(BIN)
        .arg("--no-such-flag")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!out.stderr.is_empty());
}

#[test]
fn test_lka13_usage_error_waits_the_restart_delay() {
    let start = Instant::now();
    let out = Command::new(BIN)
        .arg("--interval-ms=not-a-number")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(start.elapsed() >= Duration::from_millis(SETUP_FAILURE_EXIT_DELAY_MS));
    let out = Command::new(BIN)
        .arg("--help")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!out.stdout.is_empty());
}

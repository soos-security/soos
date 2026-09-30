//! Contractual tests for the systemd readiness contract of `soos-daemon`
//! (GitHub #203, DMN-14 residual).
//!
//! With `Type=simple`, systemd considers the unit started as soon as the process is
//! forked, so `Before=display-manager.service` does not guarantee that the socket is
//! listening when the greeter shows its first prompt. `Type=notify` plus a `READY=1`
//! sent only after `bind_socket` closes that gap.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

/// Returns the values of every active `key=` directive in `section` of the unit.
fn unit_values(section: &str, key: &str) -> Vec<String> {
    let content =
        std::fs::read_to_string(workspace_root().join("packaging/soos-daemon.service")).unwrap();
    let mut current = String::new();
    let mut out = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current = line.to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if current == section && k.trim() == key {
                out.push(v.trim().to_string());
            }
        }
    }
    out
}

#[test]
fn test_unit_uses_notify_readiness() {
    assert_eq!(unit_values("[Service]", "Type"), vec!["notify".to_string()]);
    assert_eq!(
        unit_values("[Service]", "NotifyAccess"),
        vec!["main".to_string()],
        "Only the daemon main process may report readiness"
    );
}

#[test]
fn test_unit_start_timeout_is_explicit_and_bounded() {
    let values = unit_values("[Service]", "TimeoutStartSec");
    assert_eq!(values.len(), 1, "TimeoutStartSec= must be set exactly once");
    let secs: u64 = values[0]
        .trim_end_matches('s')
        .parse()
        .expect("TimeoutStartSec is a plain number of seconds");
    assert!(
        (10..=120).contains(&secs),
        "start timeout must leave room for model loading but stay bounded, got {secs}"
    );
}

#[test]
fn test_unit_keeps_display_manager_ordering() {
    assert!(unit_values("[Unit]", "Before")
        .iter()
        .any(|v| v.split_whitespace().any(|u| u == "display-manager.service")));
}

#[test]
fn test_daemon_reports_ready_only_after_socket_bind() {
    let main = std::fs::read_to_string(workspace_root().join("crates/daemon/src/main.rs")).unwrap();
    let bind = main
        .find("bind_socket(&config.socket)")
        .expect("main.rs binds the socket");
    let ready = main
        .find("sd_notify::notify_ready()")
        .expect("main.rs must send READY=1 (Type=notify)");
    let accept = main
        .find("listener.accept()")
        .expect("main.rs accepts connections");
    assert!(
        bind < ready && ready < accept,
        "READY=1 must be sent after the socket is bound and before the accept loop"
    );
    let stopping = main
        .find("sd_notify::notify_stopping()")
        .expect("main.rs must send STOPPING=1 on shutdown");
    assert!(stopping > accept, "STOPPING=1 belongs to the shutdown path");
}

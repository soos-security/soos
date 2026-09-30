//! Contract for the default `[dispatcher] connection_timeout_ms` (user decision 2026-09-30):
//! the daemon default is 2500 ms so that the GDM PAM line `timeout_ms=2500` really gets
//! daemon time, while console/sudo stacks stay capped by the 1000 ms PAM module default
//! through their client deadline.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and expect"
)]

use soos_daemon::config::{
    DaemonConfig, DEFAULT_CONNECTION_TIMEOUT_MS, MAX_CONNECTION_TIMEOUT_MS,
    MIN_CONNECTION_TIMEOUT_MS,
};
use soos_daemon::DispatcherConfig;
use std::path::Path;
use std::time::Duration;

/// Extracts the `timeout_ms=<n>` value of a PAM line held in a Rust source file.
fn timeout_ms_in_const(source: &str, const_name: &str) -> u64 {
    let line = source
        .lines()
        .find(|l| l.contains(&format!("pub const {const_name}")))
        .unwrap_or_else(|| panic!("{const_name} not found"));
    let value = line
        .split("timeout_ms=")
        .nth(1)
        .unwrap_or_else(|| panic!("{const_name} carries no timeout_ms="));
    value
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap()
}

#[test]
fn test_default_connection_timeout_is_2500_ms() {
    assert_eq!(DEFAULT_CONNECTION_TIMEOUT_MS, 2500);
    const {
        assert!(DEFAULT_CONNECTION_TIMEOUT_MS >= MIN_CONNECTION_TIMEOUT_MS);
        assert!(DEFAULT_CONNECTION_TIMEOUT_MS <= MAX_CONNECTION_TIMEOUT_MS);
    };
    assert_eq!(
        DispatcherConfig::default().connection_timeout,
        Duration::from_millis(DEFAULT_CONNECTION_TIMEOUT_MS)
    );
}

#[test]
fn test_every_load_path_uses_the_default_connection_timeout() {
    let expected = Duration::from_millis(DEFAULT_CONNECTION_TIMEOUT_MS);
    assert_eq!(
        DaemonConfig::runtime_default()
            .dispatcher
            .connection_timeout,
        expected
    );
    for toml in [
        "",
        "[dispatcher]\n",
        "[dispatcher]\nmax_concurrent_connections = 4\n",
    ] {
        let cfg = DaemonConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.dispatcher.connection_timeout, expected, "{toml:?}");
    }
    let explicit =
        DaemonConfig::from_toml_str("[dispatcher]\nconnection_timeout_ms = 1000\n").unwrap();
    assert_eq!(
        explicit.dispatcher.connection_timeout,
        Duration::from_millis(1000),
        "an explicit value is honored"
    );
}

#[test]
fn test_default_connection_timeout_covers_the_gdm_line_and_exceeds_the_pam_default() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let gdm = std::fs::read_to_string(root.join("crates/admin-cli/src/gdm.rs")).unwrap();
    let gdm_timeout = timeout_ms_in_const(&gdm, "GDM_PAM_LINE");
    assert!(
        DEFAULT_CONNECTION_TIMEOUT_MS >= gdm_timeout,
        "daemon default {DEFAULT_CONNECTION_TIMEOUT_MS} ms must give GDM its {gdm_timeout} ms"
    );

    let pam = std::fs::read_to_string(root.join("crates/pam/src/config.rs")).unwrap();
    let pam_default: u64 = pam
        .lines()
        .find(|l| l.contains("pub const DEFAULT_TIMEOUT_MS: u64"))
        .and_then(|l| l.split('=').nth(1))
        .map(|v| v.trim().trim_end_matches(';').replace('_', ""))
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        pam_default < DEFAULT_CONNECTION_TIMEOUT_MS,
        "sudo/console stay capped by the {pam_default} ms PAM module default"
    );
}

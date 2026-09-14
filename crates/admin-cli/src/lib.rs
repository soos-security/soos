//! `soos-admin-cli` — Non-biometric diagnostic and administrative CLI tool for Linux Biometric PAM.
//!
//! Provides administrative diagnostic commands:
//! - `status`: Query daemon readiness (socket, camera, models), PID, uptime, systemd unit status
//! - `test-pam`: Simulate PAM authentication cycle with latency breakdown and verdict reporting
//! - `logs`: Filtered view of daemon journal logs with automatic redaction of sensitive data

#![forbid(unsafe_code)]

pub mod args;
pub mod error;
pub mod logs;
pub mod redact;
pub mod status;
pub mod test_pam;

pub use args::{Cli, Commands, LogsArgs, OutputFormat, StatusArgs, TestPamArgs};
pub use error::AdminCliError;
pub use logs::fetch_and_filter_logs;
pub use redact::{default_redact, RedactionFilter};
pub use status::{query_status, DaemonStatusReport};
pub use test_pam::{simulate_pam_auth, PamTestReport};

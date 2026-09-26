//! `soos-admin-cli` — Non-biometric diagnostic and administrative CLI tool for Linux Biometric PAM.
//!
//! Provides administrative diagnostic commands:
//! - `status`: Query daemon readiness (socket, camera, models), PID, uptime, systemd unit status
//! - `test-pam`: Simulate PAM authentication cycle with latency breakdown and verdict reporting
//! - `logs`: Filtered view of daemon journal logs with automatic redaction of sensitive data

#![forbid(unsafe_code)]

pub mod args;
pub mod error;
pub mod gdm;
pub mod logs;
pub mod redact;
pub mod status;
pub mod test_pam;
pub mod user;

pub use args::{
    AddUserArgs, Cli, Commands, GdmAction, GdmArgs, LogsArgs, OutputFormat, StatusArgs, TestPamArgs,
};
pub use error::AdminCliError;
pub use gdm::{configure_gdm, get_gdm_status, GdmStatus};
pub use logs::fetch_and_filter_logs;
pub use redact::{default_redact, RedactionFilter};
pub use status::{query_status, DaemonStatusReport};
pub use test_pam::{simulate_pam_auth, PamTestReport};
pub use user::{
    add_user_to_group, add_user_to_group_with_runner, add_user_to_soos_group, validate_username,
    DEFAULT_SOOS_GROUP,
};

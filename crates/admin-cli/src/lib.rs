//! `soos-admin-cli` — Non-biometric diagnostic and administrative CLI tool for Linux Biometric PAM.
//!
//! Provides administrative diagnostic commands:
//! - `status`: Query daemon readiness (socket, camera, models), PID, uptime, systemd unit status
//! - `test-pam`: Simulate PAM authentication cycle with latency breakdown and verdict reporting
//! - `logs`: Filtered view of daemon journal logs with automatic redaction of sensitive data
//! - `camera list|probe`: V4L2 node metadata, classification and the shared resolver's
//!   decision (metadata ioctls only, never frames); `camera list` resolves with the
//!   `camera_device` / `sensor_preference` of `/etc/soos/daemon.toml`
//! - `camera ir-emitter probe|configure`: read-only UVC extension-unit scan of an IR node, and
//!   the `[camera.ir_emitter]` writer (device written only with `--test-on-device`; GitHub #378)
//! - `network-lock status|install|uninstall`: opt-in polkit Wi-Fi lock (GitHub #365); the
//!   status is read-only and never prints a network name

#![forbid(unsafe_code)]

pub mod args;
pub mod camera;
pub mod daemon_config;
pub mod error;
pub mod gdm;
pub mod ir_emitter;
pub mod logs;
pub mod network_lock;
mod pam_stack;
pub mod redact;
pub mod status;
pub mod test_pam;
pub mod user;

pub use args::{
    AddUserArgs, CameraAction, CameraArgs, CameraListArgs, CameraProbeArgs, Cli, Commands,
    GdmAction, GdmArgs, LogsArgs, NetworkLockAction, NetworkLockArgs, OutputFormat, StatusArgs,
    TestPamArgs,
};
pub use camera::{CameraEnvironment, CameraListReport, CameraProbeReport};
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

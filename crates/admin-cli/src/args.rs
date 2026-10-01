//! Command-line argument structures for `soos-admin`.

use clap::parser::ValueSource;
use clap::{ArgMatches, Parser, Subcommand, ValueEnum};
use soos_camera_v4l::{parse_sensor_preference, SensorPreference};

use crate::daemon_config::DEFAULT_DAEMON_CONFIG_PATH;
use std::path::PathBuf;

pub const DEFAULT_SOCKET_PATH: &str = "/run/soos/daemon.sock";
pub const DEFAULT_SYSTEMD_UNIT: &str = "soos-daemon";
pub const DEFAULT_TIMEOUT_MS: u64 = 250;
/// Lower bound of `test-pam --timeout-ms`, equal to the PAM module `MIN_TIMEOUT_MS`
/// (`crates/pam/src/config.rs`), so the diagnostic applies the same clamp as `pam_soos.so`.
pub const MIN_TIMEOUT_MS: u64 = 10;
/// Upper bound of `test-pam --timeout-ms`, equal to the PAM module `MAX_TIMEOUT_MS`.
pub const MAX_TIMEOUT_MS: u64 = 5000;
pub const DEFAULT_SERVICE: &str = "soos-admin";

/// Output format for diagnostic summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum OutputFormat {
    /// Human-readable aligned terminal table.
    #[default]
    Table,
    /// Machine-readable structured JSON.
    Json,
}

/// Top-level CLI configuration.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "soos-admin",
    about = "Non-biometric diagnostic and administrative CLI for soos local biometric PAM",
    version
)]
pub struct Cli {
    /// Path to daemon Unix Domain Socket.
    #[arg(long, global = true)]
    pub socket_path: Option<PathBuf>,

    /// Output formatting mode.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    #[command(subcommand)]
    pub command: Commands,
}

/// Available administrative diagnostic subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Query daemon health, process status, and systemd unit state.
    Status(StatusArgs),

    /// Simulate a PAM authentication cycle and benchmark latency.
    TestPam(TestPamArgs),

    /// Display and filter daemon journal logs with sensitive data redaction.
    Logs(LogsArgs),

    /// Add a user to the soos biometric authentication system group.
    AddUser(AddUserArgs),

    /// Manage GDM login PAM integration and disable flag.
    Gdm(GdmArgs),

    /// Inspect V4L2 camera nodes and the camera soos would select (metadata only, no frames).
    Camera(CameraArgs),
}

/// Arguments for the `camera` diagnostics command (GitHub #256).
#[derive(Parser, Debug, Clone)]
pub struct CameraArgs {
    /// Diagnostic to run.
    #[command(subcommand)]
    pub action: CameraAction,
}

/// `camera` diagnostics.
#[derive(Subcommand, Debug, Clone)]
pub enum CameraAction {
    /// List every V4L2 node with its capabilities, formats, frame sizes and classification,
    /// then the device the shared resolver selects and why.
    List(CameraListArgs),
    /// Show the capabilities, formats, frame sizes and classification of one device.
    Probe(CameraProbeArgs),
}

/// Parses the `daemon.toml` `sensor_preference` vocabulary for `--sensor-preference`.
fn parse_sensor_preference_arg(value: &str) -> Result<SensorPreference, String> {
    parse_sensor_preference(value)
        .ok_or_else(|| format!("'{value}' is not one of prefer_ir, ir, prefer_rgb, rgb, any"))
}

/// Arguments for `camera list`.
#[derive(Parser, Debug, Clone)]
pub struct CameraListArgs {
    /// Print machine-readable JSON (same as the global `--format json`).
    #[arg(long)]
    pub json: bool,

    /// Sensor preference to resolve with (`sensor_preference` of `/etc/soos/daemon.toml`).
    #[arg(long, value_parser = parse_sensor_preference_arg, default_value = "prefer_ir")]
    pub sensor_preference: SensorPreference,

    /// Explicit device (`camera_device` of `/etc/soos/daemon.toml`); overrides auto-detection.
    #[arg(long)]
    pub device: Option<PathBuf>,

    /// Daemon configuration whose `[pipeline] camera_device` and `sensor_preference` are used
    /// when `--device` / `--sensor-preference` are not given.
    #[arg(long, value_name = "PATH", default_value = DEFAULT_DAEMON_CONFIG_PATH)]
    pub config: PathBuf,
}

/// Whether `camera list --sensor-preference` was given on the command line (the clap default
/// `prefer_ir` must not override `sensor_preference` of the daemon configuration).
#[must_use]
pub fn camera_list_sensor_preference_given(matches: &ArgMatches) -> bool {
    matches
        .subcommand_matches("camera")
        .and_then(|camera| camera.subcommand_matches("list"))
        .and_then(|list| list.value_source("sensor_preference"))
        == Some(ValueSource::CommandLine)
}

/// Arguments for `camera probe`.
#[derive(Parser, Debug, Clone)]
pub struct CameraProbeArgs {
    /// Device to probe (`/dev/videoN` or a `/dev/v4l/by-id/` link).
    #[arg(value_name = "DEVICE")]
    pub device: PathBuf,

    /// Print machine-readable JSON (same as the global `--format json`).
    #[arg(long)]
    pub json: bool,
}

/// Action to perform for GDM integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GdmAction {
    /// Check current status of GDM integration and disable flag.
    Status,
    /// Enable GDM facial authentication.
    Enable,
    /// Disable GDM facial authentication (creates /etc/soos/gdm.disable).
    Disable,
    /// Restore the pristine PAM file from its `.soos-backup` copy and remove the backup.
    Restore,
}

/// Arguments for `gdm` management command.
#[derive(Parser, Debug, Clone)]
pub struct GdmArgs {
    /// Action to perform.
    #[arg(value_enum)]
    pub action: GdmAction,

    /// Path to PAM service file (defaults to /etc/pam.d/gdm-password).
    #[arg(long, default_value = "/etc/pam.d/gdm-password")]
    pub pam_file: PathBuf,

    /// Path to disable flag file (defaults to /etc/soos/gdm.disable).
    #[arg(long, default_value = "/etc/soos/gdm.disable")]
    pub disable_file: PathBuf,

    /// PAM module directory that must contain `pam_soos.so` before `enable` edits the
    /// PAM file (defaults to probing the distribution security directories).
    #[arg(long)]
    pub pam_module_dir: Option<PathBuf>,
}

/// Arguments for `add-user` subcommand.
#[derive(Parser, Debug, Clone)]
pub struct AddUserArgs {
    /// Target username to add to the `soos` group.
    #[arg(value_name = "USERNAME")]
    pub username: String,
}

/// Arguments for `status` subcommand.
#[derive(Parser, Debug, Clone, Default)]
pub struct StatusArgs {
    /// Systemd unit name to query.
    #[arg(long, default_value = DEFAULT_SYSTEMD_UNIT)]
    pub unit: String,
}

/// Arguments for `test-pam` subcommand.
#[derive(Parser, Debug, Clone, Default)]
pub struct TestPamArgs {
    /// Target UID to simulate authentication for (defaults to current process UID).
    #[arg(long)]
    pub uid: Option<u32>,

    /// Declared PAM service name.
    #[arg(long, default_value = DEFAULT_SERVICE)]
    pub service: String,

    /// Maximum timeout in milliseconds before failing closed, clamped to the PAM module
    /// range (10 to 5000 ms) exactly like the `timeout_ms=` argument of `pam_soos.so`.
    #[arg(long, default_value_t = DEFAULT_TIMEOUT_MS)]
    pub timeout_ms: u64,
}

/// Arguments for `logs` subcommand.
#[derive(Parser, Debug, Clone, Default)]
pub struct LogsArgs {
    /// Number of journal lines to display.
    #[arg(short = 'n', long, default_value_t = 50)]
    pub lines: usize,

    /// Follow log output dynamically.
    #[arg(short = 'f', long)]
    pub follow: bool,

    /// Filter by minimum priority level (e.g. emerg, alert, crit, err, warning, notice, info, debug).
    #[arg(short = 'p', long)]
    pub priority: Option<String>,

    /// Show entries not older than the specified time (e.g. "1 hour ago", "today").
    #[arg(long)]
    pub since: Option<String>,

    /// Target systemd unit name.
    #[arg(short = 'u', long, default_value = DEFAULT_SYSTEMD_UNIT)]
    pub unit: String,

    /// Optional direct log file path (for testing or non-systemd journal environments).
    #[arg(long)]
    pub file: Option<PathBuf>,
}

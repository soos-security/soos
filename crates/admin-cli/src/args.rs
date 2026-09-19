//! Command-line argument structures for `soos-admin`.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

pub const DEFAULT_SOCKET_PATH: &str = "/run/soos/daemon.sock";
pub const DEFAULT_SYSTEMD_UNIT: &str = "soos-daemon";
pub const DEFAULT_TIMEOUT_MS: u64 = 250;
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

    /// Maximum timeout in milliseconds before failing closed.
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

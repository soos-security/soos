//! Command-line argument parsing definitions using `clap`.

use clap::{Args, Parser, Subcommand, ValueEnum};
use nix::unistd::User;
use std::path::PathBuf;

use crate::error::EnrollmentCliError;

/// Output formats supported by diagnostic and query subcommands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable terminal table output.
    #[default]
    Table,
    /// Machine-readable JSON output.
    Json,
}

/// Root enrollment and diagnostic CLI tool for soos local biometric PAM.
#[derive(Parser, Debug)]
#[command(
    name = "soos-enroll",
    about = "Linux Biometric PAM root enrollment and diagnostic tool",
    version
)]
pub struct Cli {
    /// Path to the biometric templates directory (defaults to /var/lib/soos/biometrics)
    #[arg(long, global = true)]
    pub biometrics_dir: Option<PathBuf>,

    /// Path to the master key file (defaults to /var/lib/soos/master.key)
    #[arg(long, global = true)]
    pub key_file: Option<PathBuf>,

    /// Path to the neural models directory containing manifest.toml
    #[arg(long, global = true)]
    pub models_dir: Option<PathBuf>,

    /// Camera device path (e.g. /dev/video0 or /dev/v4l/by-id/...)
    #[arg(long, global = true)]
    pub camera_device: Option<PathBuf>,

    /// Skip root privilege check (useful for unprivileged testing and diagnostics)
    #[arg(long, global = true, hide = true)]
    pub skip_root_check: bool,

    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands for administrative operation.
#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Enroll a user with biometric facial template
    Enroll(EnrollArgs),
    /// Diagnostic verification of a user's enrolled template
    Verify(VerifyArgs),
    /// Delete an enrolled biometric template with secure erasure
    Delete(DeleteArgs),
    /// List all enrolled users and metadata
    List(ListArgs),
}

/// Arguments for `enroll` subcommand.
#[derive(Args, Debug, Clone)]
pub struct EnrollArgs {
    /// Target Linux User ID (UID). If not specified, defaults to caller UID.
    #[arg(short = 'i', long)]
    pub uid: Option<u32>,

    /// Target username (resolved to UID via system user database).
    #[arg(short = 'u', long, conflicts_with = "uid")]
    pub username: Option<String>,

    /// Number of frames to capture for quality selection (default: 5).
    #[arg(short, long, default_value_t = 5)]
    pub frames: usize,

    /// Automatically confirm enrollment without interactive prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Model identifier to record in template metadata (default: "mobilefacenet").
    #[arg(long, default_value = "mobilefacenet")]
    pub model_id: String,

    /// Model version to record in template metadata (default: "1.0.0").
    #[arg(long, default_value = "1.0.0")]
    pub model_version: String,
}

/// Arguments for `verify` subcommand.
#[derive(Args, Debug, Clone)]
pub struct VerifyArgs {
    /// Target Linux User ID (UID) to verify.
    #[arg(short = 'i', long)]
    pub uid: Option<u32>,

    /// Target username to verify.
    #[arg(short = 'u', long, conflicts_with = "uid")]
    pub username: Option<String>,
}

/// Arguments for `delete` subcommand.
#[derive(Args, Debug, Clone)]
pub struct DeleteArgs {
    /// Target Linux User ID (UID) to delete.
    #[arg(short = 'i', long)]
    pub uid: Option<u32>,

    /// Target username to delete.
    #[arg(short = 'u', long, conflicts_with = "uid")]
    pub username: Option<String>,

    /// Automatically confirm deletion without interactive prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

/// Arguments for `list` subcommand.
#[derive(Args, Debug, Clone, Default)]
pub struct ListArgs {
    /// Output format (table or json).
    #[arg(short = 'f', long, default_value = "table")]
    pub format: OutputFormat,
}

/// Resolves target UID from optional explicit UID, optional username, or defaults to current UID.
pub fn resolve_target_uid(
    uid: Option<u32>,
    username: Option<&str>,
) -> Result<u32, EnrollmentCliError> {
    if let Some(explicit_uid) = uid {
        return Ok(explicit_uid);
    }

    if let Some(user_name) = username {
        let user = User::from_name(user_name)?
            .ok_or_else(|| EnrollmentCliError::UserNotFound(user_name.to_string()))?;
        return Ok(user.uid.as_raw());
    }

    Ok(nix::unistd::getuid().as_raw())
}

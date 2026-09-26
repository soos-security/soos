//! Command-line argument parsing definitions using `clap`.

use clap::{Args, Parser, Subcommand, ValueEnum};
use nix::unistd::User;
use std::path::{Component, Path, PathBuf};

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

    /// Force mock camera and synthetic neural inference (for testing without hardware)
    #[arg(long, global = true)]
    pub mock: bool,

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
    /// Import and encrypt a biometric template from file
    Import(ImportArgs),
    /// Capture a frame, run vision pipeline, and output HTML debug visualization
    DebugVision,
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

/// Arguments for `import` subcommand.
#[derive(Args, Debug, Clone)]
pub struct ImportArgs {
    /// Target Linux User ID (UID). If not specified, defaults to caller UID.
    #[arg(short = 'i', long)]
    pub uid: Option<u32>,

    /// Target username (resolved to UID via system user database).
    #[arg(short = 'u', long, conflicts_with = "uid")]
    pub username: Option<String>,

    /// Path to input template file (CBOR or JSON float array).
    #[arg(short = 'f', long)]
    pub file: PathBuf,

    /// Facial recognition model identifier.
    #[arg(long, default_value = "arcface_w600k_mbf")]
    pub model_id: String,

    /// Attested model version.
    #[arg(long, default_value = "2.0.0")]
    pub model_version: String,
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

/// Validates and sanitizes a path to ensure it is absolute and contains no traversal (`..`) components.
pub fn sanitize_path(path: &Path) -> Result<PathBuf, EnrollmentCliError> {
    if !path.is_absolute() {
        return Err(EnrollmentCliError::InvalidPath(format!(
            "Path '{}' must be an absolute path",
            path.display()
        )));
    }

    for component in path.components() {
        if let Component::ParentDir = component {
            return Err(EnrollmentCliError::InvalidPath(format!(
                "Path traversal ('..') is strictly forbidden: '{}'",
                path.display()
            )));
        }
    }

    let mut clean = PathBuf::from("/");
    for component in path.components() {
        if let Component::Normal(c) = component {
            clean.push(c);
        }
    }

    Ok(clean)
}

/// Allowed FHS top-level directory prefixes for soos system assets.
pub const ALLOWED_FHS_PREFIXES: [&str; 8] = [
    "/var", "/run", "/etc", "/usr", "/tmp", "/dev", "/opt", "/home",
];

/// Validates that an absolute, sanitized path complies with standard FHS hierarchies.
pub fn validate_fhs_path(path: &Path) -> Result<PathBuf, EnrollmentCliError> {
    let clean = sanitize_path(path)?;

    let matches_fhs = ALLOWED_FHS_PREFIXES
        .iter()
        .any(|prefix| clean.starts_with(Path::new(prefix)));

    if !matches_fhs {
        return Err(EnrollmentCliError::InvalidPath(format!(
            "Path '{}' violates FHS hierarchy; must reside under permitted system prefixes: {:?}",
            clean.display(),
            ALLOWED_FHS_PREFIXES
        )));
    }

    Ok(clean)
}

/// Validates that a camera device path resides strictly under `/dev/` and is not `/dev` itself.
pub fn validate_camera_device_path(path: &Path) -> Result<PathBuf, EnrollmentCliError> {
    let clean = sanitize_path(path)?;

    if !clean.starts_with(Path::new("/dev")) || clean == Path::new("/dev") {
        return Err(EnrollmentCliError::InvalidPath(format!(
            "Camera device path '{}' must reside under /dev/",
            clean.display()
        )));
    }

    Ok(clean)
}

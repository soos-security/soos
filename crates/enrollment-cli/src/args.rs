//! Command-line argument parsing definitions using `clap`.

use clap::{Args, Parser, Subcommand, ValueEnum};
use nix::unistd::User;
use std::path::{Component, Path, PathBuf};

use crate::error::EnrollmentCliError;
use crate::service::{EMBEDDING_MODEL_VERSION, MODEL_ID_EMBEDDING};

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
    Import(ImportCommand),
    /// Capture a frame, run face detection, and write an HTML debug visualization
    DebugVision(DebugVisionArgs),
    /// Re-encrypt every legacy (v1) template and evidence snapshot to the AAD-bound v2 format
    Migrate(MigrateArgs),
}

/// Arguments for `migrate` subcommand (GitHub #287).
///
/// Legacy (version 1) files stay readable without migration; the command only re-encrypts
/// them with the AAD-bound version 2 envelope. Files already in version 2 are never rewritten.
#[derive(Args, Debug, Clone, Default)]
pub struct MigrateArgs {
    /// Report what would be migrated without writing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Output format of the summary (table or json).
    #[arg(short = 'f', long, default_value = "table")]
    pub format: OutputFormat,

    /// Evidence snapshot directory (defaults to /var/lib/soos/evidence).
    #[arg(long)]
    pub evidence_dir: Option<PathBuf>,

    /// Evidence key file (defaults to /var/lib/soos/evidence.key). A missing key means
    /// evidence was never enabled: evidence is skipped and no key is created.
    #[arg(long)]
    pub evidence_key_file: Option<PathBuf>,
}

/// Arguments for `debug-vision` subcommand.
///
/// The report is written atomically with mode `0600`; the raw camera frame
/// (biometric data) is embedded only when `--embed-frame` is passed explicitly.
#[derive(Args, Debug, Clone, Default)]
pub struct DebugVisionArgs {
    /// Absolute output path of the HTML report (default: a timestamped file under
    /// /var/lib/soos/debug). The file must not exist; symbolic links are refused.
    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,

    /// Embed the raw camera frame in the report. The frame is biometric data:
    /// without this flag only detection geometry (boxes, landmarks) is written.
    #[arg(long)]
    pub embed_frame: bool,
}

/// Arguments for `enroll` subcommand.
#[derive(Args, Debug, Clone)]
pub struct EnrollArgs {
    /// Target Linux User ID (UID). If neither --uid nor --username is given, the
    /// invoking user behind sudo/pkexec (SUDO_UID, PKEXEC_UID) is the target; root is
    /// never an implicit target and must be requested explicitly (--uid 0).
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

    /// Override of the embedding model identifier recorded in template metadata.
    /// Defaults to the loaded embedding extractor (`arcface_w600k_mbf`). The daemon
    /// refuses templates whose model identifier differs from its loaded extractor.
    #[arg(long, default_value = MODEL_ID_EMBEDDING)]
    pub model_id: String,

    /// Override of the model version recorded in template metadata. Defaults to the
    /// attested `models/manifest.toml` version.
    #[arg(long, default_value = EMBEDDING_MODEL_VERSION)]
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

    /// Input template (CBOR or JSON array of 512 finite floats, at most 64 KiB), or `-` to
    /// read it from standard input. A file must be a regular file (no symlink) and, under
    /// pkexec, owned by the invoking user (`PKEXEC_UID`).
    #[arg(short = 'f', long, value_name = "FILE|-")]
    pub file: PathBuf,

    /// Facial recognition model identifier.
    #[arg(long, default_value = "arcface_w600k_mbf")]
    pub model_id: String,

    /// Attested model version.
    #[arg(long, default_value = "2.0.0")]
    pub model_version: String,
}

/// `import` subcommand: the template source ([`ImportArgs`]) plus the overwrite confirmation.
///
/// `--yes` lives here rather than in [`ImportArgs`] so that the template-source arguments stay
/// a stable library type (GitHub #237). The wrapper dereferences to [`ImportArgs`].
#[derive(Args, Debug, Clone)]
pub struct ImportCommand {
    /// Template source and target.
    #[command(flatten)]
    pub args: ImportArgs,

    /// Replace an already enrolled template for the target user. Without it, importing onto
    /// an enrolled user from a file fails with "already enrolled".
    #[arg(short = 'y', long)]
    pub yes: bool,
}

impl std::ops::Deref for ImportCommand {
    type Target = ImportArgs;

    fn deref(&self) -> &ImportArgs {
        &self.args
    }
}

/// Resolves the target UID from an explicit UID, a username, or the invoking user.
///
/// Explicit values always win (including `--uid 0` for an intentional root
/// enrollment). Without either, the target is derived by
/// [`resolve_default_target_uid`] from the real UID and the `SUDO_UID` /
/// `PKEXEC_UID` variables set by `sudo` and `pkexec` (GitHub #184 / STO-11).
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

    // A non-UTF-8 value is mapped to a non-numeric marker so it fails closed below.
    let sudo_uid = std::env::var_os("SUDO_UID");
    let pkexec_uid = std::env::var_os("PKEXEC_UID");
    resolve_default_target_uid(
        nix::unistd::getuid().as_raw(),
        sudo_uid
            .as_deref()
            .map(|v| v.to_str().unwrap_or("\u{fffd}")),
        pkexec_uid
            .as_deref()
            .map(|v| v.to_str().unwrap_or("\u{fffd}")),
    )
}

/// Maximum accepted length of an invoker UID environment value (`u32::MAX` has 10 digits).
const MAX_INVOKER_UID_LEN: usize = 10;

/// Resolves the implicit enrollment target when neither `--uid` nor `--username` is given.
///
/// - A non-root real UID is its own target (environment variables are ignored).
/// - A root real UID uses `SUDO_UID`, else `PKEXEC_UID`, when it names a non-root user.
/// - Otherwise the call fails with [`EnrollmentCliError::TargetUserRequired`]: root is
///   never enrolled implicitly and must be requested with `--uid 0` / `--username root`.
/// - A present but malformed invoker UID fails closed with
///   [`EnrollmentCliError::InvalidInvokerUid`].
pub fn resolve_default_target_uid(
    real_uid: u32,
    sudo_uid: Option<&str>,
    pkexec_uid: Option<&str>,
) -> Result<u32, EnrollmentCliError> {
    if real_uid != 0 {
        return Ok(real_uid);
    }

    for (name, value) in [("SUDO_UID", sudo_uid), ("PKEXEC_UID", pkexec_uid)] {
        let Some(raw) = value else {
            continue;
        };
        let invoker = parse_invoker_uid(raw)
            .ok_or_else(|| EnrollmentCliError::InvalidInvokerUid(name.to_string()))?;
        if invoker != 0 {
            return Ok(invoker);
        }
    }

    Err(EnrollmentCliError::TargetUserRequired)
}

/// Parses a decimal UID strictly (ASCII digits only, bounded length, fits in `u32`).
fn parse_invoker_uid(raw: &str) -> Option<u32> {
    if raw.is_empty() || raw.len() > MAX_INVOKER_UID_LEN || !raw.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    raw.parse::<u32>().ok()
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

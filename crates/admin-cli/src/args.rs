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

    /// Opt-in polkit Wi-Fi lock: status, install and uninstall (GitHub #365).
    NetworkLock(NetworkLockArgs),
}

/// Arguments for the `network-lock` command (GitHub #365).
#[derive(Parser, Debug, Clone)]
pub struct NetworkLockArgs {
    /// Action to perform.
    #[command(subcommand)]
    pub action: NetworkLockAction,
}

/// `network-lock` actions.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum NetworkLockAction {
    /// Read-only diagnostic: backend, rule files, rfkill access (never prints a network name).
    Status {
        /// Root directory to inspect (a staging copy in tests; default `/`).
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Write the polkit rule as root:root 0644; with `--rfkill` also the udev rule that
    /// removes seat-user access to /dev/rfkill (without it, an installed soos rfkill rule is
    /// removed). Requires root unless `--root` names a staging directory.
    Install {
        /// Also install the rfkill udev rule (off by default; disables the airplane-mode toggle).
        #[arg(long)]
        rfkill: bool,
        /// Root directory to write below (a staging directory in tests; default `/`).
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Remove the polkit rule and the rfkill udev rule.
    Uninstall {
        /// Root directory to remove from (a staging directory in tests; default `/`).
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
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
    /// UVC extension-unit IR emitter control: read-only probe, or write `[camera.ir_emitter]`
    /// (GitHub #378). Root only; touches Infrared uvcvideo nodes only.
    IrEmitter(IrEmitterArgs),
}

/// Arguments for `camera ir-emitter` (GitHub #378).
#[derive(Parser, Debug, Clone)]
pub struct IrEmitterArgs {
    /// Action to perform.
    #[command(subcommand)]
    pub action: IrEmitterAction,
}

/// `camera ir-emitter` actions.
#[derive(Subcommand, Debug, Clone)]
pub enum IrEmitterAction {
    /// Read-only scan of the extension-unit controls of an IR node (GET requests only).
    Probe(IrEmitterProbeArgs),
    /// Write `[camera.ir_emitter]` to the daemon configuration; the device is written only
    /// with `--test-on-device`.
    Configure(IrEmitterConfigureArgs),
}

/// `[camera.ir_emitter] mode` on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum IrEmitterModeArg {
    /// soos never touches the emitter (default).
    #[default]
    Off,
    /// soos may drive the emitter (requires a successful `--test-on-device`).
    Auto,
}

/// A control payload given as hexadecimal (`0x` prefix optional), 1 to 64 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexPayload(pub Vec<u8>);

/// Parses `--on` / `--off`.
fn parse_hex_payload(value: &str) -> Result<HexPayload, String> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    let max_digits = soos_camera_v4l::uvc_xu::XU_MAX_PAYLOAD_BYTES.saturating_mul(2);
    if digits.is_empty()
        || !digits.len().is_multiple_of(2)
        || digits.len() > max_digits
        || !digits.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(format!(
            "expected an even number of hexadecimal digits (2 to {max_digits})"
        ));
    }
    digits
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|p| u8::from_str_radix(p, 16).ok())
                .ok_or_else(|| "invalid hexadecimal digit".to_string())
        })
        .collect::<Result<Vec<u8>, String>>()
        .map(HexPayload)
}

/// Arguments for `camera ir-emitter probe`.
#[derive(Parser, Debug, Clone)]
pub struct IrEmitterProbeArgs {
    /// IR device (`/dev/videoN` or a `/dev/v4l/by-id/` link).
    #[arg(value_name = "DEVICE")]
    pub device: PathBuf,

    /// Scan this extension unit only (default: units 1 to 32).
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=255))]
    pub unit: Option<u8>,

    /// Daemon configuration whose `[camera.ir_emitter]` is compared with this node.
    #[arg(long, value_name = "PATH", default_value = DEFAULT_DAEMON_CONFIG_PATH)]
    pub config: PathBuf,

    /// Print machine-readable JSON (same as the global `--format json`).
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `camera ir-emitter configure`.
#[derive(Parser, Debug, Clone)]
pub struct IrEmitterConfigureArgs {
    /// IR device (`/dev/videoN` or a `/dev/v4l/by-id/` link).
    #[arg(value_name = "DEVICE")]
    pub device: PathBuf,

    /// Extension unit id.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=255))]
    pub unit: u8,

    /// Control selector.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=255))]
    pub selector: u8,

    /// Payload that lights the emitter (hexadecimal, 1 to 64 bytes).
    #[arg(long, value_parser = parse_hex_payload)]
    pub on: HexPayload,

    /// Payload that switches the emitter off (hexadecimal, same length as `--on`).
    #[arg(long, value_parser = parse_hex_payload)]
    pub off: HexPayload,

    /// `mode` written to the section (`auto` requires `--test-on-device`).
    #[arg(long, value_enum, default_value_t = IrEmitterModeArg::Off)]
    pub mode: IrEmitterModeArg,

    /// Daemon configuration to update.
    #[arg(long, value_name = "PATH", default_value = DEFAULT_DAEMON_CONFIG_PATH)]
    pub config: PathBuf,

    /// Print the section without writing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Write the on and off payloads to the device, read them back and restore the original
    /// value (the only device write of this command).
    #[arg(long)]
    pub test_on_device: bool,
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

    /// `restore` only: restore the backup even when the PAM file was changed after
    /// `gdm enable` (the changes are lost). Without it a stale backup is refused.
    #[arg(long)]
    pub force: bool,
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

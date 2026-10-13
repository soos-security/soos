//! Reading and changing the state of each soos feature for the onboarding tour (GitHub #379).
//!
//! Every change is an explicit user click carried out off the UI thread by [`FeatureRunner`]:
//!
//! - root-owned state only through `pkexec` with fixed argument vectors and programs named by
//!   absolute path: the two documented flag files ([`SystemFlag`], a closed set) and the
//!   existing `soos-admin network-lock install|uninstall` (never `--rfkill`, never `--root`);
//! - the user companion through the user's own systemd manager (`systemctl --user`), never as
//!   root;
//! - `soos-remote` keys are only ever turned off, by an exact line edit verified by a TOML parse
//!   and written atomically (mode 0600, never through a symbolic link).
//!
//! The GUI process itself never writes a root-owned file. Probes only read.

#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::onboarding::{
    read_user_file_bounded, soos_config_file, FeatureSnapshot, RemoteConfigState, RemoteKeys,
    UnitState, UserFileError,
};
use crate::privileged::{
    read_bounded, Notify, PKEXEC_PROGRAM, RM_PROGRAM, SOOS_ADMIN_PROGRAM, SYSTEMCTL_PROGRAM,
    TOUCH_PROGRAM,
};

/// Directory of the soos flag files.
pub const SOOS_ETC_DIR: &str = "/etc/soos";
/// Global flag: stops face unlock and presence auto-unlock (same name as `soos-daemon`).
pub const GLOBAL_DISABLE_FLAG: &str = "disabled";
/// Presence flag: stops presence auto-unlock only (same name as `soos-daemon`).
pub const PRESENCE_DISABLE_FLAG: &str = "presence.disable";
/// The Wi-Fi lock polkit rule installed by `soos-admin network-lock install`.
pub const NETWORK_LOCK_RULE_PATH: &str = "/etc/polkit-1/rules.d/49-soos-network-lock.rules";
/// The optional rfkill udev rule of the Wi-Fi lock (`network-lock install --rfkill`).
pub const NETWORK_LOCK_RFKILL_PATH: &str = "/etc/udev/rules.d/72-soos-network-lock-rfkill.rules";
/// The daemon configuration read for `[presence]` and `[preview]` (never written by the GUI).
pub const DAEMON_CONFIG_PATH: &str = "/etc/soos/daemon.toml";
/// The phone companion user unit.
pub const REMOTE_UNIT: &str = "soos-remote.service";
/// Largest `remote.toml` read (the `soos-remote` limit).
pub const MAX_REMOTE_CONFIG_BYTES: u64 = 16_384;
/// Largest `systemctl --user show` output read.
pub const MAX_UNIT_SHOW_BYTES: usize = 1024;
/// Deadline of every `systemctl --user` call (the child is killed and reaped after it).
pub const USER_SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(3);
/// Poll interval while waiting for a child under a deadline.
const CHILD_POLL: Duration = Duration::from_millis(10);

// ---------------------------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------------------------

/// A root-owned flag file. This is the complete set of root paths the GUI ever asks to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemFlag {
    /// `/etc/soos/disabled`.
    Global,
    /// `/etc/soos/presence.disable`.
    Presence,
}

impl SystemFlag {
    /// Absolute path of the flag.
    pub fn path(self) -> PathBuf {
        Path::new(SOOS_ETC_DIR).join(self.file_name())
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::Global => GLOBAL_DISABLE_FLAG,
            Self::Presence => PRESENCE_DISABLE_FLAG,
        }
    }
}

/// A `soos-remote` feature the GUI can turn off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSwitch {
    /// `allow_unlock`.
    Unlock,
    /// `allow_funnel`.
    Funnel,
    /// `push_notifications`.
    Push,
    /// `camera_view`.
    CameraView,
}

impl RemoteSwitch {
    /// The key of the feature.
    pub fn key(self) -> &'static str {
        match self {
            Self::Unlock => "allow_unlock",
            Self::Funnel => "allow_funnel",
            Self::Push => "push_notifications",
            Self::CameraView => "camera_view",
        }
    }

    /// Keys set to `false` when the feature is turned off: the key itself, then the keys
    /// `soos-remote` refuses without it (`camera_view_funnel` needs `camera_view` and
    /// `allow_funnel`).
    pub fn keys_to_clear(self) -> &'static [&'static str] {
        match self {
            Self::Unlock => &["allow_unlock"],
            Self::Funnel => &["allow_funnel", "camera_view_funnel"],
            Self::Push => &["push_notifications"],
            Self::CameraView => &["camera_view", "camera_view_funnel"],
        }
    }
}

/// A request carried out by [`FeatureRunner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureAction {
    /// Probe every feature (never writes).
    Refresh,
    /// Create (`present`) or remove a root flag through `pkexec`.
    SetSystemFlag {
        /// The flag.
        flag: SystemFlag,
        /// `true` creates the flag (feature off), `false` removes it.
        present: bool,
    },
    /// Enable and start, or disable and stop, the companion user unit.
    SetCompanion {
        /// Target state.
        enabled: bool,
    },
    /// Turn a `soos-remote` feature off in `remote.toml`, then restart the companion.
    DisableRemote(RemoteSwitch),
    /// Install or remove the Wi-Fi lock through `pkexec soos-admin network-lock`.
    SetNetworkLock {
        /// Target state.
        installed: bool,
    },
}

impl FeatureAction {
    /// Whether the action opens a Polkit dialog.
    pub fn is_privileged(self) -> bool {
        matches!(
            self,
            Self::SetSystemFlag { .. } | Self::SetNetworkLock { .. }
        )
    }
}

/// Result of a [`FeatureAction`].
#[derive(Debug)]
pub enum FeatureOutcome {
    /// Result of [`FeatureAction::Refresh`].
    Refreshed(FeatureSnapshot),
    /// Result of any other action.
    Applied {
        /// The action.
        action: FeatureAction,
        /// User-facing error message on failure (never file contents).
        result: Result<(), String>,
    },
}

// ---------------------------------------------------------------------------------------------
// Fixed argument vectors
// ---------------------------------------------------------------------------------------------

/// `pkexec touch <flag>` (create) or `pkexec rm -f -- <flag>` (remove).
pub fn system_flag_command(flag: SystemFlag, present: bool) -> (&'static str, Vec<String>) {
    let path = flag.path().to_string_lossy().into_owned();
    let args = if present {
        vec![TOUCH_PROGRAM.to_string(), path]
    } else {
        vec![
            RM_PROGRAM.to_string(),
            "-f".to_string(),
            "--".to_string(),
            path,
        ]
    };
    (PKEXEC_PROGRAM, args)
}

/// `pkexec soos-admin network-lock install|uninstall`.
pub fn network_lock_command(installed: bool) -> (&'static str, Vec<String>) {
    let verb = if installed { "install" } else { "uninstall" };
    (
        PKEXEC_PROGRAM,
        vec![
            SOOS_ADMIN_PROGRAM.to_string(),
            "network-lock".to_string(),
            verb.to_string(),
        ],
    )
}

fn user_systemctl(args: &[&str]) -> (&'static str, Vec<String>) {
    (
        SYSTEMCTL_PROGRAM,
        std::iter::once("--user")
            .chain(args.iter().copied())
            .map(str::to_string)
            .collect(),
    )
}

/// `systemctl --user enable|disable --now soos-remote.service`.
pub fn companion_command(enabled: bool) -> (&'static str, Vec<String>) {
    let verb = if enabled { "enable" } else { "disable" };
    user_systemctl(&[verb, "--now", REMOTE_UNIT])
}

/// `systemctl --user try-restart soos-remote.service` (restarts only a running unit).
pub fn companion_restart_command() -> (&'static str, Vec<String>) {
    user_systemctl(&["try-restart", REMOTE_UNIT])
}

/// `systemctl --user show --property=LoadState --property=ActiveState soos-remote.service`.
pub fn unit_show_command() -> (&'static str, Vec<String>) {
    user_systemctl(&[
        "show",
        "--property=LoadState",
        "--property=ActiveState",
        REMOTE_UNIT,
    ])
}

/// Maps `systemctl show` `Key=value` lines to a unit state.
pub fn parse_unit_show(text: &str) -> UnitState {
    let mut load = None;
    let mut active = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("LoadState=") {
            load = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("ActiveState=") {
            active = Some(value.trim());
        }
    }
    match load {
        Some("not-found") => return UnitState::NotInstalled,
        Some("masked") => return UnitState::Masked,
        _ => {}
    }
    match active {
        Some("inactive" | "failed") => UnitState::Inactive,
        Some("active" | "activating" | "reloading" | "refreshing" | "deactivating") => {
            UnitState::Active
        }
        _ => UnitState::Unknown,
    }
}

// ---------------------------------------------------------------------------------------------
// remote.toml
// ---------------------------------------------------------------------------------------------

/// Path of `remote.toml`, resolved like `soos-remote` does.
pub fn remote_config_path(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    soos_config_file(xdg_config_home, home, "remote.toml")
}

const REMOTE_KEY_NAMES: [&str; 6] = [
    "allow_unlock",
    "allow_funnel",
    "password_alerts",
    "push_notifications",
    "camera_view",
    "camera_view_funnel",
];

/// Reads the tour keys of `remote.toml` text. A missing key is `false`; a non-boolean value or a
/// TOML error is [`RemoteConfigState::Invalid`].
pub fn parse_remote_keys(text: &str) -> RemoteConfigState {
    let Ok(table) = text.parse::<toml::Table>() else {
        return RemoteConfigState::Invalid;
    };
    let mut values = [false; 6];
    for (slot, key) in values.iter_mut().zip(REMOTE_KEY_NAMES) {
        match table.get(key) {
            None => {}
            Some(toml::Value::Boolean(b)) => *slot = *b,
            Some(_) => return RemoteConfigState::Invalid,
        }
    }
    let [allow_unlock, allow_funnel, password_alerts, push_notifications, camera_view, camera_view_funnel] =
        values;
    RemoteConfigState::Keys(RemoteKeys {
        allow_unlock,
        allow_funnel,
        password_alerts,
        push_notifications,
        camera_view,
        camera_view_funnel,
    })
}

/// Reads `path` as UTF-8, bounded, never through a symbolic link and never blocking.
/// `Ok(None)` = not found.
fn read_user_text(path: &Path) -> Result<Option<String>, String> {
    let bytes = read_user_file_bounded(path, MAX_REMOTE_CONFIG_BYTES).map_err(|e| match e {
        UserFileError::NotARegularFile => {
            "remote.toml is not a regular file (symbolic links are refused)".to_string()
        }
        UserFileError::TooLarge => "remote.toml is too large".to_string(),
        UserFileError::Unreadable(kind) => format!("cannot read remote.toml ({kind})"),
    })?;
    bytes
        .map(|bytes| String::from_utf8(bytes).map_err(|_| "remote.toml is not UTF-8".to_string()))
        .transpose()
}

/// Reads `[presence] enabled` (default `true`) and whether `[preview]` allows the remote camera
/// view for `uid` (`enabled`, `remote_view` and `uid` in `allowed_uids`; default `false`) from
/// `daemon.toml` text. A key of the wrong type, or invalid TOML, gives `None` for that value.
pub fn parse_daemon_keys(text: &str, uid: u32) -> (Option<bool>, Option<bool>) {
    let Ok(table) = text.parse::<toml::Table>() else {
        return (None, None);
    };
    let section = |name: &str| table.get(name).and_then(toml::Value::as_table);
    let bool_key = |name: &str, key: &str, default: bool| -> Option<bool> {
        match section(name).and_then(|t| t.get(key)) {
            None => Some(default),
            Some(toml::Value::Boolean(b)) => Some(*b),
            Some(_) => None,
        }
    };
    let presence = if table.get("presence").is_some_and(|v| !v.is_table()) {
        None
    } else {
        bool_key("presence", "enabled", true)
    };
    let remote_view = (|| {
        if table.get("preview").is_some_and(|v| !v.is_table()) {
            return None;
        }
        let enabled = bool_key("preview", "enabled", false)?;
        let remote = bool_key("preview", "remote_view", false)?;
        let allowed = match section("preview").and_then(|t| t.get("allowed_uids")) {
            None => false,
            Some(toml::Value::Array(uids)) => {
                uids.iter().any(|v| v.as_integer() == Some(i64::from(uid)))
            }
            Some(_) => return None,
        };
        Some(enabled && remote && allowed)
    })();
    (presence, remote_view)
}

/// Reads the daemon keys through the daemon's own bounded reader. A missing file means the
/// daemon defaults; any other error is unknown.
fn read_daemon_keys(path: &Path, uid: u32) -> (Option<bool>, Option<bool>) {
    use soos_camera_v4l::daemon_config::{read_daemon_config_text, DaemonConfigError};
    match read_daemon_config_text(path) {
        Ok(text) => parse_daemon_keys(&text, uid),
        Err(DaemonConfigError::NotFound) => (Some(true), Some(false)),
        Err(_) => (None, None),
    }
}

/// Reads `remote.toml` at `path` (bounded; a symbolic link, an oversized or unreadable file is
/// [`RemoteConfigState::Invalid`]).
pub fn read_remote_config(path: &Path) -> RemoteConfigState {
    match read_user_text(path) {
        Ok(None) => RemoteConfigState::Missing,
        Ok(Some(text)) => parse_remote_keys(&text),
        Err(_) => RemoteConfigState::Invalid,
    }
}

/// Why `remote.toml` was not edited.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoteEditError {
    /// Tables are not part of the `soos-remote` schema.
    #[error("remote.toml uses a layout the GUI does not edit (tables)")]
    UnsupportedLayout,
    /// The key has a value other than `true` / `false` on its line.
    #[error("remote.toml sets `{0}` to a value that is not true or false")]
    UnsupportedValue(&'static str),
    /// The file is not valid TOML.
    #[error("remote.toml is not valid TOML")]
    Invalid,
    /// The edited text did not verify (nothing written).
    #[error("the edited remote.toml did not verify")]
    VerifyFailed,
}

/// Splits `<ws>key<ws>=<ws>rest` and returns `(indent, key, rest)` for a bare key line.
fn split_key_line(line: &str) -> Option<(&str, &str, &str)> {
    let trimmed = line.trim_start();
    let indent = line.get(..line.len().saturating_sub(trimmed.len()))?;
    let eq = trimmed.find('=')?;
    let key = trimmed.get(..eq)?.trim_end();
    if key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    let rest = trimmed.get(eq.saturating_add(1)..)?.trim_start();
    Some((indent, key, rest))
}

/// Value of `rest` when it is `true`/`false` optionally followed by a `#` comment; returns the
/// boolean and the comment part (with its leading whitespace).
fn bool_with_comment(rest: &str) -> Option<(bool, &str)> {
    let (value, tail) = match rest.strip_prefix("true") {
        Some(tail) => (true, tail),
        None => (false, rest.strip_prefix("false")?),
    };
    let after = tail.trim_start();
    if after.is_empty() || after.starts_with('#') {
        Some((value, tail.trim_end_matches(['\r'])))
    } else {
        None
    }
}

/// Turns `keys` off in `remote.toml` text by rewriting each `key = true` line to
/// `key = false`. Returns `Ok(None)` when nothing changes; never adds a key and never touches
/// another line. The result is verified by a TOML parse: every target key reads `false` or is
/// absent and every other key is unchanged.
///
/// # Errors
///
/// [`RemoteEditError`] when the text is not valid TOML, uses tables, sets a target key to a
/// value the edit does not handle, or the edited text does not verify.
pub fn disable_remote_keys(
    text: &str,
    keys: &[&'static str],
) -> Result<Option<String>, RemoteEditError> {
    let original: toml::Table = text.parse().map_err(|_| RemoteEditError::Invalid)?;
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    for line in text.split_inclusive('\n') {
        let (body, newline) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        if body.trim_start().starts_with('[') {
            return Err(RemoteEditError::UnsupportedLayout);
        }
        let target = split_key_line(body).and_then(|(indent, key, rest)| {
            keys.iter().find(|k| **k == key).map(|k| (indent, *k, rest))
        });
        match target {
            Some((indent, key, rest)) => match bool_with_comment(rest) {
                Some((true, comment)) => {
                    out.push_str(indent);
                    out.push_str(key);
                    out.push_str(" = false");
                    out.push_str(comment);
                    out.push_str(newline);
                    changed = true;
                }
                Some((false, _)) => out.push_str(line),
                None => return Err(RemoteEditError::UnsupportedValue(key)),
            },
            None => out.push_str(line),
        }
    }
    let edited: toml::Table = out.parse().map_err(|_| RemoteEditError::VerifyFailed)?;
    for key in keys {
        match edited.get(*key) {
            None | Some(toml::Value::Boolean(false)) => {}
            Some(_) => return Err(RemoteEditError::VerifyFailed),
        }
    }
    let others_equal = original
        .iter()
        .filter(|(k, _)| !keys.contains(&k.as_str()))
        .all(|(k, v)| edited.get(k) == Some(v))
        && edited.keys().all(|k| original.contains_key(k));
    if !others_equal {
        return Err(RemoteEditError::VerifyFailed);
    }
    if changed {
        Ok(Some(out))
    } else {
        Ok(None)
    }
}

/// Writes `contents` over the user file `path` atomically (fresh 0600 temporary file in the same
/// directory, `fsync`, rename). Refuses a symbolic link and a missing parent directory.
///
/// # Errors
///
/// A user-facing message carrying the error kind, never the contents.
pub fn write_user_file_atomically(path: &Path, contents: &str) -> Result<(), String> {
    crate::onboarding::write_atomically(path, contents.as_bytes())
        .map_err(|e| format!("cannot write {} ({})", display_name(path), e.kind()))
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "the file".to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

// ---------------------------------------------------------------------------------------------
// Programs
// ---------------------------------------------------------------------------------------------

/// Output of a finished program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramOutput {
    /// The program exited with status 0.
    pub success: bool,
    /// Exit code, if any.
    pub code: Option<i32>,
    /// Standard output, at most the requested bound.
    pub stdout: Vec<u8>,
}

/// Runs programs (injectable for tests).
pub trait ProgramRunner: Send + Sync {
    /// Runs `program args` with stdin and stderr closed. `deadline = None` waits for completion
    /// (`pkexec`: the user may take long in the Polkit dialog); `Some(d)` kills and reaps the
    /// child after `d`. At most `max_stdout` bytes of standard output are kept.
    ///
    /// # Errors
    ///
    /// A user-facing message when the program cannot run or exceeds its deadline.
    fn run(
        &self,
        program: &str,
        args: &[String],
        deadline: Option<Duration>,
        max_stdout: usize,
    ) -> Result<ProgramOutput, String>;
}

/// Production [`ProgramRunner`] (`std::process::Command`, no shell).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProgramRunner;

impl ProgramRunner for SystemProgramRunner {
    fn run(
        &self,
        program: &str,
        args: &[String],
        deadline: Option<Duration>,
        max_stdout: usize,
    ) -> Result<ProgramOutput, String> {
        let capture = max_stdout > 0;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(if capture {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                format!(
                    "failed to run {} ({})",
                    display_name(Path::new(program)),
                    e.kind()
                )
            })?;
        let status = match deadline {
            None => child
                .wait()
                .map_err(|e| format!("failed to wait ({})", e.kind()))?,
            Some(limit) => {
                let start = Instant::now();
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => break status,
                        Ok(None) if start.elapsed() < limit => thread::sleep(CHILD_POLL),
                        Ok(None) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(format!(
                                "{} did not answer within {} ms",
                                display_name(Path::new(program)),
                                limit.as_millis()
                            ));
                        }
                        Err(e) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(format!("failed to wait ({})", e.kind()));
                        }
                    }
                }
            }
        };
        // Outputs read here are tiny (a few short lines) and the child has exited: the pipe
        // holds everything it wrote. The read is still bounded.
        let stdout = match child.stdout.take() {
            Some(out) => read_bounded(out, max_stdout).unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(ProgramOutput {
            success: status.success(),
            code: status.code(),
            stdout,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------------------------

/// Paths and identity the executor works with (temporary directories in tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeaturePaths {
    /// Directory of the flag files (`/etc/soos`).
    pub etc_dir: PathBuf,
    /// Wi-Fi lock rule file.
    pub network_lock_rule: PathBuf,
    /// rfkill udev rule of the Wi-Fi lock.
    pub network_lock_rfkill: PathBuf,
    /// `daemon.toml`.
    pub daemon_config: PathBuf,
    /// `remote.toml` of the user, if a configuration directory is known.
    pub remote_config: Option<PathBuf>,
    /// The GUI runs as root (effective uid 0).
    pub running_as_root: bool,
    /// Real uid of the GUI user (matched against `[preview] allowed_uids`).
    pub uid: u32,
}

impl FeaturePaths {
    /// Production paths: the constants, the user's environment and the effective uid.
    pub fn system() -> Self {
        Self {
            etc_dir: PathBuf::from(SOOS_ETC_DIR),
            network_lock_rule: PathBuf::from(NETWORK_LOCK_RULE_PATH),
            network_lock_rfkill: PathBuf::from(NETWORK_LOCK_RFKILL_PATH),
            daemon_config: PathBuf::from(DAEMON_CONFIG_PATH),
            remote_config: remote_config_path(
                std::env::var_os("XDG_CONFIG_HOME").as_deref(),
                std::env::var_os("HOME").as_deref(),
            ),
            running_as_root: nix::unistd::geteuid().is_root(),
            uid: nix::unistd::getuid().as_raw(),
        }
    }
}

/// `true` when `path` exists as any entry, `false` when absent, `None` on another error (the
/// daemon's rule for its flags).
fn entry_exists(path: &Path) -> Option<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Some(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// Probes every feature. Never writes and never runs `pkexec`.
pub fn probe_snapshot(paths: &FeaturePaths, runner: &dyn ProgramRunner) -> FeatureSnapshot {
    let remote_unit = if paths.running_as_root {
        UnitState::Unknown
    } else {
        let (program, args) = unit_show_command();
        match runner.run(
            program,
            &args,
            Some(USER_SYSTEMCTL_TIMEOUT),
            MAX_UNIT_SHOW_BYTES,
        ) {
            Ok(out) if out.success => parse_unit_show(&String::from_utf8_lossy(&out.stdout)),
            _ => UnitState::Unknown,
        }
    };
    let (daemon_presence_enabled, daemon_remote_view) =
        read_daemon_keys(&paths.daemon_config, paths.uid);
    FeatureSnapshot {
        network_lock_rfkill_installed: entry_exists(&paths.network_lock_rfkill),
        daemon_presence_enabled,
        daemon_remote_view,
        global_disabled: entry_exists(&paths.etc_dir.join(GLOBAL_DISABLE_FLAG)),
        presence_disabled: entry_exists(&paths.etc_dir.join(PRESENCE_DISABLE_FLAG)),
        network_lock_installed: entry_exists(&paths.network_lock_rule),
        remote_unit,
        remote_config: paths
            .remote_config
            .as_deref()
            .map_or(RemoteConfigState::Unknown, read_remote_config),
        running_as_root: paths.running_as_root,
    }
}

/// Carries out feature actions. May block for as long as a Polkit dialog is open.
pub trait FeatureExecutor: Send + Sync {
    /// Runs `action` to completion.
    fn execute(&self, action: FeatureAction) -> FeatureOutcome;
}

/// Production executor.
pub struct SystemFeatureExecutor {
    paths: FeaturePaths,
    runner: Arc<dyn ProgramRunner>,
}

impl SystemFeatureExecutor {
    /// An executor over `paths`, running programs through `runner`.
    pub fn new(paths: FeaturePaths, runner: Arc<dyn ProgramRunner>) -> Self {
        Self { paths, runner }
    }

    fn run_checked(
        &self,
        (program, args): (&'static str, Vec<String>),
        deadline: Option<Duration>,
        what: &str,
    ) -> Result<(), String> {
        let out = self.runner.run(program, &args, deadline, 0)?;
        if out.success {
            Ok(())
        } else {
            Err(format!(
                "Failed to {what} (authorization denied or error, exit code {:?})",
                out.code
            ))
        }
    }

    fn disable_remote(&self, switch: RemoteSwitch) -> Result<(), String> {
        if self.paths.running_as_root {
            return Err("soos-gui runs as root: edit remote.toml as your own user".to_string());
        }
        let path = self
            .paths
            .remote_config
            .as_deref()
            .ok_or_else(|| "no configuration directory is known for this user".to_string())?;
        let text = read_user_text(path)?.ok_or_else(|| "remote.toml does not exist".to_string())?;
        let edited =
            disable_remote_keys(&text, switch.keys_to_clear()).map_err(|e| e.to_string())?;
        let Some(edited) = edited else {
            return Ok(());
        };
        write_user_file_atomically(path, &edited)?;
        self.run_checked(
            companion_restart_command(),
            Some(USER_SYSTEMCTL_TIMEOUT),
            "restart soos-remote",
        )
        .map_err(|e| format!("Saved, but soos-remote did not restart: {e}"))
    }

    fn apply(&self, action: FeatureAction) -> Result<(), String> {
        match action {
            FeatureAction::Refresh => Ok(()),
            FeatureAction::SetSystemFlag { flag, present } => self.run_checked(
                system_flag_command(flag, present),
                None,
                if present {
                    "create the soos flag file"
                } else {
                    "remove the soos flag file"
                },
            ),
            FeatureAction::SetNetworkLock { installed } => self.run_checked(
                network_lock_command(installed),
                None,
                if installed {
                    "install the Wi-Fi lock"
                } else {
                    "remove the Wi-Fi lock"
                },
            ),
            FeatureAction::SetCompanion { enabled } => {
                if self.paths.running_as_root {
                    return Err(
                        "soos-gui runs as root: run systemctl --user as your own user".to_string(),
                    );
                }
                self.run_checked(
                    companion_command(enabled),
                    Some(USER_SYSTEMCTL_TIMEOUT),
                    if enabled {
                        "start the phone companion"
                    } else {
                        "stop the phone companion"
                    },
                )
            }
            FeatureAction::DisableRemote(switch) => self.disable_remote(switch),
        }
    }
}

impl FeatureExecutor for SystemFeatureExecutor {
    fn execute(&self, action: FeatureAction) -> FeatureOutcome {
        match action {
            FeatureAction::Refresh => {
                FeatureOutcome::Refreshed(probe_snapshot(&self.paths, self.runner.as_ref()))
            }
            other => FeatureOutcome::Applied {
                action: other,
                result: self.apply(other),
            },
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------------------------

/// Error of [`FeatureRunner::submit`].
#[derive(Debug, thiserror::Error)]
pub enum FeatureRunnerError {
    /// Another feature action is running.
    #[error("another feature change is already in progress")]
    Busy,
    /// The worker thread could not be spawned.
    #[error("failed to start the feature worker: {0}")]
    Spawn(#[from] std::io::Error),
}

/// Runs one [`FeatureAction`] at a time on a background thread.
pub struct FeatureRunner {
    executor: Arc<dyn FeatureExecutor>,
    notify: Notify,
    tx: Sender<FeatureOutcome>,
    rx: Receiver<FeatureOutcome>,
    in_flight: Option<(JoinHandle<()>, FeatureAction)>,
}

impl FeatureRunner {
    /// A runner over `executor`, calling `notify` whenever an outcome is delivered.
    pub fn new(executor: Arc<dyn FeatureExecutor>, notify: Notify) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            executor,
            notify,
            tx,
            rx,
            in_flight: None,
        }
    }

    /// Starts `action` on a worker thread and returns immediately.
    ///
    /// # Errors
    ///
    /// [`FeatureRunnerError::Busy`] while another action runs; [`FeatureRunnerError::Spawn`]
    /// when the thread cannot be created.
    pub fn submit(&mut self, action: FeatureAction) -> Result<(), FeatureRunnerError> {
        if self.in_flight.is_some() {
            return Err(FeatureRunnerError::Busy);
        }
        let executor = Arc::clone(&self.executor);
        let notify = Arc::clone(&self.notify);
        let tx = self.tx.clone();
        let handle = thread::Builder::new()
            .name("soos-gui-features".to_string())
            .spawn(move || {
                let outcome = executor.execute(action);
                let _ = tx.send(outcome);
                notify();
            })?;
        self.in_flight = Some((handle, action));
        Ok(())
    }

    /// Whether an action is running.
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Whether a Polkit-backed action is running.
    pub fn privileged_in_flight(&self) -> bool {
        self.in_flight
            .as_ref()
            .is_some_and(|(_, action)| action.is_privileged())
    }

    /// Drains delivered outcomes without blocking. Call once per frame.
    pub fn poll(&mut self) -> Vec<FeatureOutcome> {
        let finished = self
            .in_flight
            .as_ref()
            .is_some_and(|(handle, _)| handle.is_finished());
        let mut outcomes: Vec<FeatureOutcome> = self.rx.try_iter().collect();
        if !outcomes.is_empty() || finished {
            if finished && outcomes.is_empty() {
                tracing::error!("feature worker terminated without an outcome");
                if let Some((_, action)) = self.in_flight.as_ref() {
                    if *action != FeatureAction::Refresh {
                        outcomes.push(FeatureOutcome::Applied {
                            action: *action,
                            result: Err("The feature change stopped unexpectedly.".to_string()),
                        });
                    }
                }
            }
            self.in_flight = None;
        }
        outcomes
    }
}

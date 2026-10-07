//! Configuration of `soos-remote` (architect spec §2.3).
//!
//! The file is small TOML (`deny_unknown_fields`), read through a bounded, non-blocking
//! descriptor and validated fail-closed: an empty allowlist, an out-of-range polling
//! interval or an unusable socket path refuses to start (same rule as the daemon's
//! fail-closed `daemon.toml` validation), never a clamped or defaulted value.

use std::ffi::OsStr;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use nix::fcntl::OFlag;
use serde::Deserialize;

use crate::identity::is_valid_host_name;
use crate::{
    ALERTS_ACK_FILE_NAME, CREDENTIALS_FILE_NAME, MAX_ALLOWED_HOSTS, MAX_ALLOWED_LOGINS,
    MAX_CONFIG_BYTES, MAX_CREDENTIALS_PATH_LEN, MAX_LOCK_SCREEN_PROGRAMS, MAX_LOGIN_LEN,
    MAX_POLL_INTERVAL_MS, MAX_SOCKET_PATH_LEN, MAX_VAPID_SUBJECT_LEN, MIN_POLL_INTERVAL_MS,
    PUSH_STORE_FILE_NAME, SOCKET_DIR_NAME, SOCKET_FILE_NAME, TS_NET_SUFFIX,
};
use crate::{
    CAMERA_FULL_WIDTH, CAMERA_HALF_WIDTH, DEFAULT_CAMERA_FPS, DEFAULT_CAMERA_MAX_VIEW_S,
    DEFAULT_CAMERA_QUALITY, MAX_CAMERA_FPS, MAX_CAMERA_MAX_VIEW_S, MAX_CAMERA_QUALITY,
    MIN_CAMERA_FPS, MIN_CAMERA_MAX_VIEW_S, MIN_CAMERA_QUALITY,
};
use soos_push_protocol::{PUSH_SOCKET_DIR_NAME, PUSH_SOCKET_FILE_NAME};

/// Validated Tailscale login (`Tailscale-User-Login` value), stored ASCII-lowercased.
#[derive(Clone, PartialEq, Eq)]
pub struct TailscaleLogin(String);

impl TailscaleLogin {
    /// `None` unless 1..=`MAX_LOGIN_LEN` bytes, printable ASCII (0x21..=0x7E), no `,` `;` `"`.
    /// Idempotent on its own output.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.len() > MAX_LOGIN_LEN {
            return None;
        }
        let printable = raw
            .bytes()
            .all(|b| (0x21..=0x7e).contains(&b) && b != b',' && b != b';' && b != b'"');
        printable.then(|| Self(raw.to_ascii_lowercase()))
    }

    /// The lowercased login.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The login is an identity: it never reaches a log, an error context or a test
/// diagnostic through `Debug` (RC-5).
impl fmt::Debug for TailscaleLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TailscaleLogin(<redacted>)")
    }
}

/// Validated service configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteConfig {
    /// 1..=`MAX_ALLOWED_LOGINS` entries, duplicates removed (after lowercasing), order kept.
    pub allowed_logins: Vec<TailscaleLogin>,
    /// Absolute path, at most `MAX_SOCKET_PATH_LEN` bytes, parent directory named by it.
    pub socket_path: PathBuf,
    /// `MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS`.
    pub poll_interval_ms: u64,
    /// 0..=`MAX_ALLOWED_HOSTS` lowercased DNS names; empty = any `*.ts.net` name.
    pub allowed_hosts: Vec<String>,
    /// `POST /api/unlock` enabled (ADR 2026-10-06); `false` unless the file says `true`.
    pub allow_unlock: bool,
    /// Passkey and Funnel settings (ADR 2026-10-06 "Tailscale Funnel Access and In-House
    /// Passkey Authentication for `soos-remote`").
    pub auth: AuthConfig,
    /// Failed-password alert settings (ADR 2026-10-06 "Failed-Password Alerts in
    /// `soos-remote` From the System Journal").
    pub alerts: AlertsConfig,
    /// Web Push settings (ADR 2026-10-06 "Web Push Notifications for Failed-Password Alerts
    /// Through a Separate Sender Unit").
    pub push: PushConfig,
    /// Live camera view settings (ADR 2026-10-07 "Live Camera View in `soos-remote` Through
    /// the Daemon Preview Channel").
    pub camera: CameraConfig,
}

/// Output width of the live view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraWidth {
    /// Source size (at most 640x480).
    #[default]
    Full,
    /// 2x2 box downscale when the source is wider than `CAMERA_HALF_WIDTH`.
    Half,
}

/// Live camera view settings; `Default` = off with the default bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraConfig {
    /// `camera_view`; default `false`.
    pub enabled: bool,
    /// `camera_view_funnel`; default `false` (tailnet only).
    pub funnel: bool,
    /// `camera_max_view_s` in `MIN_CAMERA_MAX_VIEW_S..=MAX_CAMERA_MAX_VIEW_S`, default 120.
    pub max_view_s: u32,
    /// `camera_fps` in `MIN_CAMERA_FPS..=MAX_CAMERA_FPS`, default 5.
    pub fps: u32,
    /// `camera_width` in {320, 640}, default 640.
    pub width: CameraWidth,
    /// `camera_quality` in `MIN_CAMERA_QUALITY..=MAX_CAMERA_QUALITY`, default 70.
    pub quality: u8,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            funnel: false,
            max_view_s: DEFAULT_CAMERA_MAX_VIEW_S,
            fps: DEFAULT_CAMERA_FPS,
            width: CameraWidth::Full,
            quality: DEFAULT_CAMERA_QUALITY,
        }
    }
}

/// What a push notification shows on the phone's lock screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PushPreviews {
    /// Source class, account class, kind and counts.
    #[default]
    Detailed,
    /// A fixed generic text (counts kept in the hidden payload member).
    Generic,
}

/// Web Push settings; `Default` = off.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PushConfig {
    /// `push_notifications`; default `false`.
    pub enabled: bool,
    /// `vapid_subject`; `None` → `https://<rp_id>` at resolution time.
    pub vapid_subject: Option<String>,
    /// `push_socket_path`; resolved to `$XDG_RUNTIME_DIR/soos-push/push.sock` when push is
    /// enabled and the key is absent.
    pub socket_path: Option<PathBuf>,
    /// `push_previews`: `detailed` (default) or `generic`.
    pub previews: PushPreviews,
}

/// Built-in lock-screen programs when `lock_screen_programs` is absent.
pub const DEFAULT_LOCK_SCREEN_PROGRAMS: [&str; 4] = [
    "/usr/bin/swaylock",
    "/usr/bin/hyprlock",
    "/usr/bin/gtklock",
    "/usr/bin/waylock",
];

/// Longest `lock_screen_programs` entry (bytes).
const MAX_LOCK_SCREEN_PROGRAM_LEN: usize = 4096;

/// Password-alert settings; `Default` = off, built-in lock-screen programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertsConfig {
    /// `password_alerts`; default `false`.
    pub enabled: bool,
    /// `lock_screen_programs`: 0..=`MAX_LOCK_SCREEN_PROGRAMS` absolute paths, duplicates
    /// removed, order kept.
    pub lock_screen_programs: Vec<String>,
}

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            lock_screen_programs: DEFAULT_LOCK_SCREEN_PROGRAMS
                .iter()
                .map(|path| (*path).to_string())
                .collect(),
        }
    }
}

/// Passkey and Funnel settings; `Default` = everything off.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuthConfig {
    /// WebAuthn RP ID = the full node host; `None` disables passkeys.
    pub rp_id: Option<String>,
    /// Accept `Tailscale-Funnel-Request: ?1` requests; requires `rp_id`.
    pub allow_funnel: bool,
    /// Explicit credential store path.
    pub credentials_path: Option<PathBuf>,
}

/// Configuration failure; every variant exits with `EXIT_CONFIG`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    /// The configuration file does not exist.
    #[error("configuration file not found")]
    NotFound,
    /// The configuration file exists but cannot be read.
    #[error("configuration file unreadable")]
    Unreadable,
    /// The configuration file exceeds `MAX_CONFIG_BYTES`.
    #[error("configuration file larger than {max} bytes")]
    TooLarge {
        /// `MAX_CONFIG_BYTES`.
        max: usize,
    },
    /// Not valid TOML for this crate (unknown fields included).
    #[error("configuration is not valid TOML for soos-remote")]
    Syntax,
    /// `allowed_logins` missing or empty.
    #[error("allowed_logins is empty")]
    NoAllowedLogins,
    /// More than `MAX_ALLOWED_LOGINS` logins.
    #[error("too many allowed_logins (max {max})")]
    TooManyLogins {
        /// `MAX_ALLOWED_LOGINS`.
        max: usize,
    },
    /// A login failed `TailscaleLogin::parse`.
    #[error("invalid login at index {index}")]
    InvalidLogin {
        /// Index in the file order.
        index: usize,
    },
    /// `poll_interval_ms` outside the accepted range.
    #[error("poll_interval_ms out of range")]
    PollIntervalOutOfRange,
    /// Relative, trailing-slash, parentless or oversize socket path.
    #[error("socket_path must be absolute and at most {max} bytes")]
    InvalidSocketPath {
        /// `MAX_SOCKET_PATH_LEN`.
        max: usize,
    },
    /// `XDG_RUNTIME_DIR` unset, empty or relative while `socket_path` is absent.
    #[error("XDG_RUNTIME_DIR is not set or not absolute")]
    NoRuntimeDir,
    /// No usable default configuration path.
    #[error("no configuration path: neither --config, XDG_CONFIG_HOME nor HOME is usable")]
    NoConfigPath,
    /// More than `MAX_ALLOWED_HOSTS` hosts.
    #[error("too many allowed_hosts (max {max})")]
    TooManyHosts {
        /// `MAX_ALLOWED_HOSTS`.
        max: usize,
    },
    /// A host is not a valid DNS name.
    #[error("invalid host at index {index}")]
    InvalidHost {
        /// Index in the file order.
        index: usize,
    },
    /// The real or effective uid is 0.
    #[error("soos-remote must not run as root")]
    RunningAsRoot,
    /// `rp_id` is not the full node host name.
    #[error("rp_id must be the full node host name")]
    InvalidRpId,
    /// `allow_funnel = true` without `rp_id`.
    #[error("allow_funnel requires rp_id")]
    FunnelNeedsRpId,
    /// More than `MAX_LOCK_SCREEN_PROGRAMS` lock-screen programs.
    #[error("too many lock_screen_programs (max {max})")]
    TooManyLockScreenPrograms {
        /// `MAX_LOCK_SCREEN_PROGRAMS`.
        max: usize,
    },
    /// A lock-screen program path is not acceptable (the path is never echoed).
    #[error("invalid lock_screen_programs entry at index {index}")]
    InvalidLockScreenProgram {
        /// Index in the file order.
        index: usize,
    },
    /// Relative, trailing-slash, parentless or oversize `credentials_path`.
    #[error("credentials_path must be absolute and at most {max} bytes")]
    InvalidCredentialsPath {
        /// `MAX_CREDENTIALS_PATH_LEN`.
        max: usize,
    },
    /// `push_notifications = true` without `password_alerts = true`.
    #[error("push_notifications requires password_alerts")]
    PushRequiresAlerts,
    /// `push_notifications = true` without `rp_id`.
    #[error("push_notifications requires rp_id")]
    PushRequiresRpId,
    /// `vapid_subject` is not an acceptable subject (the value is never echoed).
    #[error("vapid_subject must be a deliverable contact address or site")]
    InvalidVapidSubject,
    /// Relative, trailing-slash, parentless or oversize `push_socket_path`.
    #[error("push_socket_path must be absolute and at most {max} bytes")]
    InvalidPushSocketPath {
        /// `MAX_SOCKET_PATH_LEN`.
        max: usize,
    },
    /// `push_previews` is neither `detailed` nor `generic`.
    #[error("push_previews must be detailed or generic")]
    InvalidPushPreviews,
    /// `camera_view = true` without `rp_id`.
    #[error("camera_view requires rp_id")]
    CameraRequiresRpId,
    /// `camera_view_funnel = true` while `camera_view` is not `true`.
    #[error("camera_view_funnel requires camera_view")]
    CameraFunnelRequiresCameraView,
    /// `camera_view_funnel = true` without `allow_funnel = true`.
    #[error("camera_view_funnel requires allow_funnel")]
    CameraFunnelRequiresFunnel,
    /// `camera_max_view_s` outside the accepted range (the value is never echoed).
    #[error("camera_max_view_s out of range")]
    CameraMaxViewOutOfRange,
    /// `camera_fps` outside the accepted range (the value is never echoed).
    #[error("camera_fps out of range")]
    CameraFpsOutOfRange,
    /// `camera_width` is neither 320 nor 640 (the value is never echoed).
    #[error("camera_width must be 320 or 640")]
    InvalidCameraWidth,
    /// `camera_quality` outside the accepted range (the value is never echoed).
    #[error("camera_quality out of range")]
    CameraQualityOutOfRange,
}

/// Pure. `Err(RunningAsRoot)` when `uid == 0 || euid == 0`.
///
/// # Errors
///
/// [`ConfigError::RunningAsRoot`].
pub fn check_not_root(uid: u32, euid: u32) -> Result<(), ConfigError> {
    if uid == 0 || euid == 0 {
        return Err(ConfigError::RunningAsRoot);
    }
    Ok(())
}

/// On-disk shape of `remote.toml` (every key optional, unknown keys refused).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    allowed_logins: Option<Vec<String>>,
    socket_path: Option<String>,
    poll_interval_ms: Option<u64>,
    allowed_hosts: Option<Vec<String>>,
    allow_unlock: Option<bool>,
    rp_id: Option<String>,
    allow_funnel: Option<bool>,
    credentials_path: Option<String>,
    password_alerts: Option<bool>,
    lock_screen_programs: Option<Vec<String>>,
    push_notifications: Option<bool>,
    vapid_subject: Option<String>,
    push_socket_path: Option<String>,
    push_previews: Option<String>,
    camera_view: Option<bool>,
    camera_view_funnel: Option<bool>,
    camera_max_view_s: Option<u32>,
    camera_fps: Option<u32>,
    camera_width: Option<u32>,
    camera_quality: Option<u8>,
}

/// The six camera keys of the file.
struct CameraKeys {
    camera_view: Option<bool>,
    camera_view_funnel: Option<bool>,
    camera_max_view_s: Option<u32>,
    camera_fps: Option<u32>,
    camera_width: Option<u32>,
    camera_quality: Option<u8>,
}

/// The camera settings of the file (spec §4.1). Absent keys take their default; `0` is out
/// of range (never "unlimited"); range keys are validated even when `camera_view = false`.
/// Check order: rp_id, funnel needs view, funnel needs `allow_funnel`, max view, fps, width,
/// quality.
fn camera_config(
    keys: &CameraKeys,
    rp_id: Option<&str>,
    allow_funnel: bool,
) -> Result<CameraConfig, ConfigError> {
    let enabled = keys.camera_view.unwrap_or(false);
    let funnel = keys.camera_view_funnel.unwrap_or(false);
    if enabled && rp_id.is_none() {
        return Err(ConfigError::CameraRequiresRpId);
    }
    if funnel && !enabled {
        return Err(ConfigError::CameraFunnelRequiresCameraView);
    }
    if funnel && !allow_funnel {
        return Err(ConfigError::CameraFunnelRequiresFunnel);
    }
    let max_view_s = keys.camera_max_view_s.unwrap_or(DEFAULT_CAMERA_MAX_VIEW_S);
    if !(MIN_CAMERA_MAX_VIEW_S..=MAX_CAMERA_MAX_VIEW_S).contains(&max_view_s) {
        return Err(ConfigError::CameraMaxViewOutOfRange);
    }
    let fps = keys.camera_fps.unwrap_or(DEFAULT_CAMERA_FPS);
    if !(MIN_CAMERA_FPS..=MAX_CAMERA_FPS).contains(&fps) {
        return Err(ConfigError::CameraFpsOutOfRange);
    }
    let width = match keys.camera_width {
        None => CameraWidth::Full,
        Some(w) if w == CAMERA_FULL_WIDTH => CameraWidth::Full,
        Some(w) if w == CAMERA_HALF_WIDTH => CameraWidth::Half,
        Some(_) => return Err(ConfigError::InvalidCameraWidth),
    };
    let quality = keys.camera_quality.unwrap_or(DEFAULT_CAMERA_QUALITY);
    if !(MIN_CAMERA_QUALITY..=MAX_CAMERA_QUALITY).contains(&quality) {
        return Err(ConfigError::CameraQualityOutOfRange);
    }
    Ok(CameraConfig {
        enabled,
        funnel,
        max_view_s,
        fps,
        width,
        quality,
    })
}

/// One `lock_screen_programs` entry: absolute, 1..=4096 bytes, no trailing `/`, no `..`
/// component, no control byte, not ending in the kernel's ` (deleted)` suffix.
fn valid_lock_screen_program(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= MAX_LOCK_SCREEN_PROGRAM_LEN
        && raw.starts_with('/')
        && !raw.ends_with('/')
        && !raw.ends_with(crate::journal::EXE_DELETED_SUFFIX)
        && !raw.split('/').any(|component| component == "..")
        && !raw.bytes().any(|b| b < 0x20 || b == 0x7f)
}

/// The alert settings of the file (absent keys: off, built-in lock-screen programs).
fn alerts_config(
    password_alerts: Option<bool>,
    lock_screen_programs: Option<Vec<String>>,
) -> Result<AlertsConfig, ConfigError> {
    let mut alerts = AlertsConfig {
        enabled: password_alerts.unwrap_or(false),
        ..AlertsConfig::default()
    };
    if let Some(list) = lock_screen_programs {
        if list.len() > MAX_LOCK_SCREEN_PROGRAMS {
            return Err(ConfigError::TooManyLockScreenPrograms {
                max: MAX_LOCK_SCREEN_PROGRAMS,
            });
        }
        let mut programs: Vec<String> = Vec::with_capacity(list.len());
        for (index, path) in list.into_iter().enumerate() {
            if !valid_lock_screen_program(&path) {
                return Err(ConfigError::InvalidLockScreenProgram { index });
            }
            if !programs.contains(&path) {
                programs.push(path);
            }
        }
        alerts.lock_screen_programs = programs;
    }
    Ok(alerts)
}

/// Host names a push service refuses as a VAPID subject (research §3.2: Apple answers
/// `403 BadJwtToken`).
const RESERVED_SUBJECT_SUFFIXES: [&str; 7] = [
    ".localhost",
    ".local",
    ".invalid",
    ".test",
    ".example",
    ".internal",
    ".home.arpa",
];

/// A subject host: a valid DNS name with at least one dot, not a reserved name.
fn valid_subject_host(host: &str) -> bool {
    is_valid_host_name(host)
        && host.contains('.')
        && host != "localhost"
        && !RESERVED_SUBJECT_SUFFIXES
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

/// `vapid_subject` (spec §3.2): `mailto:<local>@<domain>` or `https://<host>[/<path>]`,
/// 1..=`MAX_VAPID_SUBJECT_LEN` printable ASCII bytes without space.
fn validate_vapid_subject(raw: &str) -> Result<String, ConfigError> {
    let invalid = ConfigError::InvalidVapidSubject;
    if raw.is_empty()
        || raw.len() > MAX_VAPID_SUBJECT_LEN
        || !raw.bytes().all(|b| (0x21..=0x7e).contains(&b))
    {
        return Err(invalid);
    }
    if let Some(address) = raw.strip_prefix("mailto:") {
        let mut parts = address.split('@');
        let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(invalid);
        };
        let local_ok = !local.is_empty()
            && local.len() <= 64
            && !local.bytes().any(|b| b"<>()[],;:\\\"".contains(&b));
        if local_ok && valid_subject_host(domain) {
            return Ok(raw.to_owned());
        }
        return Err(invalid);
    }
    if let Some(rest) = raw.strip_prefix("https://") {
        let (host, path) = match rest.find('/') {
            Some(slash) => rest.split_at(slash),
            None => (rest, ""),
        };
        if valid_subject_host(host) && !path.contains('?') && !path.contains('#') {
            return Ok(raw.to_owned());
        }
        return Err(invalid);
    }
    Err(invalid)
}

/// `push_previews`: exactly `detailed` or `generic`.
fn parse_push_previews(raw: Option<&str>) -> Result<PushPreviews, ConfigError> {
    match raw {
        None | Some("detailed") => Ok(PushPreviews::Detailed),
        Some("generic") => Ok(PushPreviews::Generic),
        Some(_) => Err(ConfigError::InvalidPushPreviews),
    }
}

/// The four push keys of the file.
struct PushKeys {
    push_notifications: Option<bool>,
    vapid_subject: Option<String>,
    push_socket_path: Option<String>,
    push_previews: Option<String>,
}

/// The push settings of the file; the default sender socket is resolved only when push is
/// enabled.
fn push_config(
    file: &PushKeys,
    alerts: &AlertsConfig,
    rp_id: Option<&str>,
    runtime_dir: Option<&Path>,
) -> Result<PushConfig, ConfigError> {
    let enabled = file.push_notifications.unwrap_or(false);
    if enabled && !alerts.enabled {
        return Err(ConfigError::PushRequiresAlerts);
    }
    if enabled && rp_id.is_none() {
        return Err(ConfigError::PushRequiresRpId);
    }
    let vapid_subject = match &file.vapid_subject {
        Some(raw) => Some(validate_vapid_subject(raw)?),
        None => None,
    };
    let invalid_socket = ConfigError::InvalidPushSocketPath {
        max: MAX_SOCKET_PATH_LEN,
    };
    let socket_path = match &file.push_socket_path {
        Some(raw) => Some(validate_socket_path(raw).map_err(|_| invalid_socket)?),
        None if enabled => {
            let dir = runtime_dir
                .filter(|dir| !dir.as_os_str().is_empty() && dir.is_absolute())
                .ok_or(ConfigError::NoRuntimeDir)?;
            let path = dir.join(PUSH_SOCKET_DIR_NAME).join(PUSH_SOCKET_FILE_NAME);
            if path.as_os_str().len() > MAX_SOCKET_PATH_LEN {
                return Err(invalid_socket);
            }
            Some(path)
        }
        None => None,
    };
    Ok(PushConfig {
        enabled,
        vapid_subject,
        socket_path,
        previews: parse_push_previews(file.push_previews.as_deref())?,
    })
}

/// An explicit `socket_path`: absolute, with a parent and a file name, no trailing `/`,
/// at most `MAX_SOCKET_PATH_LEN` bytes.
fn validate_socket_path(raw: &str) -> Result<PathBuf, ConfigError> {
    let invalid = ConfigError::InvalidSocketPath {
        max: MAX_SOCKET_PATH_LEN,
    };
    if raw.is_empty() || raw.len() > MAX_SOCKET_PATH_LEN || raw.ends_with('/') {
        return Err(invalid);
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute() || path.parent().is_none() || path.file_name().is_none() {
        return Err(invalid);
    }
    Ok(path)
}

/// `rp_id`: lowercased, a valid DNS name ending with `.ts.net` with at least two labels
/// before the suffix (`<node>.<tailnet>.ts.net`), and a member of `allowed_hosts` when that
/// list is non-empty. Never echoed in the error.
fn validate_rp_id(raw: &str, allowed_hosts: &[String]) -> Result<String, ConfigError> {
    let lowered = raw.to_ascii_lowercase();
    if !is_valid_host_name(&lowered) {
        return Err(ConfigError::InvalidRpId);
    }
    let prefix = lowered
        .strip_suffix(TS_NET_SUFFIX)
        .ok_or(ConfigError::InvalidRpId)?;
    let labels = prefix.split('.').filter(|label| !label.is_empty()).count();
    if prefix.is_empty() || labels < 2 {
        return Err(ConfigError::InvalidRpId);
    }
    if !allowed_hosts.is_empty() && !allowed_hosts.contains(&lowered) {
        return Err(ConfigError::InvalidRpId);
    }
    Ok(lowered)
}

/// An explicit `credentials_path`: absolute, with a parent and a file name, no trailing `/`,
/// at most `MAX_CREDENTIALS_PATH_LEN` bytes.
fn validate_credentials_path(raw: &str) -> Result<PathBuf, ConfigError> {
    let invalid = ConfigError::InvalidCredentialsPath {
        max: MAX_CREDENTIALS_PATH_LEN,
    };
    if raw.is_empty() || raw.len() > MAX_CREDENTIALS_PATH_LEN || raw.ends_with('/') {
        return Err(invalid);
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute() || path.parent().is_none() || path.file_name().is_none() {
        return Err(invalid);
    }
    Ok(path)
}

/// Default socket path `<XDG_RUNTIME_DIR>/soos-remote/remote.sock`; the runtime dir must be
/// set and absolute (never a `/tmp` fallback).
fn default_socket_path(runtime_dir: Option<&Path>) -> Result<PathBuf, ConfigError> {
    let dir = runtime_dir
        .filter(|dir| !dir.as_os_str().is_empty() && dir.is_absolute())
        .ok_or(ConfigError::NoRuntimeDir)?;
    let path = dir.join(SOCKET_DIR_NAME).join(SOCKET_FILE_NAME);
    if path.as_os_str().len() > MAX_SOCKET_PATH_LEN {
        return Err(ConfigError::InvalidSocketPath {
            max: MAX_SOCKET_PATH_LEN,
        });
    }
    Ok(path)
}

/// Pure: parses and validates TOML text. `runtime_dir` is the value of `XDG_RUNTIME_DIR`.
///
/// # Errors
///
/// Every [`ConfigError`] variant except the file-level ones.
pub fn parse_config(text: &str, runtime_dir: Option<&Path>) -> Result<RemoteConfig, ConfigError> {
    let file: FileConfig = toml::from_str(text).map_err(|_| ConfigError::Syntax)?;

    let raw_logins = file.allowed_logins.unwrap_or_default();
    if raw_logins.is_empty() {
        return Err(ConfigError::NoAllowedLogins);
    }
    if raw_logins.len() > MAX_ALLOWED_LOGINS {
        return Err(ConfigError::TooManyLogins {
            max: MAX_ALLOWED_LOGINS,
        });
    }
    let mut allowed_logins: Vec<TailscaleLogin> = Vec::with_capacity(raw_logins.len());
    for (index, raw) in raw_logins.iter().enumerate() {
        let login = TailscaleLogin::parse(raw).ok_or(ConfigError::InvalidLogin { index })?;
        if !allowed_logins.contains(&login) {
            allowed_logins.push(login);
        }
    }

    let poll_interval_ms = file
        .poll_interval_ms
        .unwrap_or(crate::DEFAULT_POLL_INTERVAL_MS);
    if !(MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS).contains(&poll_interval_ms) {
        return Err(ConfigError::PollIntervalOutOfRange);
    }

    let raw_hosts = file.allowed_hosts.unwrap_or_default();
    if raw_hosts.len() > MAX_ALLOWED_HOSTS {
        return Err(ConfigError::TooManyHosts {
            max: MAX_ALLOWED_HOSTS,
        });
    }
    let mut allowed_hosts = Vec::with_capacity(raw_hosts.len());
    for (index, raw) in raw_hosts.iter().enumerate() {
        let lowered = raw.to_ascii_lowercase();
        if !is_valid_host_name(&lowered) {
            return Err(ConfigError::InvalidHost { index });
        }
        allowed_hosts.push(lowered);
    }

    let socket_path = match file.socket_path {
        Some(raw) => validate_socket_path(&raw)?,
        None => default_socket_path(runtime_dir)?,
    };

    let rp_id = match file.rp_id {
        Some(raw) => Some(validate_rp_id(&raw, &allowed_hosts)?),
        None => None,
    };
    let allow_funnel = file.allow_funnel.unwrap_or(false);
    if allow_funnel && rp_id.is_none() {
        return Err(ConfigError::FunnelNeedsRpId);
    }
    let credentials_path = match file.credentials_path {
        Some(raw) => Some(validate_credentials_path(&raw)?),
        None => None,
    };
    let alerts = alerts_config(file.password_alerts, file.lock_screen_programs)?;
    let push_keys = PushKeys {
        push_notifications: file.push_notifications,
        vapid_subject: file.vapid_subject,
        push_socket_path: file.push_socket_path,
        push_previews: file.push_previews,
    };
    let push = push_config(&push_keys, &alerts, rp_id.as_deref(), runtime_dir)?;
    let camera_keys = CameraKeys {
        camera_view: file.camera_view,
        camera_view_funnel: file.camera_view_funnel,
        camera_max_view_s: file.camera_max_view_s,
        camera_fps: file.camera_fps,
        camera_width: file.camera_width,
        camera_quality: file.camera_quality,
    };
    let camera = camera_config(&camera_keys, rp_id.as_deref(), allow_funnel)?;
    Ok(RemoteConfig {
        allowed_logins,
        socket_path,
        poll_interval_ms,
        allowed_hosts,
        allow_unlock: file.allow_unlock.unwrap_or(false),
        auth: AuthConfig {
            rp_id,
            allow_funnel,
            credentials_path,
        },
        alerts,
        push,
        camera,
    })
}

/// Bounded read (≤ `MAX_CONFIG_BYTES`) then [`parse_config`].
///
/// The file is opened non-blocking (a FIFO never stalls the start) and must be a regular
/// file; it is read through `take(MAX_CONFIG_BYTES + 1)`, so an oversize file is refused
/// without being read in full.
///
/// # Errors
///
/// [`ConfigError::NotFound`], [`ConfigError::Unreadable`], [`ConfigError::TooLarge`] and
/// every [`parse_config`] error.
pub fn load_config(path: &Path, runtime_dir: Option<&Path>) -> Result<RemoteConfig, ConfigError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NONBLOCK | OFlag::O_CLOEXEC).bits())
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ConfigError::NotFound
            } else {
                ConfigError::Unreadable
            }
        })?;
    let metadata = file.metadata().map_err(|_| ConfigError::Unreadable)?;
    if !metadata.is_file() {
        return Err(ConfigError::Unreadable);
    }
    let limit = u64::try_from(MAX_CONFIG_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    let mut buf = Vec::with_capacity(MAX_CONFIG_BYTES.saturating_add(1).min(4096));
    file.take(limit)
        .read_to_end(&mut buf)
        .map_err(|_| ConfigError::Unreadable)?;
    if buf.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge {
            max: MAX_CONFIG_BYTES,
        });
    }
    let text = String::from_utf8(buf).map_err(|_| ConfigError::Syntax)?;
    parse_config(&text, runtime_dir)
}

/// An environment value usable as a base directory: non-empty and absolute.
fn absolute_dir(value: Option<&OsStr>) -> Option<&Path> {
    value
        .map(Path::new)
        .filter(|path| !path.as_os_str().is_empty() && path.is_absolute())
}

/// Default config path: `$XDG_CONFIG_HOME/soos/remote.toml`, else
/// `$HOME/.config/soos/remote.toml` (each must be absolute); pure over the two values. An
/// empty or relative `XDG_CONFIG_HOME` counts as unset (XDG rule).
///
/// # Errors
///
/// [`ConfigError::NoConfigPath`].
pub fn default_config_path(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf, ConfigError> {
    if let Some(xdg) = absolute_dir(xdg_config_home) {
        return Ok(xdg.join("soos").join("remote.toml"));
    }
    if let Some(home) = absolute_dir(home) {
        return Ok(home.join(".config").join("soos").join("remote.toml"));
    }
    Err(ConfigError::NoConfigPath)
}

/// Pure. `explicit` wins; otherwise `<config_path parent>/CREDENTIALS_FILE_NAME` (spec S-5).
///
/// # Errors
///
/// [`ConfigError::InvalidCredentialsPath`] when `config_path` has no parent or the result
/// exceeds `MAX_CREDENTIALS_PATH_LEN`.
pub fn resolve_credentials_path(
    auth: &AuthConfig,
    config_path: &Path,
) -> Result<PathBuf, ConfigError> {
    let invalid = || ConfigError::InvalidCredentialsPath {
        max: MAX_CREDENTIALS_PATH_LEN,
    };
    if let Some(explicit) = &auth.credentials_path {
        return Ok(explicit.clone());
    }
    let parent = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(invalid)?;
    let path = parent.join(CREDENTIALS_FILE_NAME);
    if path.as_os_str().len() > MAX_CREDENTIALS_PATH_LEN {
        return Err(invalid());
    }
    Ok(path)
}

/// Pure. `<parent of credentials_path>/ALERTS_ACK_FILE_NAME` (the acknowledgement file of
/// the failed-password alerts).
///
/// # Errors
///
/// [`ConfigError::InvalidCredentialsPath`] when `credentials_path` has no parent or the
/// result exceeds `MAX_CREDENTIALS_PATH_LEN`.
pub fn resolve_alerts_ack_path(credentials_path: &Path) -> Result<PathBuf, ConfigError> {
    let invalid = || ConfigError::InvalidCredentialsPath {
        max: MAX_CREDENTIALS_PATH_LEN,
    };
    let parent = credentials_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(invalid)?;
    let path = parent.join(ALERTS_ACK_FILE_NAME);
    if path.as_os_str().len() > MAX_CREDENTIALS_PATH_LEN {
        return Err(invalid());
    }
    Ok(path)
}

/// Pure. `<parent of credentials_path>/PUSH_STORE_FILE_NAME` (the Web Push store).
///
/// # Errors
///
/// [`ConfigError::InvalidCredentialsPath`] when `credentials_path` has no parent or the
/// result exceeds `MAX_CREDENTIALS_PATH_LEN`.
pub fn resolve_push_store_path(credentials_path: &Path) -> Result<PathBuf, ConfigError> {
    let invalid = || ConfigError::InvalidCredentialsPath {
        max: MAX_CREDENTIALS_PATH_LEN,
    };
    let parent = credentials_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(invalid)?;
    let path = parent.join(PUSH_STORE_FILE_NAME);
    if path.as_os_str().len() > MAX_CREDENTIALS_PATH_LEN {
        return Err(invalid());
    }
    Ok(path)
}

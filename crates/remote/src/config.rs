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
    MAX_ALLOWED_HOSTS, MAX_ALLOWED_LOGINS, MAX_CONFIG_BYTES, MAX_LOGIN_LEN, MAX_POLL_INTERVAL_MS,
    MAX_SOCKET_PATH_LEN, MIN_POLL_INTERVAL_MS, SOCKET_DIR_NAME, SOCKET_FILE_NAME,
};

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

    Ok(RemoteConfig {
        allowed_logins,
        socket_path,
        poll_interval_ms,
        allowed_hosts,
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

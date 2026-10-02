//! Shared reader of the camera settings of `/etc/soos/daemon.toml` (GitHub #287, #289).
//!
//! `soos-admin camera list`, `soos-enroll` and `soos-gui` (direct mode) resolve the camera with
//! the same `[pipeline] camera_device` and `[pipeline] sensor_preference` as `soos-daemon`. The
//! daemon's own loader lives in the `soos-daemon` binary crate (Tokio, ONNX Runtime), which
//! these clients must not link, so only these two keys are read here, with the daemon's field
//! types and the vocabulary of [`crate::resolver`] (`parse_sensor_preference`).
//!
//! Rules (ADR 2026-10-01 "One Shared `daemon.toml` Camera Reader"):
//! - the path is first opened with `O_PATH | O_CLOEXEC` (no driver `open`, nothing read), that
//!   handle is checked (`fstat`, regular file of at most [`MAX_DAEMON_CONFIG_BYTES`]), and only
//!   then is the pinned inode reopened for reading through `/proc/self/fd/<n>` with
//!   `O_RDONLY | O_NONBLOCK | O_CLOEXEC`, re-checked (same device and inode, still regular) and
//!   read with the bound (GitHub #291). A FIFO or a device node swapped in place of the file
//!   (directly or behind a symbolic link) can neither block the read nor run its driver's
//!   `open`; without `/proc` the file is reported unreadable (fail closed). Symbolic links are followed exactly like `soos-daemon` follows them, so the
//!   clients resolve the camera from the very file the daemon reads (symlink-managed `/etc`:
//!   stow, NixOS, ostree);
//! - a missing, unreadable, oversized, malformed or non-regular file is a
//!   [`DaemonConfigError`]; callers then use the soos-daemon defaults and say so;
//! - a single key of the wrong type falls back to its own default and is reported by name in
//!   [`DaemonCameraConfig::mistyped_keys`] (never by value); the other key still applies.

use std::fs::OpenOptions;
use std::io::Read as _;
use std::os::fd::{AsRawFd as _, RawFd};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use crate::resolver::parse_sensor_preference;
use crate::sensor::SensorPreference;

/// Path of the daemon configuration read by default (`DEFAULT_CONFIG_PATH` of `soos-daemon`).
pub const DEFAULT_DAEMON_CONFIG_PATH: &str = "/etc/soos/daemon.toml";

/// Largest configuration file read (1 MiB); a larger file is refused before parsing.
pub const MAX_DAEMON_CONFIG_BYTES: u64 = 1024 * 1024;

/// The camera keys of `[pipeline]`, as written in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DaemonCameraSettings {
    /// `camera_device`, verbatim (sentinels such as `auto` included).
    pub camera_device: Option<PathBuf>,
    /// `sensor_preference`, when it is part of the shared vocabulary.
    pub sensor_preference: Option<SensorPreference>,
    /// `true` when `sensor_preference` is a string outside the shared vocabulary (soos-daemon
    /// ignores it and keeps its default).
    pub sensor_preference_unrecognized: bool,
}

/// A camera key of `[pipeline]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonConfigKey {
    /// `[pipeline] camera_device` (a string path).
    CameraDevice,
    /// `[pipeline] sensor_preference` (a string of the shared vocabulary).
    SensorPreference,
    /// `[pipeline] allow_virtual_camera` (a boolean, `false` by default; GitHub #307).
    AllowVirtualCamera,
}

impl DaemonConfigKey {
    /// The key name as written in `daemon.toml`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CameraDevice => "camera_device",
            Self::SensorPreference => "sensor_preference",
            Self::AllowVirtualCamera => "allow_virtual_camera",
        }
    }

    /// What the key falls back to when it is ignored.
    const fn default_label(self) -> &'static str {
        match self {
            Self::CameraDevice => "auto-detection",
            Self::SensorPreference => "prefer_ir",
            Self::AllowVirtualCamera => "false",
        }
    }
}

/// Why the configuration could not be used (the caller falls back to the daemon defaults).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DaemonConfigError {
    /// The file does not exist (soos-daemon then runs with its defaults too).
    #[error("not found")]
    NotFound,
    /// The path (after following symbolic links) is not a regular file (directory, FIFO,
    /// socket or device node).
    #[error("is not a regular file")]
    NotARegularFile,
    /// The file exists but cannot be read (for example permission denied).
    #[error("cannot be read ({0})")]
    Unreadable(std::io::ErrorKind),
    /// The file exceeds [`MAX_DAEMON_CONFIG_BYTES`].
    #[error("is larger than {limit} bytes")]
    TooLarge {
        /// The size limit in bytes.
        limit: u64,
    },
    /// Not valid UTF-8 TOML, or `[pipeline]` is not a table.
    #[error(
        "cannot be parsed as the soos-daemon configuration (soos-daemon refuses to start with it)"
    )]
    Malformed,
}

/// What [`read_daemon_camera_config`] found in a usable configuration file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DaemonCameraConfig {
    /// The keys that apply (a mistyped key is `None` here).
    pub settings: DaemonCameraSettings,
    /// Keys present with the wrong TOML type, in file-independent order (`camera_device`
    /// first); each one fell back to its default. A `camera_device` string holding a NUL byte
    /// is listed here too: no path can contain one (GitHub #314, CAM-NEW-4).
    pub mistyped_keys: Vec<DaemonConfigKey>,
    /// `[pipeline] allow_virtual_camera`: opt-in to open a node that is not a physical camera
    /// (v4l2loopback, vivid, output or memory-to-memory capability). `false` when absent or
    /// mistyped (fail closed; GitHub #307, ADR 2026-10-02 "Virtual V4L2 Nodes Are Never
    /// Biometric Cameras"). Feeds `CameraConfig::allow_virtual_device`.
    pub allow_virtual_camera: bool,
}

impl DaemonCameraConfig {
    /// One warning per ignored key, naming the key and never its value.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .mistyped_keys
            .iter()
            .map(|key| {
                format!(
                    "[pipeline] {} has the wrong type and is ignored; its default ({}) applies",
                    key.name(),
                    key.default_label()
                )
            })
            .collect();
        if self.settings.sensor_preference_unrecognized {
            let key = DaemonConfigKey::SensorPreference;
            out.push(format!(
                "[pipeline] {} is not recognized and is ignored; its default ({}) applies",
                key.name(),
                key.default_label()
            ));
        }
        out
    }
}

/// Reads `[pipeline] camera_device` and `[pipeline] sensor_preference` from `path`.
///
/// # Errors
///
/// Returns a [`DaemonConfigError`] when the file is missing (a dangling symbolic link
/// included), not a regular file, unreadable, larger than [`MAX_DAEMON_CONFIG_BYTES`] or malformed. A key of the wrong
/// type is not an error: it is listed in [`DaemonCameraConfig::mistyped_keys`].
pub fn read_daemon_camera_config(path: &Path) -> Result<DaemonCameraConfig, DaemonConfigError> {
    let text = read_bounded_regular_file(path)?;
    let table: toml::Table = text.parse().map_err(|_| DaemonConfigError::Malformed)?;
    let pipeline = match table.get("pipeline") {
        None => return Ok(DaemonCameraConfig::default()),
        Some(toml::Value::Table(pipeline)) => pipeline,
        Some(_) => return Err(DaemonConfigError::Malformed),
    };

    let mut config = DaemonCameraConfig::default();
    match pipeline.get(DaemonConfigKey::CameraDevice.name()) {
        None => {}
        // A NUL byte would reach `open(2)` through `v4l`, which panics on it (GitHub #314):
        // the key is ignored like a mistyped one.
        Some(toml::Value::String(device)) if device.contains('\0') => {
            config.mistyped_keys.push(DaemonConfigKey::CameraDevice);
        }
        Some(toml::Value::String(device)) => {
            config.settings.camera_device = Some(PathBuf::from(device));
        }
        Some(_) => config.mistyped_keys.push(DaemonConfigKey::CameraDevice),
    }
    match pipeline.get(DaemonConfigKey::SensorPreference.name()) {
        None => {}
        Some(toml::Value::String(raw)) => {
            config.settings.sensor_preference = parse_sensor_preference(raw);
            config.settings.sensor_preference_unrecognized =
                config.settings.sensor_preference.is_none();
        }
        Some(_) => config.mistyped_keys.push(DaemonConfigKey::SensorPreference),
    }
    match pipeline.get(DaemonConfigKey::AllowVirtualCamera.name()) {
        None => {}
        Some(toml::Value::Boolean(allow)) => config.allow_virtual_camera = *allow,
        Some(_) => config
            .mistyped_keys
            .push(DaemonConfigKey::AllowVirtualCamera),
    }
    Ok(config)
}

/// Opens `path` without side effects (following symbolic links like `soos-daemon`), checks the
/// handle is a regular file, then reopens it for reading and reads at most
/// [`MAX_DAEMON_CONFIG_BYTES`] of UTF-8 text.
fn read_bounded_regular_file(path: &Path) -> Result<String, DaemonConfigError> {
    read_bounded_regular_file_via(path, proc_self_fd_path)
}

/// The `/proc` magic link that reopens descriptor `fd` of this process.
fn proc_self_fd_path(fd: RawFd) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{fd}"))
}

/// [`read_bounded_regular_file`] with the reopen path supplied by `reopen_path` (a test seam:
/// production always passes [`proc_self_fd_path`]).
fn read_bounded_regular_file_via(
    path: &Path,
    reopen_path: impl Fn(RawFd) -> PathBuf,
) -> Result<String, DaemonConfigError> {
    // O_PATH: the path is resolved (symbolic links followed) and pinned without opening the
    // file itself, so a device node behind the link never runs its driver's `open` (GitHub
    // #291), and a FIFO without a writer cannot block. The handle, not the path, is checked.
    let pinned = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DaemonConfigError::NotFound,
            kind => DaemonConfigError::Unreadable(kind),
        })?;
    let metadata = pinned
        .metadata()
        .map_err(|e| DaemonConfigError::Unreadable(e.kind()))?;
    if !metadata.is_file() {
        return Err(DaemonConfigError::NotARegularFile);
    }
    let too_large = DaemonConfigError::TooLarge {
        limit: MAX_DAEMON_CONFIG_BYTES,
    };
    if metadata.len() > MAX_DAEMON_CONFIG_BYTES {
        return Err(too_large);
    }
    // Reopen the pinned inode (never the path again) for reading; O_NONBLOCK stays as a second
    // line of defence. Any failure, `/proc` not being mounted included, is reported as
    // unreadable (the callers then apply the defaults with a note), never as "not found".
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(reopen_path(pinned.as_raw_fd()))
        .map_err(|e| DaemonConfigError::Unreadable(e.kind()))?;
    let reopened = file
        .metadata()
        .map_err(|e| DaemonConfigError::Unreadable(e.kind()))?;
    if !reopened.is_file() {
        return Err(DaemonConfigError::NotARegularFile);
    }
    if reopened.dev() != metadata.dev() || reopened.ino() != metadata.ino() {
        return Err(DaemonConfigError::Unreadable(std::io::ErrorKind::Other));
    }
    drop(pinned);
    if reopened.len() > MAX_DAEMON_CONFIG_BYTES {
        return Err(too_large);
    }
    let mut bytes = Vec::new();
    file.take(MAX_DAEMON_CONFIG_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| DaemonConfigError::Unreadable(e.kind()))?;
    // The file may have grown after fstat: the bounded read is the final word.
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_DAEMON_CONFIG_BYTES {
        return Err(too_large);
    }
    String::from_utf8(bytes).map_err(|_| DaemonConfigError::Malformed)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "Unit tests of the reopen seam use direct assertions"
)]
mod tests {
    use super::*;

    fn config_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "[pipeline]\ncamera_device = \"/dev/video8\"\n").unwrap();
        path
    }

    /// CDF1: the production seam reads through `/proc/self/fd`.
    #[test]
    fn test_cdf_reopen_through_proc_reads_the_pinned_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path(), "daemon.toml");
        let text = read_bounded_regular_file_via(&path, proc_self_fd_path).unwrap();
        assert!(text.contains("/dev/video8"));
        assert_eq!(proc_self_fd_path(7), PathBuf::from("/proc/self/fd/7"));
    }

    /// CDF1: without `/proc`, the reader fails closed as unreadable (never "not found").
    #[test]
    fn test_cdf_missing_proc_is_unreadable_not_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path(), "daemon.toml");
        let absent = dir.path().join("no-proc");
        let err =
            read_bounded_regular_file_via(&path, |fd| absent.join(fd.to_string())).unwrap_err();
        assert_eq!(
            err,
            DaemonConfigError::Unreadable(std::io::ErrorKind::NotFound)
        );
    }

    /// CDF1: a reopened handle naming another inode is refused.
    #[test]
    fn test_cdf_reopened_handle_must_be_the_pinned_inode() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path(), "daemon.toml");
        let other = config_file(dir.path(), "other.toml");
        let err = read_bounded_regular_file_via(&path, |_| other.clone()).unwrap_err();
        assert_eq!(
            err,
            DaemonConfigError::Unreadable(std::io::ErrorKind::Other)
        );
    }

    /// CDF1: a reopened handle that is not a regular file is refused.
    #[test]
    fn test_cdf_reopened_handle_must_stay_regular() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path(), "daemon.toml");
        let err = read_bounded_regular_file_via(&path, |_| PathBuf::from("/dev/null")).unwrap_err();
        assert_eq!(err, DaemonConfigError::NotARegularFile);
    }
}

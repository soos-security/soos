//! Contract for the shared `daemon.toml` camera-settings reader (GitHub #289, matrix rows
//! DGP4-DGP7).
//!
//! `soos-admin camera list`, `soos-enroll` and `soos-gui` read `[pipeline] camera_device` and
//! `[pipeline] sensor_preference` through `soos_camera_v4l::daemon_config`. Rules:
//! - a missing, unreadable, oversized, malformed or non-regular file is an error the callers
//!   turn into the soos-daemon defaults with a note;
//! - a single wrongly typed key falls back to its own default and is reported by name (never
//!   by value) while the other key still applies;
//! - the file is opened first (`O_NONBLOCK | O_CLOEXEC`) and the handle is then checked, so a
//!   FIFO or a device swapped in place of the file (directly or behind a symbolic link) never
//!   blocks the read; symbolic links are followed like `soos-daemon` follows them.
//!
//! Every test uses a temporary directory, never `/etc`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use soos_camera_v4l::daemon_config::{
    read_daemon_camera_config, DaemonCameraConfig, DaemonCameraSettings, DaemonConfigError,
    DaemonConfigKey, DEFAULT_DAEMON_CONFIG_PATH, MAX_DAEMON_CONFIG_BYTES,
};
use soos_camera_v4l::SensorPreference;

fn write_config(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path
}

/// Runs the reader on another thread and fails the test if it does not return within 2 s
/// (a blocking `open()` on a FIFO would otherwise hang the whole suite).
fn read_with_deadline(path: &Path) -> Result<DaemonCameraConfig, DaemonConfigError> {
    let (tx, rx) = mpsc::channel();
    let owned = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(read_daemon_camera_config(&owned));
    });
    rx.recv_timeout(Duration::from_secs(2))
        .expect("the daemon.toml reader must return promptly, never block")
}

fn make_fifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo failed");
}

// ---------------------------------------------------------------------------
// DGP4: the shared reader keeps the documented constants and vocabulary
// ---------------------------------------------------------------------------

#[test]
fn test_dgp_shared_reader_reads_both_camera_keys() {
    assert_eq!(DEFAULT_DAEMON_CONFIG_PATH, "/etc/soos/daemon.toml");
    assert_eq!(MAX_DAEMON_CONFIG_BYTES, 1024 * 1024);

    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "daemon.toml",
        "log_level = \"info\"\n[pipeline]\ncamera_device = \"/dev/video4\"\n\
         sensor_preference = \"rgb\"\nwarmup_frames = 0\n",
    );
    let config = read_with_deadline(&path).unwrap();
    assert_eq!(
        config.settings,
        DaemonCameraSettings {
            camera_device: Some(PathBuf::from("/dev/video4")),
            sensor_preference: Some(SensorPreference::PreferRgb),
            sensor_preference_unrecognized: false,
        }
    );
    assert!(config.mistyped_keys.is_empty());
    assert!(config.warnings().is_empty(), "{:?}", config.warnings());

    // No [pipeline] section: nothing configured, nothing to warn about.
    let empty = write_config(dir.path(), "empty.toml", "log_level = \"debug\"\n");
    let config = read_with_deadline(&empty).unwrap();
    assert_eq!(config.settings, DaemonCameraSettings::default());
    assert!(config.warnings().is_empty());
}

// ---------------------------------------------------------------------------
// DGP5: a wrongly typed key falls back alone, named but never quoted
// ---------------------------------------------------------------------------

#[test]
fn test_dgp_wrongly_typed_camera_device_falls_back_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "daemon.toml",
        "[pipeline]\ncamera_device = 424242\nsensor_preference = \"prefer_rgb\"\n",
    );
    let config = read_with_deadline(&path).unwrap();
    assert_eq!(config.settings.camera_device, None, "the key falls back");
    assert_eq!(
        config.settings.sensor_preference,
        Some(SensorPreference::PreferRgb),
        "the other key still applies"
    );
    assert_eq!(config.mistyped_keys, vec![DaemonConfigKey::CameraDevice]);
    let warnings = config.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("camera_device"), "{warnings:?}");
    assert!(warnings[0].contains("wrong type"), "{warnings:?}");
    assert!(
        !warnings[0].contains("424242"),
        "the warning names the key, never its value: {warnings:?}"
    );
}

#[test]
fn test_dgp_wrongly_typed_sensor_preference_falls_back_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "daemon.toml",
        "[pipeline]\ncamera_device = \"/dev/video7\"\nsensor_preference = [\"rgb\", 31337]\n",
    );
    let config = read_with_deadline(&path).unwrap();
    assert_eq!(
        config.settings.camera_device,
        Some(PathBuf::from("/dev/video7"))
    );
    assert_eq!(config.settings.sensor_preference, None);
    assert!(
        !config.settings.sensor_preference_unrecognized,
        "a wrong type is reported as mistyped, not as an unknown value"
    );
    assert_eq!(
        config.mistyped_keys,
        vec![DaemonConfigKey::SensorPreference]
    );
    assert_eq!(
        DaemonConfigKey::SensorPreference.name(),
        "sensor_preference"
    );
    assert_eq!(DaemonConfigKey::CameraDevice.name(), "camera_device");
    let warnings = config.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("sensor_preference"), "{warnings:?}");
    assert!(!warnings[0].contains("31337"), "{warnings:?}");

    // Both keys mistyped: both fall back, both are named.
    let both = write_config(
        dir.path(),
        "both.toml",
        "[pipeline]\ncamera_device = true\nsensor_preference = 3\n",
    );
    let config = read_with_deadline(&both).unwrap();
    assert_eq!(config.settings, DaemonCameraSettings::default());
    assert_eq!(
        config.mistyped_keys,
        vec![
            DaemonConfigKey::CameraDevice,
            DaemonConfigKey::SensorPreference
        ]
    );
    assert_eq!(config.warnings().len(), 2);
}

#[test]
fn test_dgp_unrecognized_preference_is_warned_without_its_value() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "daemon.toml",
        "[pipeline]\nsensor_preference = \"thermal\"\n",
    );
    let config = read_with_deadline(&path).unwrap();
    assert_eq!(config.settings.sensor_preference, None);
    assert!(config.settings.sensor_preference_unrecognized);
    assert!(config.mistyped_keys.is_empty());
    let warnings = config.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("sensor_preference"), "{warnings:?}");
    assert!(warnings[0].contains("not recognized"), "{warnings:?}");
    assert!(!warnings[0].contains("thermal"), "{warnings:?}");
}

// ---------------------------------------------------------------------------
// DGP6: file-level failures stay errors (callers apply the defaults with a note)
// ---------------------------------------------------------------------------

#[test]
fn test_dgp_file_level_failures_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        read_with_deadline(&dir.path().join("absent.toml")).unwrap_err(),
        DaemonConfigError::NotFound
    );
    assert_eq!(
        read_with_deadline(dir.path()).unwrap_err(),
        DaemonConfigError::NotARegularFile
    );
    let malformed = write_config(dir.path(), "bad.toml", "[pipeline\ncamera_device = x\n");
    assert_eq!(
        read_with_deadline(&malformed).unwrap_err(),
        DaemonConfigError::Malformed
    );
    // `[pipeline]` itself of the wrong type is a structural error, not a key.
    let section = write_config(dir.path(), "section.toml", "pipeline = 5\n");
    assert_eq!(
        read_with_deadline(&section).unwrap_err(),
        DaemonConfigError::Malformed
    );
    let not_utf8 = dir.path().join("latin1.toml");
    std::fs::write(&not_utf8, b"[pipeline]\ncamera_device = \"\xe9\"\n").unwrap();
    assert_eq!(
        read_with_deadline(&not_utf8).unwrap_err(),
        DaemonConfigError::Malformed
    );

    let oversized = dir.path().join("big.toml");
    let mut body = "# padding\n".repeat((MAX_DAEMON_CONFIG_BYTES as usize) / 10 + 1);
    body.truncate(MAX_DAEMON_CONFIG_BYTES as usize + 1);
    std::fs::write(&oversized, body).unwrap();
    assert_eq!(
        read_with_deadline(&oversized).unwrap_err(),
        DaemonConfigError::TooLarge {
            limit: MAX_DAEMON_CONFIG_BYTES
        }
    );
    // Exactly at the cap is still read.
    let at_cap = dir.path().join("cap.toml");
    let mut body = "[pipeline]\ncamera_device = \"/dev/video2\"\n".to_string();
    body.push_str(&"#".repeat(MAX_DAEMON_CONFIG_BYTES as usize - body.len()));
    std::fs::write(&at_cap, body).unwrap();
    assert_eq!(
        read_with_deadline(&at_cap).unwrap().settings.camera_device,
        Some(PathBuf::from("/dev/video2"))
    );

    let message = DaemonConfigError::NotARegularFile.to_string();
    assert_eq!(message, "is not a regular file");
}

// ---------------------------------------------------------------------------
// DGP7: open-then-fstat; symbolic links are followed, a FIFO or a device never blocks the read
// ---------------------------------------------------------------------------

#[test]
fn test_dgp_fifo_in_place_of_the_file_returns_promptly() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("daemon.toml");
    make_fifo(&fifo);
    // No writer ever opens the FIFO: a blocking open() would wait forever.
    assert_eq!(
        read_with_deadline(&fifo).unwrap_err(),
        DaemonConfigError::NotARegularFile
    );
}

#[test]
fn test_dgp_symbolic_link_is_followed_like_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let target = write_config(
        dir.path(),
        "real.toml",
        "[pipeline]\ncamera_device = \"/dev/video4\"\nsensor_preference = \"rgb\"\n",
    );
    // Symlink-managed /etc (stow, NixOS, ostree): the clients read the file the daemon reads.
    let link = dir.path().join("daemon.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let config = read_with_deadline(&link).unwrap();
    assert_eq!(
        config.settings.camera_device,
        Some(PathBuf::from("/dev/video4"))
    );
    assert_eq!(
        config.settings.sensor_preference,
        Some(SensorPreference::PreferRgb)
    );
    // A chain of links is followed too.
    let chain = dir.path().join("chain.toml");
    std::os::unix::fs::symlink(&link, &chain).unwrap();
    assert_eq!(read_with_deadline(&chain).unwrap(), config);
}

#[test]
fn test_dgp_symbolic_link_to_a_fifo_or_device_returns_promptly() {
    let dir = tempfile::tempdir().unwrap();
    // The FIFO has no writer: a blocking open() through the link would wait forever.
    let fifo = dir.path().join("pipe");
    make_fifo(&fifo);
    let fifo_link = dir.path().join("fifo-link.toml");
    std::os::unix::fs::symlink(&fifo, &fifo_link).unwrap();
    assert_eq!(
        read_with_deadline(&fifo_link).unwrap_err(),
        DaemonConfigError::NotARegularFile
    );
    // A character device (endless or not) is never read either.
    let device_link = dir.path().join("device-link.toml");
    std::os::unix::fs::symlink("/dev/zero", &device_link).unwrap();
    assert_eq!(
        read_with_deadline(&device_link).unwrap_err(),
        DaemonConfigError::NotARegularFile
    );
}

#[test]
fn test_dgp_dangling_symbolic_link_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("daemon.toml");
    std::os::unix::fs::symlink(dir.path().join("absent.toml"), &link).unwrap();
    assert_eq!(
        read_with_deadline(&link).unwrap_err(),
        DaemonConfigError::NotFound
    );
}

#[test]
fn test_dgp_unreadable_file_is_reported() {
    if is_root() {
        return; // root bypasses the permission bits
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let locked = write_config(
        dir.path(),
        "daemon.toml",
        "[pipeline]\nsensor_preference = \"rgb\"\n",
    );
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let err = read_with_deadline(&locked).unwrap_err();
    assert!(matches!(err, DaemonConfigError::Unreadable(_)), "{err:?}");
}

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2).map(|euid| euid == "0"))
        })
        .unwrap_or(false)
}

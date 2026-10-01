//! Contract for `soos-admin camera list` reading `/etc/soos/daemon.toml` (GitHub #287,
//! matrix rows CVF1-CVF3).
//!
//! `camera list` resolves with the daemon's own `[pipeline] camera_device` and
//! `[pipeline] sensor_preference`; `--device` / `--sensor-preference` override them, and a
//! missing, unreadable, oversized or malformed file falls back to the soos-daemon defaults with
//! a note. Every test uses a temporary configuration file, never `/etc`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use clap::{CommandFactory, FromArgMatches};
use soos_admin_cli::args::{camera_list_sensor_preference_given, CameraAction, Cli, Commands};
use soos_admin_cli::camera::{resolve_list_settings, SettingOrigin};
use soos_admin_cli::daemon_config::{
    read_daemon_camera_settings, DaemonCameraSettings, DaemonConfigError,
    DEFAULT_DAEMON_CONFIG_PATH, MAX_DAEMON_CONFIG_BYTES,
};
use soos_camera_v4l::SensorPreference;
use std::path::{Path, PathBuf};

fn write_config(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("daemon.toml");
    std::fs::write(&path, body).unwrap();
    path
}

// ---------------------------------------------------------------------------
// CVF1: the daemon configuration is read with the daemon's vocabulary
// ---------------------------------------------------------------------------

#[test]
fn test_cvf_camera_list_reads_daemon_config_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "log_level = \"info\"\n\n[pipeline]\ncamera_device = \"/dev/video4\"\n\
         sensor_preference = \"rgb\"\nwarmup_frames = 0\n",
    );

    let settings = read_daemon_camera_settings(&path).unwrap();
    assert_eq!(
        settings,
        DaemonCameraSettings {
            camera_device: Some(PathBuf::from("/dev/video4")),
            sensor_preference: Some(SensorPreference::PreferRgb),
            sensor_preference_unrecognized: false,
        }
    );

    let resolved = resolve_list_settings(None, None, &path);
    assert_eq!(resolved.explicit_device, Some(PathBuf::from("/dev/video4")));
    assert_eq!(resolved.device_origin, SettingOrigin::DaemonConfig);
    assert_eq!(resolved.sensor_preference, SensorPreference::PreferRgb);
    assert_eq!(resolved.preference_origin, SettingOrigin::DaemonConfig);
    let note = resolved.note();
    assert!(note.contains(&path.display().to_string()), "{note}");
    assert!(note.contains("/dev/video4 (daemon.toml)"), "{note}");
    assert!(note.contains("prefer_rgb (daemon.toml)"), "{note}");

    // No [pipeline] section: the daemon keeps auto-detection and prefer_ir.
    let empty = write_config(dir.path(), "log_level = \"debug\"\n");
    assert_eq!(
        read_daemon_camera_settings(&empty).unwrap(),
        DaemonCameraSettings::default()
    );
    let resolved = resolve_list_settings(None, None, &empty);
    assert_eq!(resolved.explicit_device, None);
    assert_eq!(resolved.device_origin, SettingOrigin::Default);
    assert_eq!(resolved.sensor_preference, SensorPreference::PreferIr);
    assert_eq!(resolved.preference_origin, SettingOrigin::Default);
}

#[test]
fn test_cvf_daemon_config_sentinel_device_and_unknown_preference() {
    let dir = tempfile::tempdir().unwrap();
    for sentinel in ["auto", "default", "", "/dev/v4l/by-id/default-camera"] {
        let path = write_config(
            dir.path(),
            &format!("[pipeline]\ncamera_device = \"{sentinel}\"\n"),
        );
        let resolved = resolve_list_settings(None, None, &path);
        assert_eq!(
            resolved.explicit_device, None,
            "'{sentinel}' keeps auto-detection, exactly like soos-daemon"
        );
        assert!(resolved.note().contains("auto"), "{}", resolved.note());
    }

    // soos-daemon ignores an unrecognized sensor_preference and keeps prefer_ir: say so.
    let path = write_config(dir.path(), "[pipeline]\nsensor_preference = \"thermal\"\n");
    let settings = read_daemon_camera_settings(&path).unwrap();
    assert_eq!(settings.sensor_preference, None);
    assert!(settings.sensor_preference_unrecognized);
    let resolved = resolve_list_settings(None, None, &path);
    assert_eq!(resolved.sensor_preference, SensorPreference::PreferIr);
    assert_eq!(resolved.preference_origin, SettingOrigin::Default);
    let note = resolved.note();
    assert!(note.contains("not recognized"), "{note}");
    assert!(
        !note.contains("thermal"),
        "the note names the key, not the raw value: {note}"
    );
}

// ---------------------------------------------------------------------------
// CVF2: CLI flags override the configuration
// ---------------------------------------------------------------------------

#[test]
fn test_cvf_camera_list_flags_override_daemon_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "[pipeline]\ncamera_device = \"/dev/video4\"\nsensor_preference = \"rgb\"\n",
    );
    let resolved = resolve_list_settings(
        Some(Path::new("/dev/video9")),
        Some(SensorPreference::Any),
        &path,
    );
    assert_eq!(resolved.explicit_device, Some(PathBuf::from("/dev/video9")));
    assert_eq!(resolved.device_origin, SettingOrigin::CommandLine);
    assert_eq!(resolved.sensor_preference, SensorPreference::Any);
    assert_eq!(resolved.preference_origin, SettingOrigin::CommandLine);
    let note = resolved.note();
    assert!(note.contains("/dev/video9 (--device)"), "{note}");
    assert!(note.contains("any (--sensor-preference)"), "{note}");

    // `--device auto` re-enables auto-detection over a configured device.
    let resolved = resolve_list_settings(Some(Path::new("auto")), None, &path);
    assert_eq!(resolved.device_origin, SettingOrigin::CommandLine);
    assert_eq!(resolved.explicit_device, None);
    assert_eq!(resolved.sensor_preference, SensorPreference::PreferRgb);
}

#[test]
fn test_cvf_camera_list_config_flag_and_explicit_preference_detection() {
    let parse = |argv: &[&str]| {
        let matches = Cli::command().try_get_matches_from(argv).unwrap();
        let cli = Cli::from_arg_matches(&matches).unwrap();
        let Commands::Camera(args) = cli.command else {
            panic!("expected the camera subcommand");
        };
        let CameraAction::List(list) = args.action else {
            panic!("expected camera list");
        };
        (camera_list_sensor_preference_given(&matches), list)
    };

    let (given, list) = parse(&["soos-admin", "camera", "list"]);
    assert!(!given, "the clap default is not an explicit preference");
    assert_eq!(list.config, PathBuf::from(DEFAULT_DAEMON_CONFIG_PATH));
    assert_eq!(DEFAULT_DAEMON_CONFIG_PATH, "/etc/soos/daemon.toml");

    let (given, list) = parse(&[
        "soos-admin",
        "camera",
        "list",
        "--sensor-preference",
        "prefer_ir",
        "--config",
        "/tmp/soos-test/daemon.toml",
    ]);
    assert!(
        given,
        "an explicit --sensor-preference overrides daemon.toml"
    );
    assert_eq!(list.sensor_preference, SensorPreference::PreferIr);
    assert_eq!(list.config, PathBuf::from("/tmp/soos-test/daemon.toml"));

    // Other subcommands never report an explicit camera preference.
    let matches = Cli::command()
        .try_get_matches_from(["soos-admin", "status"])
        .unwrap();
    assert!(!camera_list_sensor_preference_given(&matches));
}

// ---------------------------------------------------------------------------
// CVF3: missing / unreadable / oversized / malformed files fall back with a note
// ---------------------------------------------------------------------------

#[test]
fn test_cvf_missing_or_invalid_daemon_config_falls_back_with_note() {
    let dir = tempfile::tempdir().unwrap();

    let missing = dir.path().join("absent.toml");
    assert_eq!(
        read_daemon_camera_settings(&missing),
        Err(DaemonConfigError::NotFound)
    );
    let resolved = resolve_list_settings(None, None, &missing);
    assert_eq!(resolved.explicit_device, None);
    assert_eq!(resolved.sensor_preference, SensorPreference::PreferIr);
    assert_eq!(resolved.preference_origin, SettingOrigin::Default);
    let note = resolved.note();
    assert!(note.contains("not found"), "{note}");
    assert!(note.contains("soos-daemon defaults"), "{note}");
    assert!(note.contains("prefer_ir (default)"), "{note}");

    // A directory is not a configuration file.
    assert_eq!(
        read_daemon_camera_settings(dir.path()),
        Err(DaemonConfigError::NotARegularFile)
    );

    let malformed = write_config(dir.path(), "[pipeline\ncamera_device = /dev/video0\n");
    assert_eq!(
        read_daemon_camera_settings(&malformed),
        Err(DaemonConfigError::Malformed)
    );
    let note = resolve_list_settings(None, None, &malformed).note();
    assert!(note.contains("cannot be parsed"), "{note}");
    assert!(note.contains("soos-daemon defaults"), "{note}");

    // A key of the wrong type makes the daemon refuse the file: report it as malformed.
    let wrong_type = write_config(dir.path(), "[pipeline]\ncamera_device = 5\n");
    assert_eq!(
        read_daemon_camera_settings(&wrong_type),
        Err(DaemonConfigError::Malformed)
    );

    // Bounded read: one byte over the cap is refused before parsing.
    let oversized = dir.path().join("big.toml");
    let mut body = "# padding\n".repeat((MAX_DAEMON_CONFIG_BYTES as usize) / 10 + 1);
    body.truncate(MAX_DAEMON_CONFIG_BYTES as usize + 1);
    std::fs::write(&oversized, body).unwrap();
    assert_eq!(
        read_daemon_camera_settings(&oversized),
        Err(DaemonConfigError::TooLarge {
            limit: MAX_DAEMON_CONFIG_BYTES
        })
    );

    // Unreadable (EACCES): only meaningful when not running as root.
    if !nix_is_root() {
        use std::os::unix::fs::PermissionsExt;
        let locked = write_config(dir.path(), "[pipeline]\nsensor_preference = \"rgb\"\n");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = read_daemon_camera_settings(&locked);
        assert!(
            matches!(result, Err(DaemonConfigError::Unreadable(_))),
            "{result:?}"
        );
        let resolved = resolve_list_settings(None, None, &locked);
        assert_eq!(resolved.sensor_preference, SensorPreference::PreferIr);
        assert!(
            resolved.note().contains("cannot be read"),
            "{}",
            resolved.note()
        );
    }
}

fn nix_is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2).map(|euid| euid == "0"))
        })
        .unwrap_or(false)
}

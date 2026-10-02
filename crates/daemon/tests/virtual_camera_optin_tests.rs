//! Contract tests for GitHub #318 (rows VCO1, VCO2): the `soos-daemon` configuration loader
//! reads `[pipeline] allow_virtual_camera` into `CameraConfig::allow_virtual_device` (default
//! `false`, fail closed; a warning is queued for the startup log when it is enabled), and
//! refuses a `camera_device` holding a NUL byte at load time with a configuration error.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_daemon::config::DaemonConfig;
use soos_daemon::error::DaemonError;

fn accepted(toml: &str) -> DaemonConfig {
    DaemonConfig::from_toml_str(toml)
        .unwrap_or_else(|e| panic!("`{toml}` must be accepted, got {e:?}"))
}

fn rejected(toml: &str) -> String {
    match DaemonConfig::from_toml_str(toml) {
        Err(DaemonError::Config(msg)) => msg,
        Err(other) => panic!("`{toml}` must yield DaemonError::Config, got {other:?}"),
        Ok(_) => panic!("`{toml}` must be refused, but was accepted"),
    }
}

fn virtual_camera_warnings(config: &DaemonConfig) -> Vec<&String> {
    config
        .warnings
        .iter()
        .filter(|w| w.contains("allow_virtual_camera"))
        .collect()
}

/// VCO1: `allow_virtual_camera = true` reaches the camera configuration, and the daemon
/// queues a startup warning naming the key (it weakens the camera trust boundary).
#[test]
fn test_vco_daemon_allow_virtual_camera_true_allows_virtual_devices() {
    let config = accepted("[pipeline]\nallow_virtual_camera = true\n");
    assert!(
        config.pipeline.camera.allow_virtual_device,
        "[pipeline] allow_virtual_camera = true must set CameraConfig::allow_virtual_device"
    );
    let warnings = virtual_camera_warnings(&config);
    assert_eq!(
        warnings.len(),
        1,
        "exactly one startup warning must name allow_virtual_camera: {:?}",
        config.warnings
    );
    assert!(
        warnings[0].contains("virtual"),
        "the warning must say a virtual camera node may be opened: {}",
        warnings[0]
    );
}

/// VCO1: absent, empty `[pipeline]`, explicit `false` and no file at all keep virtual nodes
/// refused, without any warning.
#[test]
fn test_vco_daemon_allow_virtual_camera_defaults_false() {
    for toml in [
        "",
        "[pipeline]\n",
        "[pipeline]\nallow_virtual_camera = false\n",
    ] {
        let config = accepted(toml);
        assert!(
            !config.pipeline.camera.allow_virtual_device,
            "`{toml}` must keep virtual nodes refused"
        );
        assert!(
            virtual_camera_warnings(&config).is_empty(),
            "`{toml}` must not warn: {:?}",
            config.warnings
        );
    }
    assert!(
        !DaemonConfig::runtime_default()
            .pipeline
            .camera
            .allow_virtual_device
    );
    assert!(!DaemonConfig::default().pipeline.camera.allow_virtual_device);
}

/// VCO1: a mistyped value is a configuration error like every other typed daemon key; it can
/// never be read as an opt-in.
#[test]
fn test_vco_daemon_allow_virtual_camera_mistyped_is_refused() {
    for toml in [
        "[pipeline]\nallow_virtual_camera = \"yes\"\n",
        "[pipeline]\nallow_virtual_camera = 1\n",
    ] {
        let msg = rejected(toml);
        assert!(
            msg.contains("allow_virtual_camera"),
            "the error for `{toml}` must name the key: {msg}"
        );
    }
}

/// VCO2: a NUL byte in `camera_device` is refused at load time (from a string and from a
/// file), naming the key and never echoing the value.
#[test]
fn test_vco_daemon_rejects_nul_byte_camera_device() {
    let toml = "[pipeline]\ncamera_device = \"/dev/vid\\u0000eo-secret\"\n";
    let msg = rejected(toml);
    assert!(msg.contains("camera_device"), "must name the key: {msg}");
    assert!(msg.contains("NUL"), "must say why: {msg}");
    assert!(!msg.contains("secret"), "must not echo the value: {msg}");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.toml");
    std::fs::write(&path, toml).unwrap();
    match DaemonConfig::load_from_path(&path) {
        Err(DaemonError::Config(msg)) => assert!(msg.contains("camera_device"), "{msg}"),
        other => panic!("load_from_path must refuse a NUL camera_device, got {other:?}"),
    }

    // A clean path is still accepted.
    let config = accepted("[pipeline]\ncamera_device = \"/dev/video4\"\n");
    assert_eq!(
        config.pipeline.camera.device_path,
        std::path::PathBuf::from("/dev/video4")
    );
}

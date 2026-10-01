//! `soos-enroll` and `soos-gui` read `daemon.toml` through the shared reader of
//! `soos-camera-v4l` (GitHub #289, rows DGP5-DGP7).
//!
//! `resolve_camera_device_from_config_reported` returns the resolved device and the notes the
//! binaries print: a wrongly typed key falls back alone and is named (never quoted), a
//! missing / oversized / non-regular / symbolic-link configuration falls back to the soos-daemon
//! defaults with a note, and a FIFO in place of the file never blocks.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use soos_camera_v4l::daemon_config::MAX_DAEMON_CONFIG_BYTES;
use soos_camera_v4l::{CameraDeviceInfo, CameraEnumerator, PixelFormat};
use soos_enrollment_cli::service::{
    resolve_camera_device_from_config_reported, CameraDeviceChoice,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

struct FakeEnumerator;

impl CameraEnumerator for FakeEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        vec![
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video0"),
                card_name: "Integrated Camera: Integrated C".to_string(),
                supported_formats: vec![PixelFormat::Mjpeg, PixelFormat::Yuyv],
            },
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video2"),
                card_name: "Integrated Camera: Integrated I".to_string(),
                supported_formats: vec![PixelFormat::Grey],
            },
        ]
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        vec![
            (
                PathBuf::from("/dev/v4l/by-id/usb-BBB_Cam_RGB-video-index0"),
                PathBuf::from("/dev/video0"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-CCC_Cam_IR-video-index0"),
                PathBuf::from("/dev/video2"),
            ),
        ]
    }
}

const IR_ALIAS: &str = "/dev/v4l/by-id/usb-CCC_Cam_IR-video-index0";
const RGB_ALIAS: &str = "/dev/v4l/by-id/usb-BBB_Cam_RGB-video-index0";

fn write_config(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("daemon.toml");
    std::fs::write(&path, body).unwrap();
    path
}

fn resolve_with_deadline(path: &Path) -> CameraDeviceChoice {
    let (tx, rx) = mpsc::channel();
    let owned = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(resolve_camera_device_from_config_reported(
            None,
            Some(&owned),
            &FakeEnumerator,
        ));
    });
    rx.recv_timeout(Duration::from_secs(2))
        .expect("the camera resolution must not block on the configuration path")
}

#[test]
fn test_dgp_enroll_valid_config_has_no_notes() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = write_config(tmp.path(), "[pipeline]\ncamera_device = \"/dev/video42\"\n");
    let choice = resolve_with_deadline(&cfg);
    assert_eq!(choice.path, PathBuf::from("/dev/video42"));
    assert!(choice.notes.is_empty(), "{:?}", choice.notes);

    // No configuration path at all: the daemon defaults, nothing to report.
    let choice = resolve_camera_device_from_config_reported(None, None, &FakeEnumerator);
    assert_eq!(choice.path, PathBuf::from(IR_ALIAS));
    assert!(choice.notes.is_empty());
}

#[test]
fn test_dgp_enroll_wrongly_typed_device_keeps_the_preference() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = write_config(
        tmp.path(),
        "[pipeline]\ncamera_device = 98765\nsensor_preference = \"prefer_rgb\"\n",
    );
    let choice = resolve_with_deadline(&cfg);
    assert_eq!(
        choice.path,
        PathBuf::from(RGB_ALIAS),
        "camera_device falls back to auto-detection, sensor_preference still applies"
    );
    assert_eq!(choice.notes.len(), 1, "{:?}", choice.notes);
    assert!(
        choice.notes[0].contains("camera_device"),
        "{:?}",
        choice.notes
    );
    assert!(choice.notes[0].contains("wrong type"), "{:?}", choice.notes);
    assert!(!choice.notes[0].contains("98765"), "{:?}", choice.notes);
}

#[test]
fn test_dgp_enroll_wrongly_typed_preference_keeps_the_device() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = write_config(
        tmp.path(),
        "[pipeline]\ncamera_device = \"/dev/video42\"\nsensor_preference = 7\n",
    );
    let choice = resolve_with_deadline(&cfg);
    assert_eq!(choice.path, PathBuf::from("/dev/video42"));
    assert_eq!(choice.notes.len(), 1, "{:?}", choice.notes);
    assert!(
        choice.notes[0].contains("sensor_preference"),
        "{:?}",
        choice.notes
    );
}

#[test]
fn test_dgp_enroll_unusable_config_falls_back_with_note() {
    let tmp = tempfile::tempdir().unwrap();

    let missing = tmp.path().join("absent.toml");
    let choice = resolve_with_deadline(&missing);
    assert_eq!(choice.path, PathBuf::from(IR_ALIAS));
    assert_eq!(choice.notes.len(), 1, "{:?}", choice.notes);
    assert!(choice.notes[0].contains("not found"), "{:?}", choice.notes);
    assert!(
        choice.notes[0].contains("soos-daemon defaults"),
        "{:?}",
        choice.notes
    );

    // Bounded: an oversized file is refused instead of being read whole and applied.
    let mut body = "[pipeline]\ncamera_device = \"/dev/video42\"\n".to_string();
    body.push_str(&"#".repeat(MAX_DAEMON_CONFIG_BYTES as usize));
    let big = write_config(tmp.path(), &body);
    let choice = resolve_with_deadline(&big);
    assert_eq!(choice.path, PathBuf::from(IR_ALIAS));
    assert!(
        choice.notes[0].contains("larger than"),
        "{:?}",
        choice.notes
    );

    // A symbolic link is not followed.
    let real = tmp.path().join("real.toml");
    std::fs::write(&real, "[pipeline]\ncamera_device = \"/dev/video42\"\n").unwrap();
    let link = tmp.path().join("link.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let choice = resolve_with_deadline(&link);
    assert_eq!(choice.path, PathBuf::from(IR_ALIAS));
    assert!(
        choice.notes[0].contains("symbolic link"),
        "{:?}",
        choice.notes
    );
}

#[test]
fn test_dgp_enroll_fifo_config_returns_promptly() {
    let tmp = tempfile::tempdir().unwrap();
    let fifo = tmp.path().join("daemon.toml");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    let choice = resolve_with_deadline(&fifo);
    assert_eq!(choice.path, PathBuf::from(IR_ALIAS));
    assert_eq!(choice.notes.len(), 1, "{:?}", choice.notes);
    assert!(
        choice.notes[0].contains("is not a regular file"),
        "{:?}",
        choice.notes
    );
}

/// `soos-enroll` prints `camera_config_notes` before it opens the camera: the same notes as
/// the resolver reports.
#[test]
fn test_dgp_enroll_camera_config_notes_match_the_resolver() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = write_config(
        tmp.path(),
        "[pipeline]\ncamera_device = 1\nsensor_preference = \"thermal\"\n",
    );
    let notes = soos_enrollment_cli::service::camera_config_notes(&cfg);
    assert_eq!(notes, resolve_with_deadline(&cfg).notes);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes.iter().all(|n| n.contains(&cfg.display().to_string())));
    assert!(notes.iter().all(|n| !n.contains("thermal")), "{notes:?}");
    let valid = write_config(tmp.path(), "[pipeline]\nsensor_preference = \"rgb\"\n");
    assert!(soos_enrollment_cli::service::camera_config_notes(&valid).is_empty());
}

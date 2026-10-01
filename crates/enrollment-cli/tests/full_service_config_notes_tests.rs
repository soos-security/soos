//! GitHub #291 (SGU6): the `daemon.toml` notes `soos-enroll` prints describe the very read
//! `build_full_service` applies.
//!
//! `build_full_service_with_notes` resolves the camera once through
//! `resolve_camera_device_from_config_reported` and hands that call's notes to the caller before
//! the camera is validated or opened, so the printed notes and the applied configuration can
//! never come from two different reads of the file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use std::path::{Path, PathBuf};

use soos_enrollment_cli::args::{Cli, Commands, VerifyArgs};
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::{build_full_service_with_notes, camera_config_notes};

fn cli(camera_device: Option<PathBuf>) -> Cli {
    Cli {
        biometrics_dir: Some(PathBuf::from("/var/lib/soos/biometrics")),
        key_file: Some(PathBuf::from("/var/lib/soos/master.key")),
        models_dir: Some(PathBuf::from("/var/lib/soos/models")),
        camera_device,
        mock: false,
        command: Commands::Verify(VerifyArgs {
            uid: Some(1000),
            username: None,
        }),
    }
}

fn write_config(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("daemon.toml");
    std::fs::write(&path, body).unwrap();
    path
}

/// The configured (non-`/dev`) device is what the build applies, and the one note names the
/// mistyped key of that same file: one read, reported before the camera is validated.
#[test]
fn test_sgu_full_service_reports_the_notes_of_the_applied_config() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = write_config(
        tmp.path(),
        "[pipeline]\ncamera_device = \"/srv/sgu-not-a-device\"\nsensor_preference = 3\n",
    );

    let mut calls: Vec<Vec<String>> = Vec::new();
    let result = build_full_service_with_notes(&cli(None), &cfg, &mut |notes| {
        calls.push(notes.to_vec());
    });

    match result {
        Err(EnrollmentCliError::InvalidPath(msg)) => assert!(
            msg.contains("/srv/sgu-not-a-device"),
            "the device of the noted file is the one applied: {msg}"
        ),
        Err(other) => panic!("expected InvalidPath for the configured device, got {other:?}"),
        Ok(_) => panic!("expected InvalidPath for the configured device, got a service"),
    }
    assert_eq!(calls.len(), 1, "the notes are reported exactly once");
    assert_eq!(calls[0], camera_config_notes(&cfg));
    assert_eq!(calls[0].len(), 1, "{:?}", calls[0]);
    assert!(calls[0][0].contains("sensor_preference"), "{:?}", calls[0]);
}

/// A configuration that applies as written produces no note, and the callback still runs
/// once (with an empty slice) before the build continues.
#[test]
fn test_sgu_full_service_reports_empty_notes_for_a_clean_config() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = write_config(
        tmp.path(),
        "[pipeline]\ncamera_device = \"/srv/sgu-clean\"\n",
    );

    let mut calls: Vec<Vec<String>> = Vec::new();
    let result = build_full_service_with_notes(&cli(None), &cfg, &mut |notes| {
        calls.push(notes.to_vec());
    });

    assert!(matches!(result, Err(EnrollmentCliError::InvalidPath(_))));
    assert_eq!(calls, vec![Vec::<String>::new()]);
}

/// An unusable configuration (missing file) is noted once, and the CLI device still applies.
#[test]
fn test_sgu_full_service_notes_a_missing_config_once() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("absent.toml");

    let mut calls: Vec<Vec<String>> = Vec::new();
    let result = build_full_service_with_notes(
        &cli(Some(PathBuf::from("/etc/shadow"))),
        &missing,
        &mut |notes| calls.push(notes.to_vec()),
    );

    match result {
        Err(EnrollmentCliError::InvalidPath(msg)) => assert!(msg.contains("/etc/shadow"), "{msg}"),
        Err(other) => panic!("expected InvalidPath, got {other:?}"),
        Ok(_) => panic!("expected InvalidPath, got a service"),
    }
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], camera_config_notes(&missing));
    assert_eq!(calls[0].len(), 1, "{:?}", calls[0]);
}

/// `soos-enroll` prints the notes of the build's own read: `main.rs` no longer reads
/// `daemon.toml` a second time through `camera_config_notes`.
#[test]
fn test_sgu_enroll_main_prints_the_notes_of_the_build_read() {
    let main =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")).unwrap();
    assert!(
        !main.contains("camera_config_notes("),
        "main.rs must not read daemon.toml separately from the build"
    );
    assert!(
        !main.contains("build_full_service(&cli)"),
        "every camera command goes through build_full_service_with_notes"
    );
    assert!(
        main.contains("build_full_service_with_notes("),
        "enroll, verify and debug-vision print the notes of their own build"
    );
}

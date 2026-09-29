#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

//! Contract tests for `soos-enroll debug-vision` report safety (GitHub #149 / STO-02).
//!
//! The diagnostic report may contain a raw camera frame (biometric data) and the
//! command runs as root, so the output file must be created atomically with mode
//! `0600`, must never follow a pre-planted symlink or truncate an existing file, and
//! must embed the frame only when the administrator explicitly asks for it.

use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use tempfile::TempDir;

use soos_biometric_store::{BiometricStore, MasterKey};
use soos_camera_v4l::{CameraConfig, MockCameraManager};
use soos_enrollment_cli::args::{Cli, Commands, DebugVisionArgs};
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::{
    ensure_debug_report_dir, EnrollmentService, DEBUG_REPORT_DIR_MODE, DEBUG_REPORT_FILE_MODE,
    DEFAULT_DEBUG_REPORT_DIR,
};
use soos_inference_ort::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

/// Upper bound of a geometry-only report: a 640x480 RGB frame alone is 921,600 bytes
/// (about 1.2 MiB once base64-encoded), so anything below 64 KiB cannot hold the frame.
const GEOMETRY_ONLY_REPORT_MAX_BYTES: usize = 64 * 1024;

fn setup_service(temp: &TempDir, detector: Arc<MockFaceDetector>) -> EnrollmentService {
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("biometrics"), key).unwrap());

    let pipeline = Arc::new(VisionPipeline::new(
        detector,
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig::default(),
    ));

    let camera_config = CameraConfig {
        warmup_frames: 0,
        ..Default::default()
    };
    let camera = Arc::new(MockCameraManager::new(camera_config));

    EnrollmentService::new(store, camera, pipeline, false)
}

fn centered_face_detector() -> Arc<MockFaceDetector> {
    Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95))
}

fn args_with_output(output: &Path, embed_frame: bool) -> DebugVisionArgs {
    DebugVisionArgs {
        output: Some(output.to_path_buf()),
        embed_frame,
    }
}

fn file_mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn test_debug_vision_creates_report_with_mode_0600() {
    let temp = TempDir::new().unwrap();
    let service = setup_service(&temp, centered_face_detector());
    let output = temp.path().join("report.html");

    let written = service
        .debug_vision(&args_with_output(&output, false))
        .expect("debug-vision must succeed on a fresh output path");

    assert_eq!(
        written, output,
        "returned path must be the requested output"
    );
    let meta = std::fs::symlink_metadata(&output).unwrap();
    assert!(meta.is_file(), "report must be a regular file");
    assert_eq!(
        meta.permissions().mode() & 0o777,
        DEBUG_REPORT_FILE_MODE,
        "report must be created with mode 0600 regardless of umask"
    );
    assert_eq!(DEBUG_REPORT_FILE_MODE, 0o600);
    let html = std::fs::read_to_string(&output).unwrap();
    assert!(
        html.contains("<!DOCTYPE html>"),
        "report must be an HTML document"
    );
}

#[test]
fn test_debug_vision_refuses_symlink_output_and_leaves_target_untouched() {
    let temp = TempDir::new().unwrap();
    let service = setup_service(&temp, centered_face_detector());

    let victim = temp.path().join("victim.txt");
    std::fs::write(&victim, b"do not clobber").unwrap();
    let link = temp.path().join("report.html");
    symlink(&victim, &link).unwrap();

    let res = service.debug_vision(&args_with_output(&link, true));
    match res {
        Err(EnrollmentCliError::DebugReportRefused { path, .. }) => assert_eq!(path, link),
        other => panic!("Expected DebugReportRefused for a symlinked output, got: {other:?}"),
    }

    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"do not clobber",
        "symlink target must not be truncated or overwritten"
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the pre-planted symlink must be left in place, not replaced"
    );
}

#[test]
fn test_debug_vision_refuses_existing_file_without_truncation() {
    let temp = TempDir::new().unwrap();
    let service = setup_service(&temp, centered_face_detector());

    let output = temp.path().join("report.html");
    std::fs::write(&output, b"previous report").unwrap();

    let res = service.debug_vision(&args_with_output(&output, false));
    assert!(
        matches!(res, Err(EnrollmentCliError::DebugReportRefused { .. })),
        "Expected DebugReportRefused for an existing file, got: {res:?}"
    );
    assert_eq!(
        std::fs::read(&output).unwrap(),
        b"previous report",
        "an existing file must never be truncated"
    );
}

#[test]
fn test_debug_vision_refuses_symlinked_parent_directory() {
    let temp = TempDir::new().unwrap();
    let service = setup_service(&temp, centered_face_detector());

    let real_dir = temp.path().join("real");
    std::fs::create_dir(&real_dir).unwrap();
    let linked_dir = temp.path().join("linked");
    symlink(&real_dir, &linked_dir).unwrap();

    let output = linked_dir.join("report.html");
    let res = service.debug_vision(&args_with_output(&output, false));
    assert!(
        matches!(res, Err(EnrollmentCliError::DebugReportRefused { .. })),
        "Expected DebugReportRefused for a symlinked parent directory, got: {res:?}"
    );
    assert!(
        !real_dir.join("report.html").exists(),
        "no report may be written through a symlinked directory"
    );
}

#[test]
fn test_debug_vision_rejects_traversal_output_path() {
    let temp = TempDir::new().unwrap();
    let service = setup_service(&temp, centered_face_detector());

    let traversal = temp.path().join("..").join("report.html");
    let res = service.debug_vision(&args_with_output(&traversal, false));
    assert!(
        matches!(res, Err(EnrollmentCliError::InvalidPath(_))),
        "Expected InvalidPath for a '..' component, got: {res:?}"
    );

    let relative = PathBuf::from("report.html");
    let res = service.debug_vision(&args_with_output(&relative, false));
    assert!(
        matches!(res, Err(EnrollmentCliError::InvalidPath(_))),
        "Expected InvalidPath for a relative path, got: {res:?}"
    );
}

#[test]
fn test_debug_vision_omits_frame_without_embed_flag() {
    let temp = TempDir::new().unwrap();
    let service = setup_service(&temp, centered_face_detector());
    let output = temp.path().join("geometry-only.html");

    service
        .debug_vision(&args_with_output(&output, false))
        .expect("debug-vision must succeed");

    let html = std::fs::read_to_string(&output).unwrap();
    assert!(
        html.len() < GEOMETRY_ONLY_REPORT_MAX_BYTES,
        "report without --embed-frame must not carry pixel data (size {} bytes)",
        html.len()
    );
    assert!(
        !html.contains("putImageData"),
        "report without --embed-frame must not decode any pixel payload"
    );
    assert!(
        html.contains("strokeRect"),
        "detection geometry must still be present in the geometry-only report"
    );
    assert!(
        html.contains("--embed-frame"),
        "geometry-only report must tell the administrator how to embed the frame"
    );
}

#[test]
fn test_debug_vision_embeds_frame_only_with_explicit_flag() {
    let temp = TempDir::new().unwrap();
    let service = setup_service(&temp, centered_face_detector());
    let output = temp.path().join("with-frame.html");

    service
        .debug_vision(&args_with_output(&output, true))
        .expect("debug-vision must succeed");

    let html = std::fs::read_to_string(&output).unwrap();
    assert!(
        html.len() > GEOMETRY_ONLY_REPORT_MAX_BYTES,
        "report with --embed-frame must carry the frame payload (size {} bytes)",
        html.len()
    );
    assert!(
        html.contains("putImageData"),
        "report with --embed-frame must render the embedded frame"
    );
    assert!(
        html.to_lowercase().contains("biometric"),
        "report with an embedded frame must warn that it contains biometric data"
    );
    assert_eq!(file_mode(&output), 0o600);
}

#[test]
fn test_debug_vision_propagates_detector_error() {
    let temp = TempDir::new().unwrap();
    let detector = centered_face_detector();
    detector.set_fail_next(true);
    let service = setup_service(&temp, detector);
    let output = temp.path().join("report.html");

    let res = service.debug_vision(&args_with_output(&output, false));
    assert!(
        matches!(res, Err(EnrollmentCliError::Inference(_))),
        "detector failure must be propagated, not reported as zero detections: {res:?}"
    );
    assert!(
        !output.exists(),
        "no report may be written when the detector failed"
    );
}

#[test]
fn test_ensure_debug_report_dir_creates_0700_and_refuses_symlink() {
    let temp = TempDir::new().unwrap();

    let fresh = temp.path().join("debug");
    ensure_debug_report_dir(&fresh).expect("fresh directory must be created");
    let meta = std::fs::symlink_metadata(&fresh).unwrap();
    assert!(meta.is_dir());
    assert_eq!(meta.permissions().mode() & 0o777, DEBUG_REPORT_DIR_MODE);
    assert_eq!(DEBUG_REPORT_DIR_MODE, 0o700);

    // An existing directory is accepted as is: its mode must not be rewritten.
    let existing = temp.path().join("existing");
    std::fs::create_dir(&existing).unwrap();
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o750)).unwrap();
    ensure_debug_report_dir(&existing).expect("existing directory must be accepted");
    assert_eq!(
        file_mode(&existing),
        0o750,
        "existing directory mode must be preserved"
    );

    // A symlink in place of the directory is refused.
    let target = temp.path().join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    let linked = temp.path().join("linked-debug");
    symlink(&target, &linked).unwrap();
    let res = ensure_debug_report_dir(&linked);
    assert!(
        matches!(res, Err(EnrollmentCliError::DebugReportRefused { .. })),
        "Expected DebugReportRefused for a symlinked report directory, got: {res:?}"
    );

    // A regular file in place of the directory is refused.
    let file = temp.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let res = ensure_debug_report_dir(&file);
    assert!(
        matches!(res, Err(EnrollmentCliError::DebugReportRefused { .. })),
        "Expected DebugReportRefused for a non-directory, got: {res:?}"
    );
}

#[test]
fn test_default_debug_report_dir_is_root_only_location() {
    assert_eq!(DEFAULT_DEBUG_REPORT_DIR, "/var/lib/soos/debug");
    assert!(Path::new(DEFAULT_DEBUG_REPORT_DIR).starts_with("/var/lib/soos"));
}

#[test]
fn test_cli_parse_debug_vision_subcommand_flags() {
    let cli = Cli::try_parse_from([
        "soos-enroll",
        "debug-vision",
        "--output",
        "/tmp/soos-report.html",
        "--embed-frame",
    ])
    .expect("Failed to parse debug-vision args");
    match cli.command {
        Commands::DebugVision(args) => {
            assert_eq!(args.output, Some(PathBuf::from("/tmp/soos-report.html")));
            assert!(args.embed_frame);
        }
        other => panic!("Expected DebugVision subcommand, got: {other:?}"),
    }

    let cli = Cli::try_parse_from(["soos-enroll", "debug-vision"])
        .expect("Failed to parse bare debug-vision");
    match cli.command {
        Commands::DebugVision(args) => {
            assert_eq!(
                args.output, None,
                "output defaults to the root-only directory"
            );
            assert!(
                !args.embed_frame,
                "the raw frame must never be embedded by default"
            );
        }
        other => panic!("Expected DebugVision subcommand, got: {other:?}"),
    }
}

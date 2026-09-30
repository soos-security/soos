//! Contractual tests for `soos-enroll` output and overwrite hygiene (GitHub #232 STO-16,
//! #233 STO-17, #237 STO-21).
//!
//! Contract:
//! - `list --format json` is produced by `serde_json` and round-trips any string content.
//! - `--help` and `--version` work without root; privileges are checked after parsing.
//! - `import` onto an already enrolled UID fails with `AlreadyEnrolled` unless `--yes` is
//!   given, and nothing is replaced; with `--yes` the outcome reports the replacement.
//! - The enrollment summary shown before confirmation says whether a template is replaced.
//! - `secure_shred_file` never follows a symbolic link and refuses non-regular files.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use clap::Parser;
use soos_biometric_store::{BiometricStore, MasterKey};
use soos_camera_v4l::{CameraConfig, MockCameraManager};
use soos_enrollment_cli::args::{Cli, Commands, EnrollArgs, ImportArgs};
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::{
    format_enrolled_json, EnrolledUserSummary, EnrollmentService, IMPORT_STDIN_PATH,
};
use soos_enrollment_cli::shred::secure_shred_file;
use soos_inference_ort::{
    BoundingBox, FaceDetection, FaceLandmarks, MockEmbeddingExtractor, MockFaceDetector,
    MockPadDetector, Point2f,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};
use tempfile::{tempdir, TempDir};

fn store_only(tmp: &TempDir) -> (EnrollmentService, Arc<BiometricStore>) {
    let key = MasterKey::load_or_create(tmp.path().join("master.key")).expect("master key");
    let store = Arc::new(BiometricStore::new(tmp.path().join("biometrics"), key).expect("store"));
    (
        EnrollmentService::new_store_only(Arc::clone(&store), false),
        store,
    )
}

fn full_service(tmp: &TempDir) -> (EnrollmentService, Arc<BiometricStore>) {
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(tmp.path().join("biometrics"), key).unwrap());
    let landmarks = FaceLandmarks {
        left_eye: Point2f { x: 38.0, y: 52.0 },
        right_eye: Point2f { x: 74.0, y: 52.0 },
        nose: Point2f { x: 56.0, y: 70.0 },
        mouth_left: Point2f { x: 42.0, y: 88.0 },
        mouth_right: Point2f { x: 70.0, y: 88.0 },
    };
    let detection = FaceDetection {
        box_: BoundingBox::new(20.0, 20.0, 80.0, 80.0),
        score: 0.95,
        landmarks: Some(landmarks),
    };
    let pipeline = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_with_detections(vec![detection])),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig::default(),
    ));
    let camera = Arc::new(MockCameraManager::new(CameraConfig {
        warmup_frames: 0,
        ..Default::default()
    }));
    (
        EnrollmentService::new(Arc::clone(&store), camera, pipeline, false),
        store,
    )
}

fn write_embedding(dir: &Path, name: &str, value: f32) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_string(&vec![value; 512]).unwrap()).unwrap();
    path
}

fn file_args(uid: u32, file: PathBuf) -> ImportArgs {
    ImportArgs {
        uid: Some(uid),
        username: None,
        file,
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    }
}

// ---------------------------------------------------------------- #232 JSON escaping

#[test]
fn test_list_json_escapes_quotes_and_backslashes() {
    let summaries = vec![
        EnrolledUserSummary {
            uid: 1000,
            username: "ali\"ce".to_string(),
            model_id: "model\\id\"x".to_string(),
            model_version: "2.0.0\n\"beta\"".to_string(),
            enrollment_timestamp: 1_700_000_000,
            embedding_dim: 512,
        },
        EnrolledUserSummary {
            uid: 1001,
            username: "bob".to_string(),
            model_id: "arcface_w600k_mbf".to_string(),
            model_version: "2.0.0".to_string(),
            enrollment_timestamp: 0,
            embedding_dim: 512,
        },
    ];
    let json = format_enrolled_json(&summaries);
    let parsed: Vec<EnrolledUserSummary> =
        serde_json::from_str(&json).expect("list JSON must be valid");
    assert_eq!(parsed, summaries);
    assert_eq!(format_enrolled_json(&[]).trim(), "[]");
}

// ---------------------------------------------------------------- #237 help without root

fn run_enroll(arg: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_soos-enroll"))
        .arg(arg)
        .output()
        .expect("spawn soos-enroll")
}

#[test]
fn test_help_and_version_work_without_root() {
    let help = run_enroll("--help");
    assert!(
        help.status.success(),
        "--help must succeed for any user: {}",
        String::from_utf8_lossy(&help.stderr)
    );
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage"));

    let version = run_enroll("--version");
    assert!(version.status.success(), "--version must succeed");
    assert!(String::from_utf8_lossy(&version.stdout).contains("soos-enroll"));
}

#[test]
fn test_privilege_check_still_applies_to_real_commands() {
    if nix::unistd::geteuid().is_root() {
        return; // The refusal is only observable as a non-root user.
    }
    let out = run_enroll("list");
    assert!(!out.status.success(), "list must still require root");
    assert!(String::from_utf8_lossy(&out.stderr).contains("Root privileges"));
}

#[test]
fn test_invalid_arguments_report_a_usage_error_not_root_error() {
    let out = run_enroll("--definitely-not-a-flag");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("Root privileges"),
        "argument errors must be reported before the privilege check: {stderr}"
    );
}

// ---------------------------------------------------------------- #237 import overwrite

#[test]
fn test_import_yes_flag_parses() {
    let cli = Cli::try_parse_from([
        "soos-enroll",
        "import",
        "--uid",
        "1000",
        "--file",
        "x.json",
        "--yes",
    ])
    .expect("--yes must parse");
    match cli.command {
        Commands::Import(cmd) => {
            assert!(cmd.yes);
            assert_eq!(cmd.args.file, PathBuf::from("x.json"));
        }
        other => panic!("unexpected command {other:?}"),
    }
    let cli = Cli::try_parse_from(["soos-enroll", "import", "--file", "x.json"]).unwrap();
    match cli.command {
        Commands::Import(cmd) => assert!(!cmd.yes, "--yes must default to false"),
        other => panic!("unexpected command {other:?}"),
    }
}

#[test]
fn test_import_onto_enrolled_uid_without_yes_is_refused() {
    let tmp = tempdir().unwrap();
    let (service, store) = store_only(&tmp);
    let first = write_embedding(tmp.path(), "first.json", 0.042);
    let second = write_embedding(tmp.path(), "second.json", 0.5);

    let outcome = service
        .import(&file_args(1000, first))
        .expect("first import");
    assert!(!outcome.replaced_existing);

    match service.import(&file_args(1000, second)) {
        Err(EnrollmentCliError::AlreadyEnrolled(1000)) => {}
        other => panic!("expected AlreadyEnrolled(1000), got {other:?}"),
    }
    let kept = store.get(1000).unwrap().expect("template kept");
    assert!(
        (kept.embedding[0] - 0.042).abs() < 1e-5,
        "the existing template must stay untouched"
    );
}

#[test]
fn test_import_onto_enrolled_uid_with_yes_replaces_and_reports_it() {
    let tmp = tempdir().unwrap();
    let (service, store) = store_only(&tmp);
    let first = write_embedding(tmp.path(), "first.json", 0.042);
    let second = write_embedding(tmp.path(), "second.json", 0.5);

    service
        .import(&file_args(1000, first))
        .expect("first import");
    let outcome = service
        .import_with_overwrite(&file_args(1000, second), true)
        .expect("--yes import must replace");
    assert!(outcome.replaced_existing);
    let replaced = store.get(1000).unwrap().expect("template stored");
    assert!((replaced.embedding[0] - 0.5).abs() < 1e-5);
}

#[test]
fn test_stdin_import_reports_replacement() {
    let tmp = tempdir().unwrap();
    let (service, _store) = store_only(&tmp);
    let json = serde_json::to_string(&vec![0.042f32; 512]).unwrap();
    let args = file_args(1000, PathBuf::from(IMPORT_STDIN_PATH));

    let first = service
        .import_from_reader(&args, std::io::Cursor::new(json.clone()))
        .expect("first stdin import");
    assert!(!first.replaced_existing);
    let second = service
        .import_from_reader(&args, std::io::Cursor::new(json))
        .expect("stdin import keeps its confirmed-by-caller semantics");
    assert!(
        second.replaced_existing,
        "a replacement must be reported to the caller"
    );
}

// ---------------------------------------------------------------- #233 enroll overwrite notice

#[test]
fn test_enroll_summary_announces_replacement_of_existing_template() {
    let tmp = TempDir::new().unwrap();
    let (service, _store) = full_service(&tmp);
    let args = EnrollArgs {
        uid: Some(1002),
        username: None,
        frames: 2,
        yes: false,
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    };

    let mut seen = Vec::new();
    let first = service
        .enroll(&args, |summary| {
            seen.push(summary.already_enrolled);
            true
        })
        .expect("first enrollment");
    assert!(!first.replaced_existing);

    let second = service
        .enroll(&args, |summary| {
            seen.push(summary.already_enrolled);
            true
        })
        .expect("confirmed re-enrollment");
    assert!(second.replaced_existing);
    assert_eq!(seen, vec![false, true]);
}

// ---------------------------------------------------------------- #233 shred symlink safety

#[test]
fn test_shred_refuses_symlink_and_leaves_target_intact() {
    let tmp = tempdir().unwrap();
    let target = tmp.path().join("target.bin");
    std::fs::write(&target, b"must survive").unwrap();
    let link = tmp.path().join("link.bin");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    assert!(
        secure_shred_file(&link).is_err(),
        "a symbolic link must be refused"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"must survive");
    assert!(link.symlink_metadata().is_ok(), "the link is not removed");
}

#[test]
fn test_shred_refuses_dangling_symlink_and_directory() {
    let tmp = tempdir().unwrap();
    let dangling = tmp.path().join("dangling");
    std::os::unix::fs::symlink(tmp.path().join("missing"), &dangling).unwrap();
    assert!(secure_shred_file(&dangling).is_err());
    assert!(dangling.symlink_metadata().is_ok());

    let dir = tmp.path().join("dir");
    std::fs::create_dir(&dir).unwrap();
    assert!(secure_shred_file(&dir).is_err());
    assert!(dir.is_dir());
}

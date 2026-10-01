//! Contractual tests for enrollment model provenance (GitHub #182 / STO-09).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tempfile::TempDir;

use soos_biometric_store::{BiometricStore, MasterKey};
use soos_camera_v4l::{CameraConfig, MockCameraManager};
use soos_enrollment_cli::args::{Cli, Commands};
use soos_enrollment_cli::service::EnrollmentService;
use soos_enrollment_cli::{EnrollArgs, EMBEDDING_MODEL_VERSION, MODEL_ID_EMBEDDING};
use soos_inference_ort::{
    BoundingBox, FaceDetection, FaceLandmarks, MockEmbeddingExtractor, MockFaceDetector,
    MockPadDetector, ModelManifest, Point2f,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

fn mock_service(temp: &TempDir) -> (EnrollmentService, Arc<BiometricStore>) {
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("biometrics"), key).unwrap());
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
        EnrollmentService::new(store.clone(), camera, pipeline, false),
        store,
    )
}

fn parse_enroll(argv: &[&str]) -> EnrollArgs {
    let cli = Cli::try_parse_from(argv).expect("CLI must parse");
    match cli.command {
        Commands::Enroll(args) => args,
        other => panic!("Expected enroll subcommand, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// GitHub #182 / STO-09: recorded model provenance
// ---------------------------------------------------------------------------

#[test]
fn test_embedding_model_constants_match_attested_manifest() {
    let manifest_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    let manifest = ModelManifest::from_file(&manifest_path).expect("manifest must load");
    assert_eq!(MODEL_ID_EMBEDDING, "sface_2021dec");
    assert!(
        manifest.get_model(MODEL_ID_EMBEDDING).is_some(),
        "the recorded embedding model id must be attested by models/manifest.toml"
    );
    assert_eq!(
        EMBEDDING_MODEL_VERSION, manifest.manifest.version,
        "the recorded model version must be the attested manifest version"
    );
}

#[test]
fn test_enroll_cli_defaults_record_loaded_embedding_model() {
    let args = parse_enroll(&["soos-enroll", "enroll", "--uid", "1000"]);
    assert_eq!(args.model_id, MODEL_ID_EMBEDDING);
    assert_eq!(args.model_version, EMBEDDING_MODEL_VERSION);
    assert_ne!(
        args.model_id, "mobilefacenet",
        "retired default must be gone"
    );
}

#[test]
fn test_enroll_default_args_persist_loaded_embedding_model_id() {
    let temp = TempDir::new().unwrap();
    let (service, store) = mock_service(&temp);
    let args = parse_enroll(&["soos-enroll", "enroll", "--uid", "1500", "--frames", "2"]);

    let mut overridden = None;
    let outcome = service
        .enroll(&args, |summary| {
            overridden = Some(summary.model_overridden);
            true
        })
        .expect("enrollment must succeed");

    assert_eq!(overridden, Some(false), "defaults must not be flagged");
    assert_eq!(outcome.model_id, MODEL_ID_EMBEDDING);
    assert_eq!(outcome.model_version, EMBEDDING_MODEL_VERSION);
    let stored = store.get(1500).unwrap().expect("template stored");
    assert_eq!(stored.model_id, MODEL_ID_EMBEDDING);
    assert_eq!(stored.model_version, EMBEDDING_MODEL_VERSION);
}

#[test]
fn test_enroll_explicit_model_override_is_flagged_in_summary() {
    let temp = TempDir::new().unwrap();
    let (service, _store) = mock_service(&temp);
    let args = parse_enroll(&[
        "soos-enroll",
        "enroll",
        "--uid",
        "1501",
        "--frames",
        "1",
        "--model-id",
        "mobilefacenet",
    ]);

    let mut overridden = None;
    let _ = service.enroll(&args, |summary| {
        overridden = Some(summary.model_overridden);
        false
    });
    assert_eq!(
        overridden,
        Some(true),
        "an override differing from the loaded embedding model must be flagged before saving"
    );
}

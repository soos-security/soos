#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    clippy::unreachable,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::sync::Arc;
use tempfile::TempDir;

use soos_biometric_store::{BiometricStore, MasterKey};
use soos_camera_v4l::{CameraConfig, MockCameraManager};
use soos_enrollment_cli::args::EnrollArgs;
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::EnrollmentService;
use soos_inference_ort::{
    BoundingBox, FaceDetection, FaceLandmarks, MockEmbeddingExtractor, MockFaceDetector,
    MockPadDetector, Point2f,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

fn setup_mock_service(temp: &TempDir) -> (EnrollmentService, Arc<BiometricStore>) {
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("biometrics"), key).unwrap());

    // 5-point landmarks
    let landmarks = FaceLandmarks {
        left_eye: Point2f { x: 38.0, y: 52.0 },
        right_eye: Point2f { x: 74.0, y: 52.0 },
        nose: Point2f { x: 56.0, y: 70.0 },
        mouth_left: Point2f { x: 42.0, y: 88.0 },
        mouth_right: Point2f { x: 70.0, y: 88.0 },
    };

    // Single face detection
    let detection = FaceDetection {
        box_: BoundingBox::new(20.0, 20.0, 80.0, 80.0),
        score: 0.95,
        landmarks: Some(landmarks),
    };
    let detector = Arc::new(MockFaceDetector::new_with_detections(vec![detection]));

    // Embedding (dimension 128)
    let extractor = Arc::new(MockEmbeddingExtractor::new(128));
    let pad = Arc::new(MockPadDetector::new_live());

    let pipeline_config = VisionPipelineConfig::default();
    let pipeline = Arc::new(VisionPipeline::new(
        detector,
        pad,
        extractor,
        pipeline_config,
    ));

    let camera_config = CameraConfig {
        warmup_frames: 0,
        ..Default::default()
    };
    let camera = Arc::new(MockCameraManager::new(camera_config));

    let service = EnrollmentService::new(store.clone(), camera, pipeline, false);
    (service, store)
}

#[test]
fn test_enroll_nominal_with_auto_confirm() {
    let temp = TempDir::new().unwrap();
    let (service, store) = setup_mock_service(&temp);

    let args = EnrollArgs {
        uid: Some(1000),
        username: None,
        frames: 3,
        yes: true,
        model_id: "mobilefacenet".to_string(),
        model_version: "1.0.0".to_string(),
    };

    let outcome = service
        .enroll(&args, |_summary| {
            unreachable!("Confirmation should not be called with --yes")
        })
        .expect("Enrollment must succeed");

    assert_eq!(outcome.uid, 1000);
    assert_eq!(outcome.frames_evaluated, 3);
    assert_eq!(outcome.model_id, "mobilefacenet");
    assert_eq!(outcome.model_version, "1.0.0");
    assert_eq!(outcome.embedding_dim, 128);

    // Verify persisted in store
    let enrolled_opt = store.get(1000).expect("Store get must succeed");
    assert!(enrolled_opt.is_some());
    let enrolled = enrolled_opt.unwrap();
    assert_eq!(enrolled.uid, 1000);
    assert_eq!(enrolled.embedding.len(), 128);
}

#[test]
fn test_enroll_interactive_confirmation_rejected() {
    let temp = TempDir::new().unwrap();
    let (service, store) = setup_mock_service(&temp);

    let args = EnrollArgs {
        uid: Some(1001),
        username: None,
        frames: 3,
        yes: false,
        model_id: "mobilefacenet".to_string(),
        model_version: "1.0.0".to_string(),
    };

    let mut prompt_called = false;
    let res = service.enroll(&args, |summary| {
        prompt_called = true;
        assert_eq!(summary.uid, 1001);
        false // User declines confirmation
    });

    assert!(
        prompt_called,
        "Interactive confirmation callback must be called"
    );
    match res {
        Err(EnrollmentCliError::Cancelled) => {}
        other => panic!("Expected Err(Cancelled), got: {other:?}"),
    }

    // Must NOT be enrolled in store
    let enrolled_opt = store.get(1001).unwrap();
    assert!(enrolled_opt.is_none());
}

#[test]
fn test_enroll_already_enrolled_overwrite_rejected() {
    let temp = TempDir::new().unwrap();
    let (service, store) = setup_mock_service(&temp);

    let args = EnrollArgs {
        uid: Some(1002),
        username: None,
        frames: 2,
        yes: true,
        model_id: "mobilefacenet".to_string(),
        model_version: "1.0.0".to_string(),
    };

    // First enrollment succeeds
    service.enroll(&args, |_| true).unwrap();
    assert!(store.exists(1002).unwrap());

    // Second enrollment without --yes, rejecting overwrite
    let args2 = EnrollArgs {
        uid: Some(1002),
        username: None,
        frames: 2,
        yes: false,
        model_id: "mobilefacenet".to_string(),
        model_version: "1.0.0".to_string(),
    };

    let res = service.enroll(&args2, |_| false);
    match res {
        Err(EnrollmentCliError::Cancelled) | Err(EnrollmentCliError::AlreadyEnrolled(1002)) => {}
        other => panic!("Expected Cancelled or AlreadyEnrolled, got: {other:?}"),
    }
}

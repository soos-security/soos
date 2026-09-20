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

use std::sync::Arc;
use tempfile::TempDir;
use zeroize::Zeroizing;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use soos_camera_v4l::{CameraConfig, MockCameraManager};
use soos_enrollment_cli::args::VerifyArgs;
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::EnrollmentService;
use soos_inference_ort::{
    BoundingBox, EmbeddingExtractor, FaceDetection, FaceLandmarks, MockEmbeddingExtractor,
    MockFaceDetector, MockLandmarkDetector, MockPadDetector, Point2f,
};
use soos_protocol::Verdict;
use soos_vision::{VisionPipeline, VisionPipelineConfig};

fn setup_verify_env(
    temp: &TempDir,
    seed_extractor: Arc<dyn EmbeddingExtractor>,
) -> (EnrollmentService, Arc<BiometricStore>) {
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("biometrics"), key).unwrap());

    // Generate enrolled embedding matching standard extractor
    let standard_extractor = MockEmbeddingExtractor::new(128);
    let sample_crop = vec![128u8; 112 * 112 * 3];
    let enrolled_embedding = standard_extractor
        .extract_embedding(&sample_crop, 112, 112)
        .unwrap();

    // Enroll template for UID 2000
    let template = BiometricTemplate::new(
        2000,
        "mobilefacenet".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(enrolled_embedding.as_slice().to_vec()),
    )
    .unwrap();
    store.enroll(&template).unwrap();

    let detection = FaceDetection {
        box_: BoundingBox::new(20.0, 20.0, 80.0, 80.0),
        score: 0.98,
        landmarks: None,
    };
    let detector = Arc::new(MockFaceDetector::new_with_detections(vec![detection]));

    let landmarks = FaceLandmarks {
        left_eye: Point2f { x: 38.0, y: 52.0 },
        right_eye: Point2f { x: 74.0, y: 52.0 },
        nose: Point2f { x: 56.0, y: 70.0 },
        mouth_left: Point2f { x: 42.0, y: 88.0 },
        mouth_right: Point2f { x: 70.0, y: 88.0 },
    };
    let landmark_detector = Arc::new(MockLandmarkDetector::new_with_landmarks(landmarks));

    let pipeline_config = VisionPipelineConfig {
        match_threshold: 0.50,
        ..Default::default()
    };
    let pad = Arc::new(MockPadDetector::new_live());
    let pipeline = Arc::new(VisionPipeline::new(
        detector,
        landmark_detector,
        pad,
        seed_extractor,
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
fn test_verify_unenrolled_uid_fails() {
    let temp = TempDir::new().unwrap();
    let extractor = Arc::new(MockEmbeddingExtractor::new(128));
    let (service, _) = setup_verify_env(&temp, extractor);

    let args = VerifyArgs {
        uid: Some(9999), // not enrolled
        username: None,
    };

    let res = service.verify(&args);
    match res {
        Err(EnrollmentCliError::NotEnrolled(9999)) => {}
        other => panic!("Expected Err(NotEnrolled(9999)), got: {other:?}"),
    }
}

#[test]
fn test_verify_matching_user_reports_allow_and_metrics() {
    let temp = TempDir::new().unwrap();
    // Identical extractor -> produces matching embedding
    let extractor = Arc::new(MockEmbeddingExtractor::new(128));
    let (service, _) = setup_verify_env(&temp, extractor);

    let args = VerifyArgs {
        uid: Some(2000),
        username: None,
    };

    let report = service.verify(&args).expect("Verification must succeed");
    assert_eq!(report.uid, 2000);
    assert_eq!(report.verdict, Verdict::Allow);
    assert!(report.match_score >= 0.50);
    assert_eq!(report.face_count, 1);
    assert_eq!(report.pad_result, "PASSED");
    assert!(report.latency.total_ms >= 0.0);
}

#[test]
fn test_verify_non_matching_user_reports_deny() {
    let temp = TempDir::new().unwrap();
    // Different seed -> produces non-matching embedding
    let extractor = Arc::new(MockEmbeddingExtractor::with_seed(128, 50.0));
    let (service, _) = setup_verify_env(&temp, extractor);

    let args = VerifyArgs {
        uid: Some(2000),
        username: None,
    };

    let report = service
        .verify(&args)
        .expect("Verification diagnostic executes");
    assert_eq!(report.uid, 2000);
    assert_eq!(report.verdict, Verdict::Deny);
    assert!(report.match_score < 0.50);
}

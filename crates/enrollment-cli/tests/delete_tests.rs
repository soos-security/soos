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
use zeroize::Zeroizing;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use soos_camera_v4l::{CameraConfig, MockCameraManager};
use soos_enrollment_cli::args::DeleteArgs;
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::EnrollmentService;
use soos_inference_ort::{
    MockEmbeddingExtractor, MockFaceDetector, MockLandmarkDetector, MockPadDetector,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

fn setup_delete_service(temp: &TempDir) -> (EnrollmentService, Arc<BiometricStore>) {
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("biometrics"), key).unwrap());

    let detector = Arc::new(MockFaceDetector::new_empty());
    let landmarks = Arc::new(MockLandmarkDetector::new_canonical());
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(128));
    let pipeline = Arc::new(VisionPipeline::new(
        detector,
        landmarks,
        pad,
        extractor,
        VisionPipelineConfig::default(),
    ));
    let camera = Arc::new(MockCameraManager::new(CameraConfig::default()));

    let service = EnrollmentService::new(store.clone(), camera, pipeline, false);
    (service, store)
}

#[test]
fn test_delete_existing_template_with_auto_confirm() {
    let temp = TempDir::new().unwrap();
    let (service, store) = setup_delete_service(&temp);

    // Seed enrollment
    let template = BiometricTemplate::new(
        3000,
        "mobilefacenet".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(vec![0.5; 128]),
    )
    .unwrap();
    store.enroll(&template).unwrap();
    assert!(store.exists(3000).unwrap());

    let args = DeleteArgs {
        uid: Some(3000),
        username: None,
        yes: true,
    };

    let deleted = service
        .delete(&args, |_| {
            unreachable!("Prompt must not be called with --yes")
        })
        .expect("Delete must succeed");
    assert!(deleted);
    assert!(
        !store.exists(3000).unwrap(),
        "Template must be removed from store"
    );
}

#[test]
fn test_delete_interactive_prompt_cancelled() {
    let temp = TempDir::new().unwrap();
    let (service, store) = setup_delete_service(&temp);

    let template = BiometricTemplate::new(
        3001,
        "mobilefacenet".to_string(),
        "1.0.0".to_string(),
        1700000000,
        Zeroizing::new(vec![0.5; 128]),
    )
    .unwrap();
    store.enroll(&template).unwrap();

    let args = DeleteArgs {
        uid: Some(3001),
        username: None,
        yes: false,
    };

    let res = service.delete(&args, |uid| {
        assert_eq!(uid, 3001);
        false // User cancels deletion
    });

    match res {
        Err(EnrollmentCliError::Cancelled) => {}
        other => panic!("Expected Err(Cancelled), got: {other:?}"),
    }

    // Template must remain in store
    assert!(store.exists(3001).unwrap());
}

#[test]
fn test_delete_non_existent_uid_fails() {
    let temp = TempDir::new().unwrap();
    let (service, _) = setup_delete_service(&temp);

    let args = DeleteArgs {
        uid: Some(9999),
        username: None,
        yes: true,
    };

    let res = service.delete(&args, |_| true);
    match res {
        Err(EnrollmentCliError::NotEnrolled(9999)) => {}
        other => panic!("Expected Err(NotEnrolled(9999)), got: {other:?}"),
    }
}

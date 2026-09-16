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
use soos_enrollment_cli::args::ListArgs;
use soos_enrollment_cli::service::EnrollmentService;
use soos_inference_ort::{
    MockEmbeddingExtractor, MockFaceDetector, MockLandmarkDetector, MockPadDetector,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

fn setup_list_service(temp: &TempDir) -> (EnrollmentService, Arc<BiometricStore>) {
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
fn test_list_empty_store_returns_empty_vec() {
    let temp = TempDir::new().unwrap();
    let (service, _) = setup_list_service(&temp);

    let list = service
        .list(&ListArgs::default())
        .expect("List should succeed");
    assert!(list.is_empty());
}

#[test]
fn test_list_multiple_enrolled_users_returns_sorted_summaries() {
    let temp = TempDir::new().unwrap();
    let (service, store) = setup_list_service(&temp);

    let t1 = BiometricTemplate::new(
        1005,
        "mobilefacenet".to_string(),
        "1.0.0".to_string(),
        1700001000,
        Zeroizing::new(vec![0.1; 128]),
    )
    .unwrap();
    let t2 = BiometricTemplate::new(
        1001,
        "mobilefacenet".to_string(),
        "1.0.0".to_string(),
        1700002000,
        Zeroizing::new(vec![0.2; 128]),
    )
    .unwrap();

    store.enroll(&t1).unwrap();
    store.enroll(&t2).unwrap();

    let list = service
        .list(&ListArgs::default())
        .expect("List should succeed");
    assert_eq!(list.len(), 2);
    // Should be sorted by UID
    assert_eq!(list[0].uid, 1001);
    assert_eq!(list[0].model_id, "mobilefacenet");
    assert_eq!(list[0].model_version, "1.0.0");
    assert_eq!(list[0].embedding_dim, 128);

    assert_eq!(list[1].uid, 1005);
    assert_eq!(list[1].embedding_dim, 128);
}

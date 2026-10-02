//! Contract tests for GitHub #312 (review finding STO-NEW-2): `soos-enroll enroll` without
//! `--yes` on a user that was not enrolled when the capture started never replaces a
//! template that another `enroll`, `import` or GUI save created in the meantime (while the
//! frames were captured or the confirmation prompt was open). The final write goes through
//! `BiometricStore::enroll_if_absent` and fails with `EnrollmentCliError::AlreadyEnrolled`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::sync::Arc;
use tempfile::TempDir;
use zeroize::Zeroizing;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use soos_camera_v4l::{CameraConfig, MockCameraManager};
use soos_enrollment_cli::args::EnrollArgs;
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::EnrollmentService;
use soos_inference_ort::{
    BoundingBox, FaceDetection, FaceLandmarks, MockEmbeddingExtractor, MockFaceDetector,
    MockPadDetector, Point2f,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

const UID: u32 = 1312;

fn setup(temp: &TempDir) -> (EnrollmentService, Arc<BiometricStore>) {
    let store = Arc::new(
        BiometricStore::new(
            temp.path().join("biometrics"),
            MasterKey::generate().unwrap(),
        )
        .unwrap(),
    );
    let detection = FaceDetection {
        box_: BoundingBox::new(20.0, 20.0, 80.0, 80.0),
        score: 0.95,
        landmarks: Some(FaceLandmarks {
            left_eye: Point2f { x: 38.0, y: 52.0 },
            right_eye: Point2f { x: 74.0, y: 52.0 },
            nose: Point2f { x: 56.0, y: 70.0 },
            mouth_left: Point2f { x: 42.0, y: 88.0 },
            mouth_right: Point2f { x: 70.0, y: 88.0 },
        }),
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

fn args(yes: bool) -> EnrollArgs {
    EnrollArgs {
        uid: Some(UID),
        username: None,
        frames: 2,
        yes,
        model_id: "mobilefacenet".to_string(),
        model_version: "1.0.0".to_string(),
    }
}

/// A template another tool writes while the confirmation prompt is open.
fn concurrent_template() -> BiometricTemplate {
    BiometricTemplate::new(
        UID,
        "concurrent-model".into(),
        "9.9.9".into(),
        42,
        Zeroizing::new(vec![0.25_f32; 512]),
    )
    .unwrap()
}

#[test]
fn test_312_enroll_without_yes_never_replaces_a_template_created_meanwhile() {
    let temp = TempDir::new().unwrap();
    let (service, store) = setup(&temp);

    let res = service.enroll(&args(false), |summary| {
        assert!(
            !summary.already_enrolled,
            "not enrolled when capture started"
        );
        store
            .enroll(&concurrent_template())
            .expect("concurrent enrollment");
        true
    });

    match res {
        Err(EnrollmentCliError::AlreadyEnrolled(uid)) => assert_eq!(uid, UID),
        other => panic!("expected AlreadyEnrolled, got {other:?}"),
    }
    let kept = store.get(UID).unwrap().expect("concurrent template kept");
    assert_eq!(kept.model_id, "concurrent-model");
    assert_eq!(kept.model_version, "9.9.9");
}

#[test]
fn test_312_enroll_without_yes_on_a_fresh_user_still_enrolls() {
    let temp = TempDir::new().unwrap();
    let (service, store) = setup(&temp);
    let outcome = service
        .enroll(&args(false), |_| true)
        .expect("fresh enrollment");
    assert!(!outcome.replaced_existing);
    assert_eq!(store.get(UID).unwrap().unwrap().model_id, "mobilefacenet");
}

#[test]
fn test_312_confirmed_replacement_of_a_known_enrollment_still_replaces() {
    let temp = TempDir::new().unwrap();
    let (service, store) = setup(&temp);
    store.enroll(&concurrent_template()).unwrap();
    let outcome = service
        .enroll(&args(false), |summary| {
            assert!(summary.already_enrolled);
            true
        })
        .expect("confirmed replacement");
    assert!(outcome.replaced_existing);
    assert_eq!(store.get(UID).unwrap().unwrap().model_id, "mobilefacenet");
}

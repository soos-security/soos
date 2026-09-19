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

use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::check_privileges;

#[test]
fn test_root_check_bypassed_when_flag_disabled() {
    // When require_root is false, check_privileges must always succeed regardless of actual EUID
    let res = check_privileges(false);
    assert!(res.is_ok());
}

#[test]
fn test_root_check_enforced_against_euid() {
    let current_euid = nix::unistd::geteuid().as_raw();
    let res = check_privileges(true);

    if current_euid == 0 {
        assert!(res.is_ok());
    } else {
        match res {
            Err(EnrollmentCliError::RootRequired) => {}
            other => panic!("Expected Err(EnrollmentCliError::RootRequired), got: {other:?}"),
        }
    }
}

#[test]
fn test_root_required_error_message() {
    let err = EnrollmentCliError::RootRequired;
    let msg = err.to_string();
    assert!(msg.to_lowercase().contains("root"));
}

#[test]
fn test_verify_subcommand_enforces_root_privileges() {
    use soos_biometric_store::{BiometricStore, MasterKey};
    use soos_camera_v4l::{CameraConfig, MockCameraManager};
    use soos_enrollment_cli::args::VerifyArgs;
    use soos_enrollment_cli::service::EnrollmentService;
    use soos_inference_ort::{
        MockEmbeddingExtractor, MockFaceDetector, MockLandmarkDetector, MockPadDetector,
    };
    use soos_vision::{VisionPipeline, VisionPipelineConfig};
    use std::sync::Arc;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
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

    // Instantiate service with require_root = true
    let service = EnrollmentService::new(store, camera, pipeline, true);
    let args = VerifyArgs {
        uid: Some(1000),
        username: None,
    };

    let res = service.verify(&args);
    let current_euid = nix::unistd::geteuid().as_raw();
    if current_euid != 0 {
        match res {
            Err(EnrollmentCliError::RootRequired) => {}
            other => {
                panic!("Expected Err(EnrollmentCliError::RootRequired) on verify, got: {other:?}")
            }
        }
    }
}

#[test]
fn test_list_subcommand_enforces_root_privileges() {
    use soos_biometric_store::{BiometricStore, MasterKey};
    use soos_enrollment_cli::args::ListArgs;
    use soos_enrollment_cli::service::EnrollmentService;
    use std::sync::Arc;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("biometrics"), key).unwrap());

    // Instantiate store-only service with require_root = true
    let service = EnrollmentService::new_store_only(store, true);
    let args = ListArgs::default();

    let res = service.list(&args);
    let current_euid = nix::unistd::geteuid().as_raw();
    if current_euid != 0 {
        match res {
            Err(EnrollmentCliError::RootRequired) => {}
            other => {
                panic!("Expected Err(EnrollmentCliError::RootRequired) on list, got: {other:?}")
            }
        }
    }
}

#[test]
fn test_delete_subcommand_enforces_root_privileges() {
    use soos_biometric_store::{BiometricStore, MasterKey};
    use soos_enrollment_cli::args::DeleteArgs;
    use soos_enrollment_cli::service::EnrollmentService;
    use std::sync::Arc;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("biometrics"), key).unwrap());

    let service = EnrollmentService::new_store_only(store, true);
    let args = DeleteArgs {
        uid: Some(1000),
        username: None,
        yes: true,
    };

    let res = service.delete(&args, |_| true);
    let current_euid = nix::unistd::geteuid().as_raw();
    if current_euid != 0 {
        match res {
            Err(EnrollmentCliError::RootRequired) => {}
            other => {
                panic!("Expected Err(EnrollmentCliError::RootRequired) on delete, got: {other:?}")
            }
        }
    }
}

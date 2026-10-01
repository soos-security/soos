//! GitHub #285 (row RFX9): the pre-PAD quality gate (GitHub #218) in `soos-enroll`.
//!
//! A face rejected as too small (`VisionError::FaceTooSmall`) or too blurred
//! (`VisionError::FaceBlurred`) is an unusable capture:
//! - `enroll` counts it as an invalid candidate and keeps evaluating the remaining frames
//!   instead of aborting the whole enrollment;
//! - `verify` returns a `Deny` diagnostic report naming the gate instead of a generic error.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tempfile::TempDir;
use zeroize::Zeroizing;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use soos_camera_v4l::{CameraConfig, MockCameraManager};
use soos_enrollment_cli::args::{EnrollArgs, VerifyArgs};
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::EnrollmentService;
use soos_inference_ort::{
    BoundingBox, EmbeddingExtractor, FaceDetection, FaceDetector, FaceLandmarks, InferenceError,
    MockEmbeddingExtractor, MockPadDetector, Point2f,
};
use soos_protocol::Verdict;
use soos_vision::{VisionError, VisionPipeline, VisionPipelineConfig};

const UID: u32 = 2100;

fn landmarks() -> FaceLandmarks {
    FaceLandmarks {
        left_eye: Point2f { x: 38.0, y: 52.0 },
        right_eye: Point2f { x: 74.0, y: 52.0 },
        nose: Point2f { x: 56.0, y: 70.0 },
        mouth_left: Point2f { x: 42.0, y: 88.0 },
        mouth_right: Point2f { x: 70.0, y: 88.0 },
    }
}

/// A 60 px face (passes the default 48 px gate).
fn normal_face() -> FaceDetection {
    FaceDetection {
        box_: BoundingBox::new(20.0, 20.0, 80.0, 80.0),
        score: 0.97,
        landmarks: Some(landmarks()),
    }
}

/// A 10 px face (rejected by the default 48 px gate).
fn tiny_face() -> FaceDetection {
    FaceDetection {
        box_: BoundingBox::new(20.0, 20.0, 30.0, 30.0),
        score: 0.97,
        landmarks: Some(landmarks()),
    }
}

/// Detector returning a tiny face for the first `tiny_calls` calls, then a normal face.
struct ScriptedDetector {
    calls: AtomicUsize,
    tiny_calls: usize,
}

impl FaceDetector for ScriptedDetector {
    fn detect(
        &self,
        _rgb: &[u8],
        _width: u32,
        _height: u32,
    ) -> Result<Vec<FaceDetection>, InferenceError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![if call < self.tiny_calls {
            tiny_face()
        } else {
            normal_face()
        }])
    }
}

fn service(
    temp: &TempDir,
    tiny_calls: usize,
    config: VisionPipelineConfig,
    enroll_reference: bool,
) -> (EnrollmentService, Arc<BiometricStore>) {
    let store = Arc::new(
        BiometricStore::new(
            temp.path().join("biometrics"),
            MasterKey::generate().unwrap(),
        )
        .unwrap(),
    );
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    if enroll_reference {
        let reference = extractor
            .extract_embedding(&vec![128u8; 112 * 112 * 3], 112, 112)
            .unwrap();
        let template = BiometricTemplate::new(
            UID,
            soos_enrollment_cli::service::MODEL_ID_EMBEDDING.to_string(),
            "1.0.0".to_string(),
            1_700_000_000,
            Zeroizing::new(reference.as_slice().to_vec()),
        )
        .unwrap();
        store.enroll(&template).unwrap();
    }
    let detector = Arc::new(ScriptedDetector {
        calls: AtomicUsize::new(0),
        tiny_calls,
    });
    let pipeline = Arc::new(VisionPipeline::new(
        detector,
        Arc::new(MockPadDetector::new_live()),
        extractor,
        config,
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

fn enroll_args(frames: usize) -> EnrollArgs {
    EnrollArgs {
        uid: Some(UID),
        username: None,
        frames,
        yes: true,
        model_id: "mobilefacenet".to_string(),
        model_version: "1.0.0".to_string(),
    }
}

fn verify_args() -> VerifyArgs {
    VerifyArgs {
        uid: Some(UID),
        username: None,
    }
}

/// RFX9: a too-small face on the first frame no longer aborts enrollment; the later frames
/// are evaluated and the enrollment succeeds.
#[test]
fn test_enroll_face_too_small_frame_is_an_invalid_candidate() {
    let temp = TempDir::new().unwrap();
    let (service, store) = service(&temp, 1, VisionPipelineConfig::default(), false);

    let outcome = service
        .enroll(&enroll_args(3), |_| true)
        .expect("a too-small face on one frame must not abort the enrollment");
    assert_eq!(outcome.uid, UID);
    assert_eq!(outcome.frames_evaluated, 3);
    assert!(store.exists(UID).unwrap());
}

/// RFX9: every frame too small ends in the quality verdict, never a raw vision error, and
/// nothing is stored.
#[test]
fn test_enroll_all_faces_too_small_reports_low_quality() {
    let temp = TempDir::new().unwrap();
    let (service, store) = service(&temp, usize::MAX, VisionPipelineConfig::default(), false);

    let err = service
        .enroll(&enroll_args(3), |_| true)
        .expect_err("no usable frame must fail closed");
    assert!(
        matches!(err, EnrollmentCliError::LowQualityFrames { .. }),
        "expected LowQualityFrames, got {err:?}"
    );
    assert!(!store.exists(UID).unwrap());
}

/// RFX9: a blurred crop on every frame also ends in the quality verdict.
#[test]
fn test_enroll_all_faces_blurred_reports_low_quality() {
    let temp = TempDir::new().unwrap();
    let config = VisionPipelineConfig {
        min_pad_crop_sharpness: f32::MAX,
        ..Default::default()
    };
    let (service, store) = service(&temp, 0, config, false);

    let err = service
        .enroll(&enroll_args(2), |_| true)
        .expect_err("no usable frame must fail closed");
    assert!(
        !matches!(
            err,
            EnrollmentCliError::Vision(VisionError::FaceBlurred { .. })
        ),
        "a blurred crop must not abort with a raw vision error: {err:?}"
    );
    assert!(matches!(err, EnrollmentCliError::LowQualityFrames { .. }));
    assert!(!store.exists(UID).unwrap());
}

/// RFX9: `verify` reports a too-small face as a Deny diagnostic.
#[test]
fn test_verify_face_too_small_reports_deny() {
    let temp = TempDir::new().unwrap();
    let (service, _) = service(&temp, usize::MAX, VisionPipelineConfig::default(), true);

    let report = service
        .verify(&verify_args())
        .expect("a too-small face must produce a diagnostic report, not an error");
    assert_eq!(report.verdict, Verdict::Deny);
    assert_eq!(report.face_count, 1);
    assert_eq!(report.match_score, 0.0);
    assert!(
        report.pad_result.starts_with("FACE_TOO_SMALL("),
        "unexpected pad_result {}",
        report.pad_result
    );
    assert_eq!(report.pad_score, None);
}

/// RFX9: `verify` reports a blurred crop as a Deny diagnostic.
#[test]
fn test_verify_face_blurred_reports_deny() {
    let temp = TempDir::new().unwrap();
    let config = VisionPipelineConfig {
        min_pad_crop_sharpness: f32::MAX,
        ..Default::default()
    };
    let (service, _) = service(&temp, 0, config, true);

    let report = service
        .verify(&verify_args())
        .expect("a blurred crop must produce a diagnostic report, not an error");
    assert_eq!(report.verdict, Verdict::Deny);
    assert_eq!(report.face_count, 1);
    assert!(
        report.pad_result.starts_with("FACE_BLURRED("),
        "unexpected pad_result {}",
        report.pad_result
    );
    assert_eq!(report.pad_score, None);
}

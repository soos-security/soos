//! Contract tests for the PAD status reported by `soos-enroll verify`
//! (GitHub #216 / PAD-11 and GitHub #236 / STO-20).
//!
//! A presentation attack must produce a `Deny` diagnostic report carrying the PAD score
//! and threshold (never a generic error), and a live presentation must report the real
//! PAD score instead of a bare `PASSED` literal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use std::sync::Arc;
use tempfile::TempDir;
use zeroize::Zeroizing;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use soos_camera_v4l::{CameraConfig, CameraManager, Frame, MockCameraManager, PixelFormat};
use soos_enrollment_cli::args::VerifyArgs;
use soos_enrollment_cli::service::EnrollmentService;
use soos_inference_ort::{
    AttackType, BoundingBox, EmbeddingExtractor, FaceDetection, FaceLandmarks,
    MockEmbeddingExtractor, MockFaceDetector, MockPadDetector, PadDetector, Point2f,
};
use soos_protocol::Verdict;
use soos_vision::{VisionPipeline, VisionPipelineConfig};

const PAD_THRESHOLD: f32 = 0.85;

/// Camera double serving one fixed frame (used to inject an underexposed IR frame).
struct FixedFrameCamera(Arc<Frame>);

impl CameraManager for FixedFrameCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        Some(Arc::clone(&self.0))
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn notify_activity(&self) {}
    fn stop(&self) {}
}

fn setup(temp: &TempDir, pad: Arc<dyn PadDetector>, format: PixelFormat) -> EnrollmentService {
    let camera = Arc::new(MockCameraManager::new(CameraConfig {
        warmup_frames: 0,
        format,
        ..Default::default()
    }));
    setup_with_camera(temp, pad, camera)
}

fn setup_with_camera(
    temp: &TempDir,
    pad: Arc<dyn PadDetector>,
    camera: Arc<dyn CameraManager>,
) -> EnrollmentService {
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("biometrics"), key).unwrap());

    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let sample_crop = vec![128u8; 112 * 112 * 3];
    let enrolled = extractor.extract_embedding(&sample_crop, 112, 112).unwrap();
    let template = BiometricTemplate::new(
        2000,
        soos_enrollment_cli::service::MODEL_ID_EMBEDDING.to_string(),
        "1.0.0".to_string(),
        1_700_000_000,
        Zeroizing::new(enrolled.as_slice().to_vec()),
    )
    .unwrap();
    store.enroll(&template).unwrap();

    let detection = FaceDetection {
        box_: BoundingBox::new(20.0, 20.0, 80.0, 80.0),
        score: 0.98,
        landmarks: Some(FaceLandmarks {
            left_eye: Point2f { x: 38.0, y: 52.0 },
            right_eye: Point2f { x: 74.0, y: 52.0 },
            nose: Point2f { x: 56.0, y: 70.0 },
            mouth_left: Point2f { x: 42.0, y: 88.0 },
            mouth_right: Point2f { x: 70.0, y: 88.0 },
        }),
    };
    let detector = Arc::new(MockFaceDetector::new_with_detections(vec![detection]));
    let pipeline = Arc::new(VisionPipeline::new(
        detector,
        pad,
        extractor,
        VisionPipelineConfig {
            match_threshold: 0.50,
            pad_threshold: PAD_THRESHOLD,
            ..Default::default()
        },
    ));
    EnrollmentService::new(store, camera, pipeline, false)
}

fn args() -> VerifyArgs {
    VerifyArgs {
        uid: Some(2000),
        username: None,
    }
}

/// PAD-11: a spoof is a `Deny` report carrying the PAD score, not a generic error.
#[test]
fn test_verify_spoof_reports_deny_with_pad_score() {
    let temp = TempDir::new().unwrap();
    let pad = Arc::new(MockPadDetector::new_spoof(AttackType::PrintPhoto, 0.12));
    let service = setup(&temp, pad, PixelFormat::Rgb24);

    let report = service
        .verify(&args())
        .expect("a spoof presentation must produce a diagnostic report, not an error");
    assert_eq!(report.uid, 2000);
    assert_eq!(report.verdict, Verdict::Deny);
    assert_eq!(report.face_count, 1);
    assert_eq!(report.match_score, 0.0, "no match is computed for a spoof");
    assert_eq!(report.pad_result, "SPOOF(score=0.120, threshold=0.85)");
    let score = report.pad_score.expect("PAD score must be reported");
    assert!((score - 0.12).abs() < 1e-6);
    let threshold = report
        .pad_threshold
        .expect("PAD threshold must be reported");
    assert!((threshold - PAD_THRESHOLD).abs() < 1e-6);
    assert!(report.latency.total_ms >= 0.0);
}

/// PAD-11: a live-but-below-threshold score is also a spoof verdict with its score.
#[test]
fn test_verify_low_live_score_reports_deny_with_pad_score() {
    let temp = TempDir::new().unwrap();
    let pad = Arc::new(MockPadDetector::new_with_result(
        soos_inference_ort::PadResult::live(0.40),
    ));
    let service = setup(&temp, pad, PixelFormat::Rgb24);

    let report = service.verify(&args()).expect("diagnostic report expected");
    assert_eq!(report.verdict, Verdict::Deny);
    assert!(report.pad_result.starts_with("SPOOF(score=0.400"));
    assert!((report.pad_score.unwrap() - 0.40).abs() < 1e-6);
}

/// STO-20: a live presentation reports the real PAD score and threshold.
#[test]
fn test_verify_live_reports_real_pad_score() {
    let temp = TempDir::new().unwrap();
    let pad = Arc::new(MockPadDetector::new_with_result(
        soos_inference_ort::PadResult::live(0.93),
    ));
    let service = setup(&temp, pad, PixelFormat::Rgb24);

    let report = service.verify(&args()).expect("verification must succeed");
    assert_eq!(report.verdict, Verdict::Allow);
    assert_eq!(report.pad_result, "PASSED");
    let score = report.pad_score.expect("PAD score must be reported");
    assert!((score - 0.93).abs() < 1e-6, "mock PAD score must be echoed");
    let threshold = report
        .pad_threshold
        .expect("PAD threshold must be reported");
    assert!((threshold - PAD_THRESHOLD).abs() < 1e-6);
    assert_eq!(
        report.pad_status(),
        "PASSED (score=0.930, threshold=0.85)",
        "the operator-facing PAD line includes the score"
    );
}

/// STO-20: face-level failures report no PAD score (PAD never ran).
#[test]
fn test_verify_no_face_reports_no_pad_score() {
    let temp = TempDir::new().unwrap();
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("biometrics"), key).unwrap());
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let enrolled = extractor
        .extract_embedding(&vec![128u8; 112 * 112 * 3], 112, 112)
        .unwrap();
    store
        .enroll(
            &BiometricTemplate::new(
                2000,
                soos_enrollment_cli::service::MODEL_ID_EMBEDDING.to_string(),
                "1.0.0".to_string(),
                1_700_000_000,
                Zeroizing::new(enrolled.as_slice().to_vec()),
            )
            .unwrap(),
        )
        .unwrap();
    let pipeline = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_with_detections(Vec::new())),
        Arc::new(MockPadDetector::new_live()),
        extractor,
        VisionPipelineConfig::default(),
    ));
    let camera = Arc::new(MockCameraManager::new(CameraConfig {
        warmup_frames: 0,
        ..Default::default()
    }));
    let service = EnrollmentService::new(store, camera, pipeline, false);

    let report = service.verify(&args()).expect("diagnostic report expected");
    assert_eq!(report.verdict, Verdict::Deny);
    assert_eq!(report.pad_result, "NO_FACE");
    assert_eq!(report.pad_score, None);
    assert_eq!(report.pad_status(), "NO_FACE");
}

/// PAD-11: a monochrome frame scored below the IR threshold is a `Deny` report carrying
/// the effective (IR) threshold, not a generic error.
#[test]
fn test_verify_monochrome_spoof_reports_ir_threshold() {
    let temp = TempDir::new().unwrap();
    let pad = Arc::new(MockPadDetector::new_spoof(AttackType::ScreenReplay, 0.05));
    let service = setup(&temp, pad, PixelFormat::Grey);

    let report = service
        .verify(&args())
        .expect("an IR PAD rejection must produce a diagnostic report, not an error");
    assert_eq!(report.verdict, Verdict::Deny);
    assert!(
        report.pad_result.starts_with("SPOOF(score=0.050"),
        "unexpected PAD status: {}",
        report.pad_result
    );
    let threshold = report.pad_threshold.expect("IR threshold must be reported");
    assert!(
        threshold >= PAD_THRESHOLD,
        "the IR threshold is never below the colour threshold"
    );
}

/// PAD-11: a monochrome crop rejected by the fail-closed IR liveness gate is a `Deny`
/// report naming the gate, with no PAD score (the PAD model never ran).
#[test]
fn test_verify_ir_gate_rejection_reports_deny() {
    let temp = TempDir::new().unwrap();
    let pad = Arc::new(MockPadDetector::new_live());
    let (width, height) = (640_u32, 480_u32);
    let black = Frame::new(
        vec![0_u8; (width * height) as usize],
        width,
        height,
        0,
        PixelFormat::Grey,
        1,
    );
    let service = setup_with_camera(&temp, pad, Arc::new(FixedFrameCamera(Arc::new(black))));

    let report = service
        .verify(&args())
        .expect("an IR gate rejection must produce a diagnostic report, not an error");
    assert_eq!(report.verdict, Verdict::Deny);
    assert!(
        report.pad_result.starts_with("IR_GATE_REJECTED("),
        "unexpected PAD status: {}",
        report.pad_result
    );
    assert_eq!(report.pad_score, None);
    assert!(report.pad_threshold.is_some());
}

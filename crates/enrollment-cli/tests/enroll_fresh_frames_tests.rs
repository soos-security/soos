//! GitHub #228 (STO-05): multi-frame enrollment evaluates distinct camera frames and counts
//! presentation-attack rejections as invalid candidates instead of aborting.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::TempDir;

use soos_biometric_store::{BiometricStore, MasterKey};
use soos_camera_v4l::{CameraManager, Frame, PixelFormat};
use soos_enrollment_cli::args::EnrollArgs;
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::{
    EnrollmentService, EnrollmentSummary, ENROLL_FRESH_FRAME_TIMEOUT_MS,
};
use soos_inference_ort::{
    AttackType, BoundingBox, FaceDetection, FaceDetector, FaceLandmarks, InferenceError,
    MockEmbeddingExtractor, MockFaceDetector, MockPadDetector, PadResult, Point2f,
};
use soos_vision::{VisionError, VisionPipeline, VisionPipelineConfig};

const WIDTH: u32 = 128;
const HEIGHT: u32 = 128;
const SWAP_INTERVAL_MS: u64 = 50;

/// Camera that publishes a new frame (sequence + 1) only every `SWAP_INTERVAL_MS`; between
/// swaps `latest_frame` keeps returning the same cached frame, like `V4lCameraManager`.
/// Every pixel byte of a frame equals its sequence number (mod 256), so the detector can tell
/// which frame it was given. `frozen` pins the sequence forever (stalled capture thread).
struct SlowSwapCamera {
    start: Instant,
    frozen: bool,
}

impl SlowSwapCamera {
    fn new(frozen: bool) -> Self {
        Self {
            start: Instant::now(),
            frozen,
        }
    }

    fn current_sequence(&self) -> u64 {
        if self.frozen {
            7
        } else {
            self.start.elapsed().as_millis() as u64 / SWAP_INTERVAL_MS
        }
    }
}

impl CameraManager for SlowSwapCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        let seq = self.current_sequence();
        let data = vec![(seq % 256) as u8; (WIDTH * HEIGHT * 3) as usize];
        Some(Arc::new(Frame::new(
            data,
            WIDTH,
            HEIGHT,
            seq.saturating_mul(SWAP_INTERVAL_MS * 1_000_000),
            PixelFormat::Rgb24,
            seq,
        )))
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn notify_activity(&self) {}

    fn stop(&self) {}
}

/// Face detector recording the first RGB byte (the frame sequence) of every evaluated frame.
struct RecordingDetector {
    inner: MockFaceDetector,
    seen: Mutex<Vec<u8>>,
}

impl FaceDetector for RecordingDetector {
    fn detect(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<FaceDetection>, InferenceError> {
        if let Some(first) = rgb.first() {
            self.seen.lock().unwrap().push(*first);
        }
        self.inner.detect(rgb, width, height)
    }
}

fn single_face() -> FaceDetection {
    FaceDetection {
        box_: BoundingBox::new(20.0, 20.0, 80.0, 80.0),
        score: 0.95,
        landmarks: Some(FaceLandmarks {
            left_eye: Point2f { x: 38.0, y: 52.0 },
            right_eye: Point2f { x: 74.0, y: 52.0 },
            nose: Point2f { x: 56.0, y: 70.0 },
            mouth_left: Point2f { x: 42.0, y: 88.0 },
            mouth_right: Point2f { x: 70.0, y: 88.0 },
        }),
    }
}

struct Fixture {
    _temp: TempDir,
    service: EnrollmentService,
    store: Arc<BiometricStore>,
    detector: Arc<RecordingDetector>,
}

fn fixture(camera: SlowSwapCamera, pad: Arc<MockPadDetector>) -> Fixture {
    let temp = TempDir::new().unwrap();
    let store = Arc::new(
        BiometricStore::new(
            temp.path().join("biometrics"),
            MasterKey::generate().unwrap(),
        )
        .unwrap(),
    );
    let detector = Arc::new(RecordingDetector {
        inner: MockFaceDetector::new_with_detections(vec![single_face()]),
        seen: Mutex::new(Vec::new()),
    });
    let pipeline = Arc::new(VisionPipeline::new(
        Arc::clone(&detector) as Arc<dyn FaceDetector>,
        pad,
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig::default(),
    ));
    let service = EnrollmentService::new(Arc::clone(&store), Arc::new(camera), pipeline, false);
    Fixture {
        _temp: temp,
        service,
        store,
        detector,
    }
}

fn args(uid: u32, frames: usize, yes: bool) -> EnrollArgs {
    EnrollArgs {
        uid: Some(uid),
        username: None,
        frames,
        yes,
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    }
}

#[test]
fn test_enroll_candidates_carry_distinct_frame_sequences() {
    let fx = fixture(
        SlowSwapCamera::new(false),
        Arc::new(MockPadDetector::new_live()),
    );

    let outcome = fx
        .service
        .enroll(&args(1000, 4, true), |_| true)
        .expect("enrollment over 4 fresh frames must succeed");
    assert_eq!(outcome.frames_evaluated, 4);

    let seen = fx.detector.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 4, "exactly one detection per candidate");
    let distinct: BTreeSet<u8> = seen.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        4,
        "every candidate must be a distinct camera frame, saw sequences {seen:?}"
    );
    for pair in seen.windows(2) {
        assert!(
            pair[1] > pair[0],
            "sequences must strictly increase: {seen:?}"
        );
    }
}

#[test]
fn test_enroll_counts_pad_rejection_as_invalid_candidate() {
    let pad = Arc::new(MockPadDetector::new_live());
    pad.set_result_sequence(vec![
        PadResult::spoof(0.05, AttackType::PrintPhoto),
        PadResult::live(0.98),
        PadResult::live(0.98),
    ]);
    let fx = fixture(SlowSwapCamera::new(false), pad);

    let mut summary: Option<EnrollmentSummary> = None;
    let outcome = fx
        .service
        .enroll(&args(1001, 3, false), |s| {
            summary = Some(s.clone());
            true
        })
        .expect("a transient PAD rejection must not abort enrollment");

    let summary = summary.expect("confirmation prompt must be shown");
    assert_eq!(summary.frames_evaluated, 3);
    assert_eq!(summary.valid_candidates, 2);
    assert_eq!(summary.pad_rejections, 1);
    assert_eq!(outcome.frames_evaluated, 3);
    assert!(fx.store.get(1001).unwrap().is_some());
}

#[test]
fn test_enroll_all_pad_rejections_fails_closed_without_storing() {
    let pad = Arc::new(MockPadDetector::new_spoof(AttackType::ScreenReplay, 0.05));
    let fx = fixture(SlowSwapCamera::new(false), pad);

    let res = fx.service.enroll(&args(1002, 3, true), |_| true);
    assert!(
        matches!(
            res,
            Err(EnrollmentCliError::Vision(VisionError::PadFailed { .. }))
        ),
        "all-spoof enrollment must fail with the PAD error, got {res:?}"
    );
    assert_eq!(fx.detector.seen.lock().unwrap().len(), 3);
    assert!(fx.store.get(1002).unwrap().is_none());
}

#[test]
fn test_enroll_frozen_camera_fails_with_bounded_wait() {
    let fx = fixture(
        SlowSwapCamera::new(true),
        Arc::new(MockPadDetector::new_live()),
    );

    let start = Instant::now();
    let res = fx.service.enroll(&args(1003, 3, true), |_| true);
    let elapsed = start.elapsed();

    assert!(
        matches!(
            res,
            Err(EnrollmentCliError::Camera(
                soos_camera_v4l::CameraError::Starved
            ))
        ),
        "a camera that never publishes a new frame must fail enrollment, got {res:?}"
    );
    assert_eq!(
        fx.detector.seen.lock().unwrap().len(),
        1,
        "the stale frame must never be re-evaluated"
    );
    assert!(fx.store.get(1003).unwrap().is_none());
    // Bounded: one fresh-frame wait, never the whole-run capture budget per frame.
    assert!(
        elapsed < Duration::from_millis(ENROLL_FRESH_FRAME_TIMEOUT_MS * 4),
        "stale-frame wait must be bounded, took {elapsed:?}"
    );
}

//! GitHub #337 (ECB): when no frame arrives in time and the camera status reports
//! `Error { kind: DeviceBusy, .. }` (normally `soos-daemon` holds the device), `soos-enroll`
//! returns the dedicated `EnrollmentCliError::CameraBusy` with operator guidance instead of the
//! generic `Camera(CameraError::Starved)`. Every other timeout keeps `Starved`, and a frame that
//! arrives in time is used whatever the status says.

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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use zeroize::Zeroizing;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use soos_camera_v4l::{
    CameraConfig, CameraError, CameraErrorKind, CameraManager, CameraStatus, Frame,
    MockCameraManager, PixelFormat,
};
use soos_enrollment_cli::args::{EnrollArgs, VerifyArgs};
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::{EnrollmentService, MODEL_ID_EMBEDDING};
use soos_inference_ort::{
    BoundingBox, EmbeddingExtractor, FaceDetection, FaceLandmarks, MockEmbeddingExtractor,
    MockFaceDetector, MockPadDetector, Point2f,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

const WIDTH: u32 = 128;
const HEIGHT: u32 = 128;

/// Camera stub with a fixed `status()`. It publishes `frames_before_silence` distinct frames
/// (one new sequence per `latest_frame` call) and then returns `None` forever; `u64::MAX`
/// means it never goes silent.
struct StatusStubCamera {
    status: CameraStatus,
    frames_before_silence: u64,
    calls: AtomicU64,
}

impl StatusStubCamera {
    fn silent(status: CameraStatus) -> Self {
        Self::with_frames(status, 0)
    }

    fn with_frames(status: CameraStatus, frames_before_silence: u64) -> Self {
        Self {
            status,
            frames_before_silence,
            calls: AtomicU64::new(0),
        }
    }
}

impl CameraManager for StatusStubCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        let seq = self.calls.fetch_add(1, Ordering::SeqCst);
        if seq >= self.frames_before_silence {
            return None;
        }
        Some(Arc::new(Frame::new(
            vec![128u8; (WIDTH * HEIGHT * 3) as usize],
            WIDTH,
            HEIGHT,
            seq.saturating_add(1).saturating_mul(33_000_000),
            PixelFormat::Rgb24,
            seq.saturating_add(1),
        )))
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn notify_activity(&self) {}

    fn stop(&self) {}

    fn status(&self) -> CameraStatus {
        self.status
    }
}

fn single_face() -> FaceDetection {
    FaceDetection {
        box_: BoundingBox::new(20.0, 20.0, 80.0, 80.0),
        score: 0.98,
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
}

fn fixture(camera: Arc<dyn CameraManager>) -> Fixture {
    let temp = TempDir::new().unwrap();
    let store = Arc::new(
        BiometricStore::new(
            temp.path().join("biometrics"),
            MasterKey::generate().unwrap(),
        )
        .unwrap(),
    );
    let pipeline = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_with_detections(vec![single_face()])),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig {
            match_threshold: 0.50,
            ..Default::default()
        },
    ));
    let service = EnrollmentService::new(Arc::clone(&store), camera, pipeline, false);
    Fixture {
        _temp: temp,
        service,
        store,
    }
}

fn mock_camera_with_error(error: CameraError) -> Arc<MockCameraManager> {
    let camera = Arc::new(MockCameraManager::new(CameraConfig {
        warmup_frames: 0,
        ..Default::default()
    }));
    camera.set_error(Some(error));
    wait_until_no_cached_frame(&camera);
    camera
}

/// Setup guard against the mock worker race: a worker iteration that read "no error" just
/// before `set_error`/`set_starved` may still publish one frame after the slot was cleared; the
/// worker withdraws it on its next iteration and never publishes again while the fault is set.
/// Waits (bounded) until the slot has stayed empty for `STABLE_EMPTY` (many 33 ms frame
/// intervals), so no frame is cached when the service call starts.
fn wait_until_no_cached_frame(camera: &MockCameraManager) {
    const STABLE_EMPTY: Duration = Duration::from_millis(250);
    const DEADLINE: Duration = Duration::from_secs(5);
    let start = Instant::now();
    let mut empty_since: Option<Instant> = None;
    while start.elapsed() < DEADLINE {
        if camera.latest_frame().is_none() {
            let since = *empty_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= STABLE_EMPTY {
                break;
            }
        } else {
            empty_since = None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        camera.latest_frame().is_none(),
        "fixture setup: the faulted mock camera must not cache a frame"
    );
}

fn busy_mock_camera() -> Arc<MockCameraManager> {
    mock_camera_with_error(CameraError::Simulated {
        code: libc::EBUSY,
        message: "device held by another process".to_string(),
    })
}

fn enroll_args(uid: u32, frames: usize) -> EnrollArgs {
    EnrollArgs {
        uid: Some(uid),
        username: None,
        frames,
        yes: true,
        model_id: "arcface_w600k_mbf".to_string(),
        model_version: "2.0.0".to_string(),
    }
}

fn enroll_template(store: &BiometricStore, uid: u32) {
    let embedding = MockEmbeddingExtractor::new(512)
        .extract_embedding(&vec![128u8; 112 * 112 * 3], 112, 112)
        .unwrap();
    let template = BiometricTemplate::new(
        uid,
        MODEL_ID_EMBEDDING.to_string(),
        "1.0.0".to_string(),
        1_700_000_000,
        Zeroizing::new(embedding.as_slice().to_vec()),
    )
    .unwrap();
    store.enroll(&template).unwrap();
}

fn assert_starved(res: &Result<impl std::fmt::Debug, EnrollmentCliError>, case: &str) {
    assert!(
        matches!(res, Err(EnrollmentCliError::Camera(CameraError::Starved))),
        "{case}: a timeout without a DeviceBusy status must stay Camera(Starved), got {res:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// ECB1 — busy camera ⇒ CameraBusy with operator guidance
// ---------------------------------------------------------------------------------------------

#[test]
fn test_ecb1_enroll_busy_camera_returns_camera_busy() {
    let camera = busy_mock_camera();
    assert!(
        matches!(
            camera.status(),
            CameraStatus::Error {
                kind: CameraErrorKind::DeviceBusy,
                ..
            }
        ),
        "fixture precondition: the mock must report DeviceBusy"
    );
    let fx = fixture(camera);

    let res = fx.service.enroll(&enroll_args(3370, 3), |_| true);

    assert!(
        matches!(res, Err(EnrollmentCliError::CameraBusy)),
        "first-frame timeout with a DeviceBusy status must return CameraBusy, got {res:?}"
    );
    assert!(
        fx.store.get(3370).unwrap().is_none(),
        "a busy camera must never store a template"
    );
}

#[test]
fn test_ecb1_verify_busy_camera_returns_camera_busy() {
    let fx = fixture(busy_mock_camera());
    enroll_template(&fx.store, 3371);

    let res = fx.service.verify(&VerifyArgs {
        uid: Some(3371),
        username: None,
    });

    assert!(
        matches!(res, Err(EnrollmentCliError::CameraBusy)),
        "verify capture timeout with a DeviceBusy status must return CameraBusy, got {res:?}"
    );
}

#[test]
fn test_ecb1_enroll_busy_custom_camera_with_failure_count_returns_camera_busy() {
    let camera: Arc<dyn CameraManager> = Arc::new(StatusStubCamera::silent(CameraStatus::Error {
        kind: CameraErrorKind::DeviceBusy,
        failures: 17,
    }));
    let fx = fixture(camera);

    let res = fx.service.enroll(&enroll_args(3372, 1), |_| true);

    assert!(
        matches!(res, Err(EnrollmentCliError::CameraBusy)),
        "DeviceBusy with any failure count must return CameraBusy, got {res:?}"
    );
}

#[test]
fn test_ecb1_enroll_camera_goes_busy_mid_enrollment_returns_camera_busy() {
    // First candidate is captured, then the device becomes busy and no fresh frame arrives
    // within the per-candidate budget: the fresh-frame path reports CameraBusy as well.
    let camera: Arc<dyn CameraManager> = Arc::new(StatusStubCamera::with_frames(
        CameraStatus::Error {
            kind: CameraErrorKind::DeviceBusy,
            failures: 2,
        },
        1,
    ));
    let fx = fixture(camera);

    let res = fx.service.enroll(&enroll_args(3373, 3), |_| true);

    assert!(
        matches!(res, Err(EnrollmentCliError::CameraBusy)),
        "fresh-frame timeout with a DeviceBusy status must return CameraBusy, got {res:?}"
    );
    assert!(fx.store.get(3373).unwrap().is_none());
}

#[test]
fn test_ecb1_camera_busy_message_guides_operator() {
    let text = EnrollmentCliError::CameraBusy.to_string();
    for needle in [
        "another process holds",
        "soos-daemon",
        "soos-gui",
        "sudo systemctl stop soos-daemon",
        "sudo systemctl start soos-daemon",
    ] {
        assert!(
            text.contains(needle),
            "CameraBusy message must contain {needle:?}, got {text:?}"
        );
    }
}

#[test]
fn test_ecb1_camera_busy_from_enroll_displays_guidance() {
    let fx = fixture(busy_mock_camera());

    let err = fx
        .service
        .enroll(&enroll_args(3374, 1), |_| true)
        .expect_err("a busy camera must fail enrollment");
    let text = err.to_string();

    for needle in [
        "another process holds",
        "soos-daemon",
        "soos-gui",
        "sudo systemctl stop soos-daemon",
        "sudo systemctl start soos-daemon",
    ] {
        assert!(
            text.contains(needle),
            "enroll error on a busy camera must contain {needle:?}, got {text:?}"
        );
    }
    assert!(
        !text.contains("starved"),
        "the busy message must not be the generic starvation text, got {text:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// ECB2 — every other status keeps Camera(Starved)
// ---------------------------------------------------------------------------------------------

#[test]
fn test_ecb2_enroll_device_not_found_stays_starved() {
    let fx = fixture(mock_camera_with_error(CameraError::Simulated {
        code: libc::ENODEV,
        message: "device unplugged".to_string(),
    }));
    let res = fx.service.enroll(&enroll_args(3380, 1), |_| true);
    assert_starved(&res, "ENODEV");
}

#[test]
fn test_ecb2_enroll_starved_mock_stays_starved() {
    let camera = Arc::new(MockCameraManager::new(CameraConfig {
        warmup_frames: 0,
        ..Default::default()
    }));
    camera.set_starved(true);
    wait_until_no_cached_frame(&camera);
    let fx = fixture(camera);
    let res = fx.service.enroll(&enroll_args(3381, 1), |_| true);
    assert_starved(&res, "set_starved");
}

#[test]
fn test_ecb2_enroll_suspended_status_stays_starved() {
    let fx = fixture(Arc::new(StatusStubCamera::silent(CameraStatus::Suspended)));
    let res = fx.service.enroll(&enroll_args(3382, 1), |_| true);
    assert_starved(&res, "Suspended");
}

#[test]
fn test_ecb2_enroll_starting_status_stays_starved() {
    let fx = fixture(Arc::new(StatusStubCamera::silent(CameraStatus::Starting)));
    let res = fx.service.enroll(&enroll_args(3383, 1), |_| true);
    assert_starved(&res, "Starting");
}

#[test]
fn test_ecb2_enroll_other_error_kinds_stay_starved() {
    for kind in CameraErrorKind::ALL
        .into_iter()
        .filter(|k| *k != CameraErrorKind::DeviceBusy)
    {
        let fx = fixture(Arc::new(StatusStubCamera::silent(CameraStatus::Error {
            kind,
            failures: 3,
        })));
        // `NotEnrolled` is checked before capture, so a template must exist.
        enroll_template(&fx.store, 3384);
        let res = fx.service.verify(&VerifyArgs {
            uid: Some(3384),
            username: None,
        });
        assert_starved(&res, &format!("Error {{ kind: {kind:?} }}"));
    }
}

#[test]
fn test_ecb2_verify_device_not_found_stays_starved() {
    let fx = fixture(mock_camera_with_error(CameraError::Simulated {
        code: libc::ENODEV,
        message: "device unplugged".to_string(),
    }));
    enroll_template(&fx.store, 3385);
    let res = fx.service.verify(&VerifyArgs {
        uid: Some(3385),
        username: None,
    });
    assert_starved(&res, "verify ENODEV");
}

// ---------------------------------------------------------------------------------------------
// ECB-F — a frame that arrives in time is used whatever the status (guards a status-first impl)
// ---------------------------------------------------------------------------------------------

#[test]
fn test_ecb_frame_arriving_while_status_busy_is_used_for_enroll() {
    let camera: Arc<dyn CameraManager> = Arc::new(StatusStubCamera::with_frames(
        CameraStatus::Error {
            kind: CameraErrorKind::DeviceBusy,
            failures: 1,
        },
        u64::MAX,
    ));
    let fx = fixture(camera);

    let res = fx.service.enroll(&enroll_args(3390, 3), |_| true);

    assert!(
        res.is_ok(),
        "frames that arrive in time must be used even with a stale DeviceBusy status, got {res:?}"
    );
    assert!(fx.store.get(3390).unwrap().is_some());
}

#[test]
fn test_ecb_frame_arriving_while_status_busy_is_used_for_verify() {
    let camera: Arc<dyn CameraManager> = Arc::new(StatusStubCamera::with_frames(
        CameraStatus::Error {
            kind: CameraErrorKind::DeviceBusy,
            failures: 1,
        },
        u64::MAX,
    ));
    let fx = fixture(camera);
    enroll_template(&fx.store, 3391);

    let res = fx.service.verify(&VerifyArgs {
        uid: Some(3391),
        username: None,
    });

    assert!(
        res.is_ok(),
        "verify must use a frame that arrives in time whatever the status, got {res:?}"
    );
}

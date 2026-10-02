//! GUI findings of the 2026-10-02 review (GitHub #304, #305, #306, #314; matrix rows GCV1,
//! GCV2, GCV3, GCV15, GCV16, GCV17, GCV18).
//!
//! - #304: guided enrollment samples only frames with exactly one face;
//! - #305: a greyscale preview (the daemon sends IR frames as Grey) takes the Monochrome PAD path;
//! - #306: an empty preview after the first frame withdraws the frame and reports the source
//!   as unavailable;
//! - #314 S2: unknown wire formats and payloads that disagree with the geometry are protocol
//!   errors; CAM-NEW-5: `--mock` requires `--dev-store`; CAM-NEW-6: frame copies are
//!   zeroizing; CAM-NEW-7(a): transitional systemd states count as active.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite uses direct assertions and synthetic fixtures"
)]

#[path = "common/stamps.rs"]
mod stamps;

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use soos_camera_v4l::{CameraErrorKind, CameraManager, CameraStatus, Frame, PixelFormat};
use soos_enrollment_cli::guided_enrollment::DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES;
use soos_gui::args::GuiArgs;
use soos_gui::daemon_control::active_state_means_running;
use soos_gui::ipc_camera::frame_from_preview;
use soos_gui::state::LatestFrameData;
use soos_gui::worker::{
    feed_guided_enrollment, guided_enrollment_feedback, new_guided_enrollment_session,
    GuiEnrollmentFeedback,
};
use soos_gui::{IpcCameraManager, IpcPreviewError};
use soos_inference_ort::{
    BoundingBox, FaceDetection, MockEmbeddingExtractor, MockFaceDetector, MockPadDetector,
    PadResult,
};
use soos_protocol::codec::{decode, encode_preview};
use soos_protocol::types::{PreviewResponse, Request, CURRENT_VERSION};
use soos_vision::pose::HeadPose;
use soos_vision::{PadInputModality, VisionAnalysis, VisionPipeline, VisionPipelineConfig};
use zeroize::Zeroizing;

const W: u32 = 640;
const H: u32 = 480;
const PAD_THRESHOLD: f32 = 0.85;

fn detection(x1: f32) -> FaceDetection {
    FaceDetection::new(BoundingBox::new(x1, 140.0, x1 + 200.0, 340.0), 0.95)
}

/// A live, centred, fully analysed frame with the given detections.
fn analysis_with(detections: Vec<FaceDetection>) -> VisionAnalysis {
    let mut embedding = vec![0.0f32; 128];
    embedding[0] = 1.0;
    VisionAnalysis {
        rgb: Zeroizing::new(Vec::new()),
        detections,
        pad_result: Some(PadResult::live(0.98)),
        pose: Some(HeadPose {
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
        }),
        aligned_crop: None,
        embedding: Some(Zeroizing::new(embedding)),
        quality_rejection: None,
    }
}

// ---------------------------------------------------------------------------
// GCV1 — #304: one face only
// ---------------------------------------------------------------------------

/// GCV1: a two-face frame is never sampled, even with a live verdict and an embedding, and
/// it breaks the consecutive-live streak.
#[test]
fn test_gcv_two_face_frame_records_no_sample_and_breaks_streak() {
    let mut session = new_guided_enrollment_session();
    let one = analysis_with(vec![detection(220.0)]);
    let two = analysis_with(vec![detection(100.0), detection(340.0)]);

    for _ in 1..DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES {
        feed_guided_enrollment(&mut session, &one, W, H, PAD_THRESHOLD);
    }
    assert_eq!(
        feed_guided_enrollment(&mut session, &two, W, H, PAD_THRESHOLD),
        None,
        "a multi-face frame reports nothing through the session feed"
    );
    assert!(
        session.progress_percent() <= 0.0,
        "no sample may be recorded from a multi-face frame"
    );
    // The streak restarted: the next single-face frame is not yet sampled.
    let next = feed_guided_enrollment(&mut session, &one, W, H, PAD_THRESHOLD);
    assert!(
        !matches!(
            next,
            Some(
                soos_enrollment_cli::guided_enrollment::EnrollmentStepFeedback::SampleAccepted { .. }
            )
        ),
        "{next:?}"
    );
}

/// GCV1: the GUI reports "one face only" for a multi-face frame.
#[test]
fn test_gcv_gui_feedback_reports_one_face_only() {
    let mut session = new_guided_enrollment_session();
    let two = analysis_with(vec![detection(100.0), detection(340.0)]);
    assert_eq!(
        guided_enrollment_feedback(&mut session, &two, W, H, PAD_THRESHOLD),
        Some(GuiEnrollmentFeedback::OneFaceOnly)
    );
    assert!(GuiEnrollmentFeedback::OneFaceOnly
        .message()
        .to_ascii_lowercase()
        .contains("one face"));
    let one = analysis_with(vec![detection(220.0)]);
    assert!(matches!(
        guided_enrollment_feedback(&mut session, &one, W, H, PAD_THRESHOLD),
        Some(GuiEnrollmentFeedback::Step(_))
    ));
}

/// GCV1 end to end: a mock detector reporting two confident faces never yields a sample.
#[test]
fn test_gcv_mock_detector_with_two_faces_never_yields_a_sample() {
    let faces = [160.0f32, 480.0].map(|cx| {
        let box_ = BoundingBox::new(cx - 70.0, 170.0, cx + 70.0, 310.0);
        let landmarks = MockFaceDetector::canonical_landmarks_for_box(&box_);
        FaceDetection::with_landmarks(box_, 0.97, landmarks)
    });
    let pipeline = VisionPipeline::new(
        Arc::new(MockFaceDetector::new_with_detections(faces.to_vec())),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(128)),
        VisionPipelineConfig::default(),
    );
    let frame = Frame::new(
        vec![128u8; (W * H * 3) as usize],
        W,
        H,
        1,
        PixelFormat::Rgb24,
        1,
    );
    let mut session = new_guided_enrollment_session();
    for _ in 0..(4 * DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES) {
        let analysis = pipeline.analyze_frame(&frame).unwrap();
        assert_eq!(
            guided_enrollment_feedback(&mut session, &analysis, W, H, PAD_THRESHOLD),
            Some(GuiEnrollmentFeedback::OneFaceOnly)
        );
    }
    assert!(session.progress_percent() <= 0.0);
}

// ---------------------------------------------------------------------------
// GCV2 / GCV18 — #305 and S2: preview decoding
// ---------------------------------------------------------------------------

fn preview(width: u32, height: u32, format: u8, data: Vec<u8>) -> PreviewResponse {
    PreviewResponse {
        version: CURRENT_VERSION,
        sequence: 3,
        width,
        height,
        format,
        timestamp_monotonic_ns: 42,
        data,
    }
}

/// GCV2: a greyscale preview (how the daemon sends IR frames) is a Monochrome PAD source.
#[test]
fn test_gcv_grey_preview_frame_takes_monochrome_pad_path() {
    let mut resp = preview(4, 2, 1, vec![9u8; 8]);
    let frame = frame_from_preview(&mut resp).unwrap().expect("a frame");
    assert_eq!(frame.format, PixelFormat::Grey);
    assert_eq!(
        PadInputModality::for_frame(&frame),
        PadInputModality::Monochrome
    );
    let mut colour = preview(4, 2, 0, vec![9u8; 24]);
    let frame = frame_from_preview(&mut colour).unwrap().expect("a frame");
    assert_eq!(PadInputModality::for_frame(&frame), PadInputModality::Color);
}

/// GCV18: unknown wire formats and geometry mismatches are protocol errors, never Rgb24.
#[test]
fn test_gcv_preview_validation_rejects_unknown_format_and_bad_length() {
    let cases = [
        preview(4, 2, 5, vec![1u8; 24]),
        preview(4, 2, 254, vec![1u8; 24]),
        preview(4, 2, 255, vec![1u8; 24]),
        preview(4, 2, 0, vec![1u8; 23]),
        preview(4, 2, 0, vec![1u8; 25]),
        preview(4, 2, 1, vec![1u8; 9]),
        preview(4, 2, 2, vec![1u8; 15]),
        preview(4, 2, 3, vec![1u8; 11]),
        preview(0, 2, 0, vec![1u8; 6]),
        preview(4, 0, 4, vec![1u8; 6]),
    ];
    for mut resp in cases {
        let label = format!("{}x{} format {}", resp.width, resp.height, resp.format);
        assert_eq!(
            frame_from_preview(&mut resp).map(|f| f.is_some()),
            Err(IpcPreviewError::Protocol),
            "{label}"
        );
    }
    // Valid payloads of every known format; MJPEG is compressed (any non-empty length).
    for (format, len) in [(0u8, 24usize), (1, 8), (2, 16), (3, 12), (4, 5)] {
        let mut resp = preview(4, 2, format, vec![1u8; len]);
        assert!(
            frame_from_preview(&mut resp).unwrap().is_some(),
            "format {format}"
        );
    }
    // The explicit empty preview is not an error.
    let mut empty = preview(0, 0, 255, Vec::new());
    assert_eq!(
        frame_from_preview(&mut empty).map(|f| f.is_some()),
        Ok(false)
    );
}

// ---------------------------------------------------------------------------
// GCV3 — #306: empty previews after the first frame
// ---------------------------------------------------------------------------

fn read_request(stream: &mut UnixStream) -> Option<Request> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).ok()?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + len];
    buf[..4].copy_from_slice(&len_bytes);
    stream.read_exact(&mut buf[4..]).ok()?;
    decode::<Request>(&buf).ok()
}

fn encoded(resp: &PreviewResponse) -> Vec<u8> {
    encode_preview(resp).unwrap()
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    condition()
}

/// GCV3: one frame, then empty previews: the frame is withdrawn, the manager is not ready and
/// the status names the unavailable source; a new frame recovers.
#[test]
fn test_gcv_empty_preview_after_first_frame_marks_source_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (phase_tx, phase_rx) = std::sync::mpsc::channel::<u8>();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut phase = 0u8;
        while let Some(_req) = read_request(&mut stream) {
            while let Ok(p) = phase_rx.try_recv() {
                phase = p;
            }
            let reply = match phase {
                0 => preview(4, 2, 0, vec![7u8; 24]),
                1 => preview(0, 0, 255, Vec::new()),
                2 => preview(4, 2, 0, vec![8u8; 24]),
                _ => break,
            };
            if stream.write_all(&encoded(&reply)).is_err() {
                break;
            }
        }
    });

    let manager = IpcCameraManager::spawn(&sock);
    assert!(wait_until(Duration::from_secs(5), || manager.is_ready()));
    assert!(manager.latest_frame().is_some());

    phase_tx.send(1).unwrap();
    assert!(
        wait_until(Duration::from_secs(5), || !manager.is_ready()),
        "an empty preview after a frame must mark the source not ready"
    );
    assert!(
        manager.latest_frame().is_none(),
        "the frozen frame is withdrawn"
    );
    assert!(
        matches!(
            manager.status(),
            CameraStatus::Error {
                kind: CameraErrorKind::SourceUnavailable,
                ..
            }
        ),
        "{:?}",
        manager.status()
    );
    assert_eq!(manager.last_error(), Some(IpcPreviewError::Unavailable));

    phase_tx.send(2).unwrap();
    assert!(
        wait_until(Duration::from_secs(5), || manager.is_ready()),
        "a new frame recovers the source"
    );
    assert_eq!(manager.last_error(), None);
    phase_tx.send(3).unwrap();
    drop(manager);
    server.join().unwrap();
}

/// GCV3: before the first frame an empty preview means "warming up", not a failure.
#[test]
fn test_gcv_empty_preview_before_first_frame_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        while read_request(&mut stream).is_some() {
            if stream
                .write_all(&encoded(&preview(0, 0, 255, Vec::new())))
                .is_err()
            {
                break;
            }
        }
    });
    let manager = IpcCameraManager::spawn(&sock);
    std::thread::sleep(Duration::from_millis(250));
    assert!(!manager.is_ready());
    assert_eq!(manager.last_error(), None);
    assert_eq!(manager.status(), CameraStatus::Starting);
    drop(manager);
    server.join().unwrap();
}

// ---------------------------------------------------------------------------
// GCV15 / GCV16 / GCV17 — #314
// ---------------------------------------------------------------------------

/// GCV15 (CAM-NEW-5): mock models may only write to the developer store.
#[test]
fn test_gcv_mock_mode_requires_dev_store() {
    assert!(GuiArgs::try_parse_from(["soos-gui", "--mock"]).is_err());
    let args =
        GuiArgs::try_parse_from(["soos-gui", "--mock", "--dev-store", "/home/u/soos-dev"]).unwrap();
    assert!(args.mock);
    assert!(GuiArgs::try_parse_from(["soos-gui"]).is_ok());
}

/// GCV16 (CAM-NEW-6): published frame copies are zeroizing containers (type-level) and
/// `Debug` never prints their bytes.
#[test]
fn test_gcv_latest_frame_buffers_are_zeroizing_and_redacted() {
    let data = LatestFrameData {
        rgb: Zeroizing::new(vec![201u8; 12]),
        width: 2,
        height: 2,
        detections: Vec::new(),
        pad_result: None,
        pad_live: false,
        pose: None,
        aligned_crop: Some(Zeroizing::new(vec![203u8; 12])),
        pipeline_latency_ms: 0.0,
        det_latency_ms: 0.0,
        pad_latency_ms: 0.0,
        fps: 0.0,
        sequence: 1,
    };
    let rgb: &Zeroizing<Vec<u8>> = &data.rgb;
    assert_eq!(rgb.len(), 12);
    let text = format!("{data:?}");
    assert!(!text.contains("201"), "{text}");
    assert!(!text.contains("203"), "{text}");
}

/// GCV17 (CAM-NEW-7a): a daemon that is starting, restarting, reloading or stopping still
/// owns the camera; only `inactive` and `failed` let the GUI open the device directly.
#[test]
fn test_gcv_transitional_systemd_states_count_as_running() {
    for state in [
        "active",
        "activating",
        "deactivating",
        "reloading",
        "refreshing",
        "",
        "something-new",
    ] {
        assert!(active_state_means_running(state), "{state:?}");
    }
    for state in ["inactive", "failed", "inactive\n", " failed "] {
        assert!(!active_state_means_running(state), "{state:?}");
    }
}

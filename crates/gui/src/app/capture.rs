//! Synthetic README capture scenes (GitHub #381, PR 1 of #356; ADR 2026-10-13 "README Assets
//! Are Generated From Synthetic Data, Never Hand-Edited"; spec
//! `AI/architect_spec_readme_capture.md` §3.1 and §10).
//!
//! Compiled only with the dev-only `readme-capture` feature. A capture scene is a real
//! [`SoosApp`] whose side effects are all replaced: no vision worker, no camera thread or
//! device, no `systemctl`, no `pkexec`, no biometric store, no host identity. The vision
//! pipeline (mock detector, liveness and embedding models) runs once, synchronously, on the
//! synthetic frame the caller passes (a `soos-test-fixtures` placeholder face), and the
//! telemetry values are fixed constants.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use soos_camera_v4l::{CameraManager, Frame, PixelFormat};
use soos_enrollment_cli::service::{
    EnrolledUserSummary, EMBEDDING_MODEL_VERSION, MODEL_ID_EMBEDDING,
};
use soos_inference_ort::{
    MockEmbeddingExtractor, MockFaceDetector, MockPadDetector, EMBEDDING_DIMENSION,
};
use soos_vision::{PadInputModality, VisionPipeline, VisionPipelineConfig};

use super::{AppTab, SoosApp};
use crate::daemon_control::{DaemonMonitor, DaemonState, DaemonStatusProbe, DAEMON_POLL_INTERVAL};
use crate::privileged::{PrivilegedAction, PrivilegedExecutor, PrivilegedOutcome, TaskRunner};
use crate::state::{EnrollmentGuiState, LatestFrameData, ProfilesGuiState};
use crate::store_mode::GuiStore;
use crate::theme;
use crate::worker::WorkerSharedInput;

/// Username shown by every capture scene (never the host account).
pub const CAPTURE_USERNAME: &str = "alice";
/// Uid shown by every capture scene.
pub const CAPTURE_UID: u32 = 1000;
/// Fixed frame rate shown in the header band.
pub const CAPTURE_FPS: f32 = 30.0;
/// Fixed pipeline latency shown in the header band and the telemetry card.
pub const CAPTURE_LATENCY_MS: f64 = 12.5;
/// Enrollment time of the synthetic profile (2026-10-01T00:00:00Z).
pub const CAPTURE_ENROLLED_AT_UNIX_S: u64 = 1_790_812_800;
/// Logical window size of every capture, in points (the `soos-gui` default window).
pub const CAPTURE_WINDOW_POINTS: [f32; 2] = [1120.0, 780.0];
/// Bound on the wait for the fixed daemon probe to publish its first state.
pub const CAPTURE_DAEMON_STATE_TIMEOUT: Duration = Duration::from_secs(2);
/// Detection score of the mock face detector.
const CAPTURE_DETECTION_SCORE: f32 = 0.97;
/// Poll period of the daemon-state wait.
const DAEMON_STATE_POLL: Duration = Duration::from_millis(10);

/// One README screen of `soos-gui`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureScene {
    /// Live diagnostic view with the model overlays.
    LiveInspection,
    /// Guided enrollment, before the first step.
    GuidedEnrollment,
    /// Enrolled profiles (one synthetic profile).
    Profiles,
}

impl CaptureScene {
    /// Every scene, in README order.
    pub const ALL: [CaptureScene; 3] = [
        CaptureScene::LiveInspection,
        CaptureScene::GuidedEnrollment,
        CaptureScene::Profiles,
    ];

    /// File stem of the scene's asset under `Docs/assets/screenshots/`.
    pub fn file_stem(self) -> &'static str {
        match self {
            CaptureScene::LiveInspection => "gui-live-inspection",
            CaptureScene::GuidedEnrollment => "gui-guided-enrollment",
            CaptureScene::Profiles => "gui-profiles",
        }
    }

    fn tab(self) -> AppTab {
        match self {
            CaptureScene::LiveInspection => AppTab::LiveInspection,
            CaptureScene::GuidedEnrollment => AppTab::GuidedEnrollment,
            CaptureScene::Profiles => AppTab::Profiles,
        }
    }
}

/// Why a capture scene could not be built.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The frame is not RGB24 with a consistent, non-empty buffer.
    #[error("capture frame must be RGB24 with non-zero dimensions")]
    UnsupportedFrame,
    /// The (mock) vision pipeline refused the frame.
    #[error("vision pipeline failed on the capture frame: {0}")]
    Pipeline(String),
    /// The daemon-state thread could not be spawned.
    #[error("capture thread could not be spawned: {0}")]
    Spawn(std::io::Error),
    /// The fixed daemon state was not published within the bound.
    #[error("the fixed daemon state was not published in time")]
    DaemonStateTimeout,
}

/// The identities a capture scene can display (spec §10, F-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureIdentities {
    /// Target username of the enrollment form.
    pub enrollment_username: String,
    /// Target uid of the enrollment form.
    pub enrollment_uid: u32,
    /// Usernames of the listed profiles.
    pub profile_usernames: Vec<String>,
}

/// Returns the identities `app` can display.
pub fn shown_identities(app: &SoosApp) -> CaptureIdentities {
    CaptureIdentities {
        enrollment_username: app.enrollment.target_username.clone(),
        enrollment_uid: app.enrollment.target_uid,
        profile_usernames: app
            .profiles
            .profiles
            .iter()
            .map(|p| p.username.clone())
            .collect(),
    }
}

/// Thread-free camera: ready, never yields a frame (the scene frame is injected directly).
#[derive(Debug, Default)]
struct StaticCaptureCamera;

impl CameraManager for StaticCaptureCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        None
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn notify_activity(&self) {}

    fn stop(&self) {}
}

/// Daemon probe answering a fixed state without running any process.
#[derive(Debug)]
struct FixedDaemonProbe {
    active: bool,
}

impl DaemonStatusProbe for FixedDaemonProbe {
    fn is_active(&self) -> bool {
        self.active
    }
}

/// Executor refusing every privileged action without running anything (never reached: a
/// capture submits no action).
#[derive(Debug)]
struct RefusingExecutor;

impl PrivilegedExecutor for RefusingExecutor {
    fn execute(&self, action: PrivilegedAction) -> PrivilegedOutcome {
        fn refused<T>() -> Result<T, String> {
            Err("privileged actions are disabled in README captures".to_string())
        }
        match action {
            PrivilegedAction::PauseDaemon => PrivilegedOutcome::DaemonPaused(refused()),
            PrivilegedAction::ResumeDaemon => PrivilegedOutcome::DaemonResumed(refused()),
            PrivilegedAction::ListProfiles => PrivilegedOutcome::ProfilesListed(refused()),
            PrivilegedAction::ImportTemplate { uid, .. } => PrivilegedOutcome::TemplateImported {
                uid,
                result: refused(),
            },
            PrivilegedAction::DeleteTemplate { uid } => PrivilegedOutcome::TemplateDeleted {
                uid,
                result: refused(),
            },
        }
    }
}

/// Whether `frame` is RGB24 with a consistent, non-empty buffer.
fn frame_is_supported(frame: &Frame) -> bool {
    if frame.format != PixelFormat::Rgb24 || frame.width == 0 || frame.height == 0 {
        return false;
    }
    (frame.width as usize)
        .checked_mul(frame.height as usize)
        .and_then(|pixels| pixels.checked_mul(3))
        .is_some_and(|len| len == frame.data.len())
}

/// Builds a [`SoosApp`] showing `scene` from the synthetic `frame`, without any worker,
/// camera, `systemctl`, `pkexec`, store or host-identity side effect.
///
/// # Errors
///
/// [`CaptureError`] for an unsupported frame, a pipeline failure, a thread spawn failure or
/// a daemon state not published within [`CAPTURE_DAEMON_STATE_TIMEOUT`].
pub fn capture_app(
    cc: &eframe::CreationContext<'_>,
    scene: CaptureScene,
    frame: &Frame,
) -> Result<SoosApp, CaptureError> {
    theme::apply(&cc.egui_ctx);
    if !frame_is_supported(frame) {
        return Err(CaptureError::UnsupportedFrame);
    }

    let pipeline = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_centered_face(
            frame.width,
            frame.height,
            CAPTURE_DETECTION_SCORE,
        )),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(EMBEDDING_DIMENSION)),
        VisionPipelineConfig::default(),
    ));
    let analysis = pipeline
        .analyze_frame(frame)
        .map_err(|e| CaptureError::Pipeline(e.to_string()))?;
    let modality = PadInputModality::for_frame(frame);
    let pad_live = analysis
        .pad_result
        .as_ref()
        .is_some_and(|p| pipeline.config().pad_passes(p, modality));
    let frame_data = LatestFrameData {
        rgb: analysis.rgb.clone(),
        width: frame.width,
        height: frame.height,
        detections: analysis.detections.clone(),
        pad_result: analysis.pad_result.clone(),
        pad_live,
        pose: analysis.pose,
        aligned_crop: analysis.aligned_crop.clone(),
        pipeline_latency_ms: CAPTURE_LATENCY_MS,
        det_latency_ms: CAPTURE_LATENCY_MS * 0.45,
        pad_latency_ms: CAPTURE_LATENCY_MS * 0.25,
        fps: CAPTURE_FPS,
        sequence: 1,
    };
    let latest_frame_slot = Arc::new(ArcSwapOption::from_pointee(frame_data));

    let monitor = DaemonMonitor::spawn(
        Arc::new(FixedDaemonProbe { active: true }),
        DAEMON_POLL_INTERVAL,
    )
    .map_err(CaptureError::Spawn)?;
    let started = Instant::now();
    while monitor.state() != DaemonState::Active {
        if started.elapsed() >= CAPTURE_DAEMON_STATE_TIMEOUT {
            return Err(CaptureError::DaemonStateTimeout);
        }
        std::thread::sleep(DAEMON_STATE_POLL);
    }

    let tasks = TaskRunner::new(Arc::new(RefusingExecutor), Arc::new(|| {}));
    let enrollment = EnrollmentGuiState {
        target_username: CAPTURE_USERNAME.to_string(),
        target_uid: CAPTURE_UID,
        ..EnrollmentGuiState::default()
    };
    let profiles = ProfilesGuiState {
        profiles: if scene == CaptureScene::Profiles {
            vec![EnrolledUserSummary {
                uid: CAPTURE_UID,
                username: CAPTURE_USERNAME.to_string(),
                model_id: MODEL_ID_EMBEDDING.to_string(),
                model_version: EMBEDDING_MODEL_VERSION.to_string(),
                enrollment_timestamp: CAPTURE_ENROLLED_AT_UNIX_S,
                embedding_dim: EMBEDDING_DIMENSION,
            }]
        } else {
            Vec::new()
        },
        ..ProfilesGuiState::default()
    };

    Ok(SoosApp {
        current_tab: scene.tab(),
        store: GuiStore::Polkit,
        store_banner: None,
        camera: Arc::new(StaticCaptureCamera),
        pipeline,
        match_reference_note: None,
        latest_frame_slot,
        worker_input: Arc::new(WorkerSharedInput::default()),
        worker_running: Arc::new(AtomicBool::new(false)),
        _worker_thread: None,
        video_texture: None,
        aligned_crop_texture: None,
        last_rendered_seq: u64::MAX,
        show_bbox: true,
        show_landmarks: true,
        show_pad_crop: true,
        show_pose_stats: true,
        show_crop_inset: true,
        enrollment,
        profiles,
        camera_notice: None,
        daemon_monitor: Some(monitor),
        tasks,
        store_tasks: None,
        pending_store_username: None,
        daemon_message: None,
        last_camera_status: None,
        camera_source: None,
        switchable_camera: None,
        handover_slot: Arc::new(OnceLock::new()),
        last_source_generation: 0,
    })
}

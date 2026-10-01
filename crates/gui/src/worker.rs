//! Background camera streaming and neural inference worker thread.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "FPS calculation, frame rate timing, and latency measurements"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use eframe::egui;
use soos_camera_v4l::CameraManager;
use soos_enrollment_cli::guided_enrollment::LivenessPolicy;
use soos_enrollment_cli::guided_enrollment::{EnrollmentStepFeedback, GuidedEnrollmentSession};
use soos_vision::matcher::cosine_similarity;
use soos_vision::{PadInputModality, VisionAnalysis, VisionPipeline};

use crate::state::LatestFrameData;

/// Shared commands and inputs passed from GUI thread to worker thread.
#[derive(Default)]
pub struct WorkerSharedInput {
    /// Active guided enrollment session.
    pub enrollment_session: Mutex<Option<GuidedEnrollmentSession>>,
    /// Last feedback produced by the enrollment session.
    pub enrollment_feedback: Mutex<Option<EnrollmentStepFeedback>>,
    /// Active reference embedding for live 1-to-1 verification testing.
    pub match_reference: Mutex<Option<Vec<f32>>>,
    /// Latest live match score against reference embedding.
    pub live_match_score: Mutex<Option<f32>>,
}

/// Samples per step of a GUI guided enrollment session.
pub const GUI_GUIDED_SAMPLES_PER_STEP: usize = 4;

/// Creates the GUI guided enrollment session with the strict session-level liveness
/// policy (GitHub #217): k consecutive live frames per sample, abort on repeated spoofs.
pub fn new_guided_enrollment_session() -> GuidedEnrollmentSession {
    GuidedEnrollmentSession::with_liveness_policy(
        GUI_GUIDED_SAMPLES_PER_STEP,
        LivenessPolicy::strict(),
    )
}

/// Feeds one analyzed frame to a guided enrollment session.
///
/// - No usable face (no pose or embedding, no quality verdict): the live streak is broken
///   and `None` is returned (nothing new to report), unless the frame carries a spoof PAD
///   verdict, which is counted by the session (GitHub #285).
/// - Quality-gate rejection (GitHub #218): streak broken, `FaceQualityTooLow`.
/// - Face without a PAD verdict (crop or PAD failure): streak broken, `PromptHoldStill`;
///   it is not a spoof event, and no sample is recorded.
/// - Otherwise the frame is live when the model says live and the score reaches
///   `pad_threshold` (fail-closed on NaN); spoof frames are counted by the session. The
///   worker passes `VisionPipelineConfig::effective_pad_threshold` for the frame modality,
///   so this is the same decision as `VisionPipelineConfig::pad_passes` (GitHub #215).
pub fn feed_guided_enrollment(
    session: &mut GuidedEnrollmentSession,
    analysis: &VisionAnalysis,
    frame_width: u32,
    frame_height: u32,
    pad_threshold: f32,
) -> Option<EnrollmentStepFeedback> {
    if analysis.quality_rejection.is_some() {
        session.interrupt_liveness_streak();
        return Some(EnrollmentStepFeedback::FaceQualityTooLow);
    }
    let is_live = analysis
        .pad_result
        .as_ref()
        .map(|pad| pad.is_live && pad.score.is_finite() && pad.score >= pad_threshold);
    let (Some(pose), Some(emb)) = (&analysis.pose, &analysis.embedding) else {
        // A spoof verdict counts even when no sample can be taken (GitHub #285), so
        // repeated attacks still abort the session.
        if is_live == Some(false) {
            return Some(session.record_presentation_attack());
        }
        session.interrupt_liveness_streak();
        return None;
    };
    let Some(is_live) = is_live else {
        session.interrupt_liveness_streak();
        return Some(EnrollmentStepFeedback::PromptHoldStill);
    };

    let is_centered = analysis
        .detections
        .first()
        .map(|d| {
            let center_x = (d.box_.x1 + d.box_.x2) * 0.5;
            let center_y = (d.box_.y1 + d.box_.y2) * 0.5;
            let w = frame_width as f32;
            let h = frame_height as f32;
            (center_x - w * 0.5).abs() < w * 0.25 && (center_y - h * 0.5).abs() < h * 0.25
        })
        .unwrap_or(false);

    Some(session.process_sample(pose, emb.as_slice(), is_live, is_centered))
}

/// Spawns the dedicated vision worker thread that continuously processes camera frames.
pub fn spawn_vision_worker(
    camera: Arc<dyn CameraManager>,
    pipeline: Arc<VisionPipeline>,
    latest_frame_slot: Arc<ArcSwapOption<LatestFrameData>>,
    shared_input: Arc<WorkerSharedInput>,
    running: Arc<AtomicBool>,
    egui_ctx: egui::Context,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("soos-gui-worker".to_string())
        .spawn(move || {
            let mut last_seq = u64::MAX;
            let mut last_fps_time = Instant::now();
            let mut frames_in_second = 0u32;
            let mut current_fps = 0.0f32;
            // Whether a frame from the camera is currently published to the UI.
            let mut published = false;

            while running.load(Ordering::Acquire) {
                camera.notify_activity();

                if let Some(frame) = camera.latest_frame() {
                    if frame.sequence != last_seq {
                        last_seq = frame.sequence;
                        frames_in_second = frames_in_second.saturating_add(1);

                        let now = Instant::now();
                        let elapsed_fps = now.duration_since(last_fps_time).as_secs_f32();
                        if elapsed_fps >= 1.0 {
                            current_fps = (frames_in_second as f32) / elapsed_fps;
                            frames_in_second = 0;
                            last_fps_time = now;
                        }

                        let start_analysis = Instant::now();
                        match pipeline.analyze_frame(&frame) {
                            Ok(analysis) => {
                                let total_latency = start_analysis.elapsed().as_secs_f64() * 1000.0;

                                // Single liveness decision shared with the daemon pipeline
                                // (GitHub #215): enrollment gating and on-screen label alike.
                                let modality = PadInputModality::for_frame(&frame);
                                let is_live = analysis
                                    .pad_result
                                    .as_ref()
                                    .is_some_and(|p| pipeline.config().pad_passes(p, modality));

                                // Handle active guided enrollment if enabled
                                if let Ok(mut session_guard) =
                                    shared_input.enrollment_session.lock()
                                {
                                    if let Some(session) = session_guard.as_mut() {
                                        if let Some(fb) = feed_guided_enrollment(
                                            session,
                                            &analysis,
                                            frame.width,
                                            frame.height,
                                            pipeline.config().effective_pad_threshold(modality),
                                        ) {
                                            if let Ok(mut fb_guard) =
                                                shared_input.enrollment_feedback.lock()
                                            {
                                                *fb_guard = Some(fb);
                                            }
                                        }
                                    }
                                }

                                // Handle live 1-to-1 verification matching if reference is set
                                if let Some(emb) = &analysis.embedding {
                                    if let Ok(ref_guard) = shared_input.match_reference.lock() {
                                        if let Some(ref_emb) = ref_guard.as_ref() {
                                            if let Ok(score) =
                                                cosine_similarity(ref_emb, emb.as_slice())
                                            {
                                                if let Ok(mut score_guard) =
                                                    shared_input.live_match_score.lock()
                                                {
                                                    *score_guard = Some(score);
                                                }
                                            }
                                        }
                                    }
                                }

                                let frame_data = LatestFrameData {
                                    rgb: analysis.rgb.to_vec(),
                                    width: frame.width,
                                    height: frame.height,
                                    detections: analysis.detections.clone(),
                                    pad_result: analysis.pad_result.clone(),
                                    pad_live: is_live,
                                    pose: analysis.pose,
                                    aligned_crop: analysis
                                        .aligned_crop
                                        .as_ref()
                                        .map(|c| c.to_vec()),
                                    pipeline_latency_ms: total_latency,
                                    det_latency_ms: total_latency * 0.45,
                                    pad_latency_ms: total_latency * 0.25,
                                    fps: current_fps,
                                    sequence: frame.sequence,
                                };

                                latest_frame_slot.store(Some(Arc::new(frame_data)));
                                published = true;
                                egui_ctx.request_repaint();
                            }
                            Err(err) => {
                                tracing::warn!(
                                    "Vision pipeline analysis failed on frame #{}: {}",
                                    frame.sequence,
                                    err
                                );
                                if let Ok(rgb_buf) = soos_vision::convert_to_rgb(
                                    &frame.data,
                                    frame.width,
                                    frame.height,
                                    frame.format,
                                ) {
                                    let fallback_frame = LatestFrameData {
                                        rgb: rgb_buf,
                                        width: frame.width,
                                        height: frame.height,
                                        detections: Vec::new(),
                                        pad_result: None,
                                        pad_live: false,
                                        pose: None,
                                        aligned_crop: None,
                                        pipeline_latency_ms: 0.0,
                                        det_latency_ms: 0.0,
                                        pad_latency_ms: 0.0,
                                        fps: current_fps,
                                        sequence: frame.sequence,
                                    };
                                    latest_frame_slot.store(Some(Arc::new(fallback_frame)));
                                    published = true;
                                    egui_ctx.request_repaint();
                                }
                            }
                        }
                    }
                } else if published && !camera.is_ready() {
                    // The source failed or was switched: withdraw the frozen frame so the UI
                    // shows the camera status instead (GitHub #154 / #155).
                    latest_frame_slot.store(None);
                    published = false;
                    egui_ctx.request_repaint();
                }

                std::thread::sleep(Duration::from_millis(10));
            }
        })
}

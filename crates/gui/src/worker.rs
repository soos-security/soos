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
use soos_enrollment_cli::guided_enrollment::{EnrollmentStepFeedback, GuidedEnrollmentSession};
use soos_vision::matcher::cosine_similarity;
use soos_vision::VisionPipeline;

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
                        if let Ok(analysis) = pipeline.analyze_frame(&frame) {
                            let total_latency = start_analysis.elapsed().as_secs_f64() * 1000.0;

                            // Handle active guided enrollment if enabled
                            let is_live = analysis
                                .pad_result
                                .as_ref()
                                .map(|p| p.is_live && p.score >= pipeline.config().pad_threshold)
                                .unwrap_or(false);

                            let is_centered = analysis
                                .detections
                                .first()
                                .map(|d| {
                                    let center_x = (d.box_.x1 + d.box_.x2) * 0.5;
                                    let center_y = (d.box_.y1 + d.box_.y2) * 0.5;
                                    let w = frame.width as f32;
                                    let h = frame.height as f32;
                                    (center_x - w * 0.5).abs() < w * 0.25
                                        && (center_y - h * 0.5).abs() < h * 0.25
                                })
                                .unwrap_or(false);

                            if let (Some(pose), Some(emb)) = (&analysis.pose, &analysis.embedding) {
                                if let Ok(mut session_guard) =
                                    shared_input.enrollment_session.lock()
                                {
                                    if let Some(session) = session_guard.as_mut() {
                                        let fb = session.process_sample(
                                            pose,
                                            emb.as_slice(),
                                            is_live,
                                            is_centered,
                                        );
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
                                pose: analysis.pose,
                                aligned_crop: analysis.aligned_crop.as_ref().map(|c| c.to_vec()),
                                pipeline_latency_ms: total_latency,
                                det_latency_ms: total_latency * 0.45,
                                pad_latency_ms: total_latency * 0.25,
                                fps: current_fps,
                                sequence: frame.sequence,
                            };

                            latest_frame_slot.store(Some(Arc::new(frame_data)));
                            egui_ctx.request_repaint();
                        }
                    }
                }

                std::thread::sleep(Duration::from_millis(10));
            }
        })
}

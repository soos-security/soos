//! Main interactive GUI application implementing `eframe::App`.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "GUI coordinate calculations, texture scaling, and progress percentage presentation"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwapOption;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use soos_biometric_store::{BiometricStore, BiometricTemplate};
use soos_camera_v4l::CameraManager;
use soos_enrollment_cli::guided_enrollment::{
    EnrollmentStep, EnrollmentStepFeedback, GuidedEnrollmentSession,
};
use soos_enrollment_cli::service::EnrolledUserSummary;
use soos_vision::VisionPipeline;
use zeroize::Zeroizing;

use crate::state::{EnrollmentGuiState, LatestFrameData, ProfilesGuiState};
use crate::worker::{spawn_vision_worker, WorkerSharedInput};

/// Application navigation tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    /// Real-time live camera feed with authentic ONNX model overlays.
    LiveInspection,
    /// Apple FaceID-style guided multi-step enrollment flow.
    GuidedEnrollment,
    /// Management of enrolled biometric templates (view, match test, delete).
    Profiles,
}

/// The primary SOOS GUI application state.
pub struct SoosApp {
    current_tab: AppTab,
    store: Arc<BiometricStore>,
    _camera: Arc<dyn CameraManager>,
    _pipeline: Arc<VisionPipeline>,
    latest_frame_slot: Arc<ArcSwapOption<LatestFrameData>>,
    worker_input: Arc<WorkerSharedInput>,
    worker_running: Arc<AtomicBool>,
    _worker_thread: Option<std::thread::JoinHandle<()>>,

    // Visual display state
    video_texture: Option<egui::TextureHandle>,
    aligned_crop_texture: Option<egui::TextureHandle>,
    last_rendered_seq: u64,

    // Toggles for live overlays
    show_bbox: bool,
    show_landmarks: bool,
    show_pad_crop: bool,
    show_pose_stats: bool,
    show_crop_inset: bool,

    // Sub-states
    enrollment: EnrollmentGuiState,
    profiles: ProfilesGuiState,
}

impl SoosApp {
    /// Creates and initializes a new `SoosApp` with background inference worker.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store: Arc<BiometricStore>,
        camera: Arc<dyn CameraManager>,
        pipeline: Arc<VisionPipeline>,
    ) -> Self {
        let latest_frame_slot = Arc::new(ArcSwapOption::empty());
        let worker_input = Arc::new(WorkerSharedInput::default());
        let worker_running = Arc::new(AtomicBool::new(true));

        let worker_thread = spawn_vision_worker(
            Arc::clone(&camera),
            Arc::clone(&pipeline),
            Arc::clone(&latest_frame_slot),
            Arc::clone(&worker_input),
            Arc::clone(&worker_running),
            cc.egui_ctx.clone(),
        )
        .map_err(|e| {
            tracing::error!("Failed to spawn vision worker thread: {e}");
            e
        })
        .ok();

        let mut app = Self {
            current_tab: AppTab::LiveInspection,
            store,
            _camera: camera,
            _pipeline: pipeline,
            latest_frame_slot,
            worker_input,
            worker_running,
            _worker_thread: worker_thread,
            video_texture: None,
            aligned_crop_texture: None,
            last_rendered_seq: u64::MAX,
            show_bbox: true,
            show_landmarks: true,
            show_pad_crop: true,
            show_pose_stats: true,
            show_crop_inset: true,
            enrollment: EnrollmentGuiState::default(),
            profiles: ProfilesGuiState::default(),
        };

        app.refresh_profiles();
        app
    }

    /// Reloads the enrolled profiles list from the biometric store.
    pub fn refresh_profiles(&mut self) {
        let uids = self.store.list_enrolled().unwrap_or_default();
        let mut summaries = Vec::with_capacity(uids.len());

        for uid in uids {
            if let Ok(Some(template)) = self.store.get(uid) {
                let username = match nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)) {
                    Ok(Some(u)) => u.name,
                    _ => uid.to_string(),
                };

                summaries.push(EnrolledUserSummary {
                    uid,
                    username,
                    model_id: template.model_id,
                    model_version: template.model_version,
                    enrollment_timestamp: template.enrollment_timestamp,
                    embedding_dim: template.embedding_dim,
                });
            }
        }

        summaries.sort_by_key(|s| s.uid);
        self.profiles.profiles = summaries;
    }

    /// Renders top navigation bar.
    fn render_header(&mut self, ui: &mut egui::Ui, latest_frame: Option<&LatestFrameData>) {
        ui.horizontal(|ui| {
            ui.heading("SOOS — Biometric PAM Manager");
            ui.separator();

            ui.selectable_value(
                &mut self.current_tab,
                AppTab::LiveInspection,
                "🔍 Live Model Diagnostics",
            );
            ui.selectable_value(
                &mut self.current_tab,
                AppTab::GuidedEnrollment,
                "👤 Guided Enrollment",
            );
            ui.selectable_value(
                &mut self.current_tab,
                AppTab::Profiles,
                "📁 Biometric Profiles",
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(frame) = latest_frame {
                    ui.label(format!("{:.0} FPS", frame.fps));
                    ui.separator();
                    ui.label(format!("{:.1} ms latency", frame.pipeline_latency_ms));
                    ui.separator();
                    ui.label(format!("{}×{}", frame.width, frame.height));
                } else {
                    ui.label("Waiting for camera feed...");
                }
            });
        });
        ui.separator();
    }

    /// Renders Tab 1: Live camera view with authentic ONNX model overlays.
    fn render_live_inspection(&mut self, ui: &mut egui::Ui, frame: &LatestFrameData) {
        ui.horizontal(|ui| {
            // Main camera view area (75% width)
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.show_bbox, "Bounding Box (SCRFD)");
                    ui.checkbox(&mut self.show_landmarks, "5 Landmarks (SCRFD)");
                    ui.checkbox(&mut self.show_pad_crop, "PAD Area (MiniFASNetV2)");
                    ui.checkbox(&mut self.show_crop_inset, "Aligned Crop (112×112)");
                    ui.checkbox(&mut self.show_pose_stats, "Pose & Angles");
                });

                if let Some(texture) = &self.video_texture {
                    let avail_size = ui.available_size();
                    let aspect_ratio = (frame.width as f32) / (frame.height as f32);
                    let target_w = avail_size.x.min(avail_size.y * aspect_ratio);
                    let target_h = target_w / aspect_ratio;
                    let target_size = Vec2::new(target_w, target_h);

                    let (response, painter) =
                        ui.allocate_painter(target_size, egui::Sense::hover());
                    let rect = response.rect;

                    // 1. Paint live camera image
                    painter.image(
                        texture.id(),
                        rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );

                    let to_screen = |x: f32, y: f32| -> Pos2 {
                        Pos2::new(
                            rect.min.x + (x / frame.width as f32) * rect.width(),
                            rect.min.y + (y / frame.height as f32) * rect.height(),
                        )
                    };

                    // 2. Paint authentic ONNX detections
                    for det in &frame.detections {
                        let is_live = frame
                            .pad_result
                            .as_ref()
                            .map(|p| p.is_live && p.score >= 0.80)
                            .unwrap_or(false);

                        let box_color = if is_live {
                            Color32::from_rgb(0, 230, 118) // Bright Green (Live)
                        } else {
                            Color32::from_rgb(255, 23, 68) // Red (Spoof / Unverified)
                        };

                        if self.show_bbox {
                            let p1 = to_screen(det.box_.x1, det.box_.y1);
                            let p2 = to_screen(det.box_.x2, det.box_.y2);
                            painter.rect_stroke(
                                Rect::from_two_pos(p1, p2),
                                4.0,
                                Stroke::new(2.5, box_color),
                                egui::StrokeKind::Outside,
                            );

                            // Confidence badge
                            let label = format!("Face: {:.1}%", det.score * 100.0);
                            painter.text(
                                Pos2::new(p1.x + 4.0, p1.y + 14.0),
                                egui::Align2::LEFT_BOTTOM,
                                label,
                                egui::FontId::proportional(14.0),
                                box_color,
                            );
                        }

                        // 3. Paint authentic 5-point landmarks
                        if self.show_landmarks {
                            if let Some(lmk) = &det.landmarks {
                                let pts = [
                                    (lmk.left_eye, Color32::from_rgb(0, 229, 255)), // Cyan: Left eye
                                    (lmk.right_eye, Color32::from_rgb(0, 229, 255)), // Cyan: Right eye
                                    (lmk.nose, Color32::from_rgb(255, 234, 0)),      // Yellow: Nose
                                    (lmk.mouth_left, Color32::from_rgb(255, 145, 0)), // Orange: Mouth L
                                    (lmk.mouth_right, Color32::from_rgb(255, 145, 0)), // Orange: Mouth R
                                ];

                                for (pt, color) in pts {
                                    painter.circle_filled(to_screen(pt.x, pt.y), 4.5, color);
                                    painter.circle_stroke(
                                        to_screen(pt.x, pt.y),
                                        5.5,
                                        Stroke::new(1.0, Color32::BLACK),
                                    );
                                }

                                // Eye axis
                                painter.line_segment(
                                    [
                                        to_screen(lmk.left_eye.x, lmk.left_eye.y),
                                        to_screen(lmk.right_eye.x, lmk.right_eye.y),
                                    ],
                                    Stroke::new(1.5, Color32::LIGHT_BLUE),
                                );
                            }
                        }

                        // 4. Paint PAD 2.7x expanded bounding box context crop
                        if self.show_pad_crop {
                            let pad_box = soos_vision::crop::expand_bbox_for_pad(
                                &det.box_,
                                2.7,
                                frame.width,
                                frame.height,
                            );
                            let pad_p1 = to_screen(pad_box.x1, pad_box.y1);
                            let pad_p2 = to_screen(pad_box.x2, pad_box.y2);
                            painter.rect_stroke(
                                Rect::from_two_pos(pad_p1, pad_p2),
                                2.0,
                                Stroke::new(
                                    1.0,
                                    Color32::from_rgba_premultiplied(200, 200, 200, 100),
                                ),
                                egui::StrokeKind::Outside,
                            );
                        }
                    }

                    // 5. Inset preview: 112x112 aligned face crop
                    if self.show_crop_inset {
                        if let Some(crop_tex) = &self.aligned_crop_texture {
                            let inset_size = Vec2::new(112.0, 112.0);
                            let inset_rect = Rect::from_min_size(
                                Pos2::new(rect.max.x - 124.0, rect.max.y - 124.0),
                                inset_size,
                            );
                            painter.rect_filled(
                                inset_rect.expand(4.0),
                                4.0,
                                Color32::from_black_alpha(180),
                            );
                            painter.image(
                                crop_tex.id(),
                                inset_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                            painter.text(
                                Pos2::new(inset_rect.min.x + 2.0, inset_rect.min.y - 4.0),
                                egui::Align2::LEFT_BOTTOM,
                                "ArcFace (112×112)",
                                egui::FontId::proportional(11.0),
                                Color32::LIGHT_GRAY,
                            );
                        }
                    }
                }
            });

            // Sidebar telemetry and controls (25% width)
            ui.separator();
            ui.vertical(|ui| {
                ui.heading("Telemetry & Analysis");
                ui.add_space(8.0);

                egui::Grid::new("telemetry_grid")
                    .num_columns(2)
                    .spacing([20.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Face Count:");
                        ui.label(format!("{}", frame.detections.len()));
                        ui.end_row();

                        ui.label("Detection Score:");
                        if let Some(det) = frame.detections.first() {
                            ui.label(format!("{:.2}%", det.score * 100.0));
                        } else {
                            ui.label("None");
                        }
                        ui.end_row();

                        ui.label("Anti-Spoof (PAD):");
                        if let Some(pad) = &frame.pad_result {
                            let status = if pad.is_live && pad.score >= 0.80 {
                                "LIVE ✓"
                            } else {
                                "SPOOF ✗"
                            };
                            let color = if pad.is_live {
                                Color32::GREEN
                            } else {
                                Color32::RED
                            };
                            ui.colored_label(color, format!("{status} ({:.2})", pad.score));
                        } else {
                            ui.label("Waiting...");
                        }
                        ui.end_row();

                        if let Some(pose) = &frame.pose {
                            ui.label("Head Yaw:");
                            ui.label(format!("{:.1}°", pose.yaw));
                            ui.end_row();

                            ui.label("Head Pitch:");
                            ui.label(format!("{:.1}°", pose.pitch));
                            ui.end_row();

                            ui.label("Head Roll:");
                            ui.label(format!("{:.1}°", pose.roll));
                            ui.end_row();
                        }

                        ui.label("Inference Time:");
                        ui.label(format!("{:.1} ms", frame.pipeline_latency_ms));
                        ui.end_row();
                    });

                ui.add_space(16.0);
                ui.separator();
                ui.heading("Live 1-to-1 Match Test");
                ui.label("Test real-time biometric PAM unlocking against enrolled profiles:");

                if self.profiles.profiles.is_empty() {
                    ui.label("No enrolled profiles available.");
                } else {
                    egui::ComboBox::from_label("Profile")
                        .selected_text(
                            self.profiles
                                .selected_uid
                                .map(|u| format!("UID {u}"))
                                .unwrap_or_else(|| "Select Profile...".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for p in &self.profiles.profiles {
                                if ui
                                    .selectable_label(
                                        self.profiles.selected_uid == Some(p.uid),
                                        format!("{} (UID {})", p.username, p.uid),
                                    )
                                    .clicked()
                                {
                                    self.profiles.selected_uid = Some(p.uid);
                                    if let Ok(Some(template)) = self.store.get(p.uid) {
                                        if let Ok(mut ref_guard) =
                                            self.worker_input.match_reference.lock()
                                        {
                                            *ref_guard =
                                                Some(template.embedding.as_slice().to_vec());
                                        }
                                    }
                                }
                            }
                        });

                    if let Ok(score_guard) = self.worker_input.live_match_score.lock() {
                        if let Some(score) = *score_guard {
                            let match_threshold = 0.45f32;
                            let is_match = score >= match_threshold;
                            ui.add_space(8.0);
                            ui.label(format!("Cosine Match Score: {:.4}", score));

                            let bar_color = if is_match {
                                Color32::GREEN
                            } else {
                                Color32::RED
                            };
                            let progress = (score / 1.0).clamp(0.0, 1.0);
                            ui.add(
                                egui::ProgressBar::new(progress)
                                    .fill(bar_color)
                                    .text(format!("{:.1}%", score * 100.0)),
                            );

                            if is_match {
                                ui.colored_label(Color32::GREEN, "VERDICT: PAM_SUCCESS (UNLOCKED)");
                            } else {
                                ui.colored_label(Color32::RED, "VERDICT: PAM_IGNORE (LOCKED)");
                            }
                        }
                    }
                }
            });
        });
    }

    /// Renders Tab 2: Apple FaceID-style guided multi-step enrollment flow.
    fn render_guided_enrollment(&mut self, ui: &mut egui::Ui, frame: &LatestFrameData) {
        ui.horizontal(|ui| {
            // Main guided video panel (65% width)
            ui.vertical(|ui| {
                if let Some(texture) = &self.video_texture {
                    let avail_size = ui.available_size();
                    let aspect_ratio = (frame.width as f32) / (frame.height as f32);
                    let target_w = (avail_size.x * 0.95).min(avail_size.y * aspect_ratio);
                    let target_h = target_w / aspect_ratio;
                    let target_size = Vec2::new(target_w, target_h);

                    let (response, painter) =
                        ui.allocate_painter(target_size, egui::Sense::hover());
                    let rect = response.rect;

                    painter.image(
                        texture.id(),
                        rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );

                    // Guided enrollment oval reticle
                    let center = rect.center();
                    let oval_w = rect.width() * 0.40;
                    let oval_h = rect.height() * 0.60;
                    let reticle_rect = Rect::from_center_size(center, Vec2::new(oval_w, oval_h));

                    let reticle_stroke = if self.enrollment.is_active {
                        Stroke::new(4.0, Color32::from_rgb(0, 229, 255))
                    } else {
                        Stroke::new(2.0, Color32::from_gray(120))
                    };
                    painter.rect_stroke(reticle_rect, 120.0, reticle_stroke, egui::StrokeKind::Outside);

                    // Draw guidance arrows if active
                    if let Some(fb) = &self.enrollment.last_feedback {
                        match fb {
                            EnrollmentStepFeedback::PromptTurnLeft => {
                                painter.arrow(
                                    center,
                                    Vec2::new(-80.0, 0.0),
                                    Stroke::new(6.0, Color32::YELLOW),
                                );
                            }
                            EnrollmentStepFeedback::PromptTurnRight => {
                                painter.arrow(
                                    center,
                                    Vec2::new(80.0, 0.0),
                                    Stroke::new(6.0, Color32::YELLOW),
                                );
                            }
                            EnrollmentStepFeedback::PromptTiltUp => {
                                painter.arrow(
                                    center,
                                    Vec2::new(0.0, -70.0),
                                    Stroke::new(6.0, Color32::YELLOW),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            });

            // Guided enrollment guidance cards and controls (35% width)
            ui.separator();
            ui.vertical(|ui| {
                ui.heading("Guided Multi-Angle Enrollment");
                ui.label(
                    "Captures high-quality embeddings across multiple head angles (Center, Left, Right, Up) \
                     to optimize real-time unlock accuracy.",
                );
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label("Username:");
                    ui.text_edit_singleline(&mut self.enrollment.target_username);
                });

                ui.horizontal(|ui| {
                    ui.label("Target UID:");
                    ui.add(egui::DragValue::new(&mut self.enrollment.target_uid));
                });

                ui.add_space(10.0);

                // Progress meter
                let (progress, current_step) = if let Ok(session_guard) =
                    self.worker_input.enrollment_session.lock()
                {
                    if let Some(session) = session_guard.as_ref() {
                        (session.progress_percent(), session.current_step())
                    } else {
                        (0.0, EnrollmentStep::Frontal)
                    }
                } else {
                    (0.0, EnrollmentStep::Frontal)
                };

                ui.label(format!("Enrollment Progress: {:.0}%", progress));
                ui.add(
                    egui::ProgressBar::new(progress / 100.0)
                        .show_percentage()
                        .animate(self.enrollment.is_active),
                );

                ui.add_space(12.0);

                // Step status list
                let render_step = |ui: &mut egui::Ui, name: &str, step: EnrollmentStep| {
                    let (status, color) = if current_step == EnrollmentStep::Completed {
                        ("✓ Done", Color32::GREEN)
                    } else if current_step == step {
                        ("▶ In Progress", Color32::from_rgb(0, 229, 255))
                    } else if (current_step as usize) > (step as usize) {
                        ("✓ Done", Color32::GREEN)
                    } else {
                        ("○ Waiting", Color32::GRAY)
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(color, status);
                        ui.label(name);
                    });
                };

                render_step(ui, "1. Frontal Pose (Look straight)", EnrollmentStep::Frontal);
                render_step(ui, "2. Turn Left (Slowly turn head left)", EnrollmentStep::TurnLeft);
                render_step(ui, "3. Turn Right (Slowly turn head right)", EnrollmentStep::TurnRight);
                render_step(ui, "4. Tilt Up (Slowly tilt head up)", EnrollmentStep::TiltUp);

                ui.add_space(16.0);

                // Live dynamic guidance banner
                if let Ok(fb_guard) = self.worker_input.enrollment_feedback.lock() {
                    if let Some(fb) = fb_guard.as_ref() {
                        self.enrollment.last_feedback = Some(fb.clone());
                    }
                }

                if let Some(fb) = &self.enrollment.last_feedback {
                    let (msg, color) = match fb {
                        EnrollmentStepFeedback::PromptCenterFace => (
                            "Please center your face inside the target frame.",
                            Color32::YELLOW,
                        ),
                        EnrollmentStepFeedback::PromptTurnLeft => {
                            ("Slowly turn your head slightly to the left.", Color32::from_rgb(0, 229, 255))
                        }
                        EnrollmentStepFeedback::PromptTurnRight => {
                            ("Slowly turn your head slightly to the right.", Color32::from_rgb(0, 229, 255))
                        }
                        EnrollmentStepFeedback::PromptTiltUp => {
                            ("Slowly tilt your head slightly upward.", Color32::from_rgb(0, 229, 255))
                        }
                        EnrollmentStepFeedback::PromptHoldStill => {
                            ("Hold still, capturing high-quality sample...", Color32::WHITE)
                        }
                        EnrollmentStepFeedback::SpoofDetected => (
                            "Anti-spoof check: Live face required.",
                            Color32::RED,
                        ),
                        EnrollmentStepFeedback::SampleAccepted { .. } => (
                            "Sample recorded!",
                            Color32::GREEN,
                        ),
                        EnrollmentStepFeedback::StepCompleted { .. } => (
                            "Step complete! Moving to next angle...",
                            Color32::GREEN,
                        ),
                        EnrollmentStepFeedback::AllStepsCompleted => (
                            "All angles captured! Composite template generated.",
                            Color32::GREEN,
                        ),
                    };

                    ui.group(|ui| {
                        ui.colored_label(color, msg);
                    });
                }

                ui.add_space(16.0);

                // Action controls
                ui.horizontal(|ui| {
                    if !self.enrollment.is_active {
                        if ui.button("▶ Start Guided Enrollment").clicked() {
                            self.enrollment.is_active = true;
                            if let Ok(mut session_guard) =
                                self.worker_input.enrollment_session.lock()
                            {
                                *session_guard = Some(GuidedEnrollmentSession::new(4));
                            }
                        }
                    } else if ui.button("⏹ Cancel").clicked() {
                        self.enrollment.is_active = false;
                        if let Ok(mut session_guard) =
                            self.worker_input.enrollment_session.lock()
                        {
                            *session_guard = None;
                        }
                    }

                    if current_step == EnrollmentStep::Completed
                        && ui
                            .button("💾 Save & Encrypt Biometric Template")
                            .clicked()
                    {
                        let fused_opt = if let Ok(session_guard) =
                            self.worker_input.enrollment_session.lock()
                        {
                            session_guard
                                .as_ref()
                                .and_then(|session| session.compute_composite_embedding().ok())
                        } else {
                            None
                        };

                        if let Some(fused_embedding) = fused_opt {
                            let timestamp = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();

                            let res = BiometricTemplate::new(
                                self.enrollment.target_uid,
                                "arcface_w600k_mbf".to_string(),
                                "2.0.0".to_string(),
                                timestamp,
                                Zeroizing::new(fused_embedding),
                            );

                            match res {
                                Ok(template) => {
                                    if let Err(e) = self.store.enroll(&template) {
                                        self.enrollment.status_message = Some((
                                            format!("Failed to save template: {e}"),
                                            true,
                                        ));
                                    } else {
                                        self.enrollment.status_message = Some((
                                            format!(
                                                "User {} enrolled successfully with high quality!",
                                                self.enrollment.target_username
                                            ),
                                            false,
                                        ));
                                        self.refresh_profiles();
                                        self.enrollment.is_active = false;
                                    }
                                }
                                Err(e) => {
                                    self.enrollment.status_message = Some((
                                        format!("Template creation error: {e}"),
                                        true,
                                    ));
                                }
                            }
                        }
                    }
                });

                if let Some((msg, is_err)) = &self.enrollment.status_message {
                    ui.add_space(8.0);
                    let color = if *is_err { Color32::RED } else { Color32::GREEN };
                    ui.colored_label(color, msg);
                }
            });
        });
    }

    /// Renders Tab 3: Enrolled profiles and template deletion.
    fn render_profiles(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Enrolled Biometric Profiles");
            if ui.button("🔄 Refresh").clicked() {
                self.refresh_profiles();
            }
        });
        ui.label(
            "Biometric templates stored securely on disk, encrypted with AES-256-GCM. \
             Deleting a template performs anti-forensic cryptographic shredding.",
        );
        ui.add_space(10.0);

        if self.profiles.profiles.is_empty() {
            ui.label("No biometric templates currently enrolled on this system.");
        } else {
            egui::Grid::new("profiles_table")
                .striped(true)
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    ui.strong("UID");
                    ui.strong("Username");
                    ui.strong("Model ID");
                    ui.strong("Version");
                    ui.strong("Timestamp");
                    ui.strong("Dim");
                    ui.strong("Actions");
                    ui.end_row();

                    let mut uid_to_delete = None;

                    for p in &self.profiles.profiles {
                        ui.label(format!("{}", p.uid));
                        ui.label(&p.username);
                        ui.label(&p.model_id);
                        ui.label(&p.model_version);
                        ui.label(format!("{}", p.enrollment_timestamp));
                        ui.label(format!("{}", p.embedding_dim));

                        ui.horizontal(|ui| {
                            if ui.button("🗑 Delete & Shred").clicked() {
                                uid_to_delete = Some(p.uid);
                            }
                        });
                        ui.end_row();
                    }

                    if let Some(uid) = uid_to_delete {
                        self.profiles.confirm_delete_uid = Some(uid);
                    }
                });

            // Confirmation modal
            if let Some(uid) = self.profiles.confirm_delete_uid {
                egui::Window::new("Confirm Deletion")
                    .collapsible(false)
                    .resizable(false)
                    .show(ui.ctx(), |ui| {
                        ui.label(format!(
                            "Are you sure you want to securely shred and delete the biometric template for UID {uid}?"
                        ));
                        ui.label("This operation cannot be undone.");
                        ui.horizontal(|ui| {
                            if ui.button("Yes, Shred Template").clicked() {
                                if let Err(e) = self.store.delete(uid) {
                                    self.profiles.status_message = Some((
                                        format!("Failed to delete template: {e}"),
                                        true,
                                    ));
                                } else {
                                    self.profiles.status_message = Some((
                                        format!("Template for UID {uid} shredded and removed."),
                                        false,
                                    ));
                                    self.refresh_profiles();
                                }
                                self.profiles.confirm_delete_uid = None;
                            }
                            if ui.button("Cancel").clicked() {
                                self.profiles.confirm_delete_uid = None;
                            }
                        });
                    });
            }
        }

        if let Some((msg, is_err)) = &self.profiles.status_message {
            ui.add_space(8.0);
            let color = if *is_err {
                Color32::RED
            } else {
                Color32::GREEN
            };
            ui.colored_label(color, msg);
        }
    }
}

impl eframe::App for SoosApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Read latest video frame from worker
        let latest = self.latest_frame_slot.load();
        if let Some(frame_data) = latest.as_ref() {
            if frame_data.sequence != self.last_rendered_seq {
                self.last_rendered_seq = frame_data.sequence;

                // Update main camera video texture
                let color_img = egui::ColorImage::from_rgb(
                    [frame_data.width as usize, frame_data.height as usize],
                    &frame_data.rgb,
                );
                self.video_texture =
                    Some(ctx.load_texture("video_frame", color_img, egui::TextureOptions::LINEAR));

                // Update 112x112 aligned face crop texture if available
                if let Some(crop_rgb) = &frame_data.aligned_crop {
                    let crop_img = egui::ColorImage::from_rgb([112, 112], crop_rgb);
                    self.aligned_crop_texture = Some(ctx.load_texture(
                        "aligned_crop",
                        crop_img,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
        }

        let frame_ref = latest.as_deref();
        self.render_header(ui, frame_ref);

        if let Some(frame) = frame_ref {
            match self.current_tab {
                AppTab::LiveInspection => self.render_live_inspection(ui, frame),
                AppTab::GuidedEnrollment => self.render_guided_enrollment(ui, frame),
                AppTab::Profiles => self.render_profiles(ui),
            }
        } else {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.spinner();
                ui.heading("Connecting to camera and initializing models...");
            });
        }
    }
}

impl Drop for SoosApp {
    fn drop(&mut self) {
        self.worker_running.store(false, Ordering::Release);
    }
}

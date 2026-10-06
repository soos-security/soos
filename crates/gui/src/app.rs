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
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwapOption;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use soos_biometric_store::BiometricTemplate;
use soos_camera_v4l::{CameraManager, CameraStatus};
use soos_enrollment_cli::guided_enrollment::{EnrollmentStep, EnrollmentStepFeedback};
use soos_enrollment_cli::service::EnrolledUserSummary;
use soos_vision::VisionPipeline;

use crate::brand;
use crate::camera_source::{
    CameraSourceBackend, CameraSourceSupervisor, HandoverExecutor, HandoverSlot, SwitchableCamera,
};
use crate::camera_status::{camera_status_banner, render_status_banner};
use crate::daemon_control::{DaemonMonitor, DaemonState, SystemctlProbe, DAEMON_POLL_INTERVAL};
use crate::header;
use crate::privileged::{PkexecExecutor, PrivilegedAction, PrivilegedOutcome, TaskRunner};
use crate::state::{EnrollmentGuiState, LatestFrameData, ProfilesGuiState};
use crate::store_mode::GuiStore;
use crate::store_tasks::{StoreTask, StoreTaskOutcome, StoreTaskRunner};
use crate::theme;
use crate::widgets::{self, BannerKind};
use crate::worker::{spawn_vision_worker, GuiEnrollmentFeedback, WorkerSharedInput};

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
    /// Store backing this session (system, Polkit-only or explicit developer store).
    store: GuiStore,
    /// Persistent developer-mode warning shown on every frame (GitHub #156).
    store_banner: Option<String>,
    camera: Arc<dyn CameraManager>,
    pipeline: Arc<VisionPipeline>,
    /// Why the selected profile cannot be live-matched (template of another embedding model,
    /// GitHub #278), shown instead of a score.
    match_reference_note: Option<String>,
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

    /// Camera ownership notice shown instead of the feed (GitHub #150).
    camera_notice: Option<String>,

    // Background daemon-state polling and privileged (pkexec) operations (GitHub #154)
    daemon_monitor: Option<DaemonMonitor>,
    tasks: TaskRunner,
    /// Direct-mode template store mutations, off the UI thread (GitHub #291); `None` in
    /// Polkit mode.
    store_tasks: Option<StoreTaskRunner>,
    /// Username shown in the success message of the store enrollment in flight, captured when
    /// the task is submitted (the username field may be edited while the save runs).
    pending_store_username: Option<String>,
    daemon_message: Option<(String, bool)>,
    last_camera_status: Option<CameraStatus>,

    // Runtime camera-source switching driven by the daemon state (GitHub #154 / #150)
    camera_source: Option<CameraSourceSupervisor>,
    switchable_camera: Option<Arc<SwitchableCamera>>,
    handover_slot: HandoverSlot,
    last_source_generation: u64,
}

impl SoosApp {
    /// Creates and initializes a new `SoosApp` with background inference worker.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store: GuiStore,
        camera: Arc<dyn CameraManager>,
        pipeline: Arc<VisionPipeline>,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        let store_banner = store.banner();
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

        let daemon_monitor = DaemonMonitor::spawn(Arc::new(SystemctlProbe), DAEMON_POLL_INTERVAL)
            .map_err(|e| {
                tracing::error!("Failed to spawn daemon status monitor thread: {e}");
                e
            })
            .ok();
        let store_repaint_ctx = cc.egui_ctx.clone();
        let store_tasks = store.local().map(|local| {
            StoreTaskRunner::new(
                Arc::clone(local),
                Arc::new(move || store_repaint_ctx.request_repaint()),
            )
        });
        let repaint_ctx = cc.egui_ctx.clone();
        // `ResumeDaemon` first releases any direct V4L2 manager (on the privileged worker
        // thread) so the starting daemon never meets EBUSY because of the GUI.
        let handover_slot: HandoverSlot = Arc::new(OnceLock::new());
        let tasks = TaskRunner::new(
            Arc::new(HandoverExecutor::new(
                Arc::new(PkexecExecutor),
                Arc::clone(&handover_slot),
            )),
            Arc::new(move || repaint_ctx.request_repaint()),
        );

        let mut app = Self {
            current_tab: AppTab::LiveInspection,
            store,
            store_banner,
            camera,
            pipeline,
            match_reference_note: None,
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
            camera_notice: None,
            daemon_monitor,
            tasks,
            store_tasks,
            pending_store_username: None,
            daemon_message: None,
            last_camera_status: None,
            camera_source: None,
            switchable_camera: None,
            handover_slot,
            last_source_generation: 0,
        };

        app.refresh_profiles();
        app
    }

    /// Sets the camera ownership notice rendered in place of the camera feed (GitHub #150).
    pub fn set_camera_notice(&mut self, notice: Option<String>) {
        self.camera_notice = notice;
    }

    /// Hands the camera to the runtime source supervisor (GitHub #154 / #150).
    ///
    /// `camera` must be the [`SwitchableCamera`] this app was created with. The supervisor
    /// follows the background `DaemonMonitor`: daemon active => daemon IPC preview (or a
    /// blocked notice), daemon paused => direct V4L2 through the shared resolver. All probing
    /// and device release happen on the supervisor thread.
    pub fn attach_camera_source(
        &mut self,
        camera: Arc<SwitchableCamera>,
        backend: Arc<dyn CameraSourceBackend>,
    ) {
        let Some(monitor) = self.daemon_monitor.as_ref() else {
            tracing::error!("No daemon status monitor: the camera stays disabled (fail-closed)");
            self.camera_notice = Some(
                "soos-gui cannot determine whether soos-daemon owns the camera, so it will not \
                 open the camera. Restart soos-gui."
                    .to_string(),
            );
            return;
        };
        match CameraSourceSupervisor::spawn(
            Arc::clone(&camera),
            backend,
            Arc::new(monitor.state_reader()),
        ) {
            Ok(supervisor) => {
                let _ = self.handover_slot.set(supervisor.handover());
                self.camera_source = Some(supervisor);
                self.switchable_camera = Some(camera);
            }
            Err(e) => {
                tracing::error!("Failed to spawn camera source supervisor thread: {e}");
                self.camera_notice =
                    Some(format!("soos-gui could not start its camera source: {e}"));
            }
        }
    }

    /// Notice rendered instead of the camera feed, if any.
    fn current_camera_notice(&self) -> Option<String> {
        self.switchable_camera
            .as_ref()
            .and_then(|camera| camera.notice())
            .or_else(|| self.camera_notice.clone())
    }

    /// Drops the displayed frame when the camera source was switched (no stale feed).
    fn sync_camera_source_generation(&mut self) {
        let Some(camera) = self.switchable_camera.as_ref() else {
            return;
        };
        let generation = camera.generation();
        if generation != self.last_source_generation {
            self.last_source_generation = generation;
            self.latest_frame_slot.store(None);
            self.video_texture = None;
            self.aligned_crop_texture = None;
            self.last_rendered_seq = u64::MAX;
        }
    }

    /// Reloads the enrolled profiles list.
    ///
    /// Unprivileged sessions query the root-owned system store through `pkexec soos-enroll
    /// list` on a background thread; the result arrives in [`Self::handle_task_outcomes`].
    pub fn refresh_profiles(&mut self) {
        if self.store.uses_polkit() {
            if let Err(e) = self.tasks.submit(PrivilegedAction::ListProfiles) {
                self.profiles.status_message = Some((format!("Cannot load profiles: {e}"), true));
                self.load_local_profiles();
            }
            return;
        }
        self.load_local_profiles();
    }

    /// Lists the templates readable from the GUI's own biometric store.
    fn load_local_profiles(&mut self) {
        // Polkit mode keeps no local store: nothing to list in-process (GitHub #156).
        let Some(store) = self.store.local().map(Arc::clone) else {
            self.profiles.profiles.clear();
            return;
        };
        let uids = store.list_enrolled().unwrap_or_default();
        let mut summaries = Vec::with_capacity(uids.len());

        for uid in uids {
            if let Ok(Some(template)) = store.get_metadata(uid) {
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

    /// Submits a privileged action, reporting a busy runner instead of blocking.
    fn submit_privileged(&mut self, action: PrivilegedAction) -> bool {
        match self.tasks.submit(action) {
            Ok(()) => true,
            Err(e) => {
                self.daemon_message = Some((format!("{e}"), true));
                false
            }
        }
    }

    /// Applies the outcomes of finished background privileged operations (non-blocking).
    fn handle_task_outcomes(&mut self) {
        for outcome in self.tasks.poll() {
            match outcome {
                PrivilegedOutcome::DaemonPaused(result)
                | PrivilegedOutcome::DaemonResumed(result) => {
                    if let Some(monitor) = &self.daemon_monitor {
                        monitor.request_refresh();
                    }
                    self.daemon_message = result.err().map(|e| {
                        tracing::warn!("soos-daemon service control failed: {e}");
                        (e, true)
                    });
                }
                PrivilegedOutcome::ProfilesListed(Ok(summaries)) => {
                    self.profiles.profiles = summaries;
                }
                PrivilegedOutcome::ProfilesListed(Err(e)) => {
                    tracing::warn!("Loading system profiles failed: {e}");
                    self.profiles.status_message = Some((e, true));
                    self.load_local_profiles();
                }
                PrivilegedOutcome::TemplateImported { uid, result } => match result {
                    Ok(()) => {
                        self.enrollment.status_message = Some((
                            format!(
                                "User {} (UID {uid}) enrolled successfully into system store! Ready for PAM unlock.",
                                self.enrollment.target_username
                            ),
                            false,
                        ));
                        self.enrollment.is_active = false;
                        self.refresh_profiles();
                    }
                    Err(e) => {
                        self.enrollment.status_message = Some((
                            format!("System store import failed: {e}. Please grant Polkit authorization."),
                            true,
                        ));
                    }
                },
                PrivilegedOutcome::TemplateDeleted { uid, result } => match result {
                    Ok(()) => {
                        self.profiles.status_message = Some((
                            format!("Template for UID {uid} removed from the system store."),
                            false,
                        ));
                        self.refresh_profiles();
                    }
                    Err(e) => {
                        self.profiles.status_message = Some((
                            format!("Failed to delete template for UID {uid} from system store via Polkit: {e}"),
                            true,
                        ));
                    }
                },
            }
        }
    }

    /// Applies the outcomes of finished direct-mode store mutations (non-blocking, GitHub #291).
    fn handle_store_task_outcomes(&mut self) {
        let Some(runner) = self.store_tasks.as_mut() else {
            return;
        };
        for outcome in runner.poll() {
            match outcome {
                StoreTaskOutcome::Enrolled { uid, result } => match result {
                    Ok(()) => {
                        let target = if matches!(self.store, GuiStore::Developer { .. }) {
                            "the developer store (not used by PAM)"
                        } else {
                            "the system store"
                        };
                        let message = match self.pending_store_username.take() {
                            Some(username) => format!(
                                "User {username} (UID {uid}) enrolled successfully into {target}."
                            ),
                            None => format!("UID {uid} enrolled successfully into {target}."),
                        };
                        self.enrollment.status_message = Some((message, false));
                        self.enrollment.is_active = false;
                        self.refresh_profiles();
                    }
                    Err(e) => {
                        self.pending_store_username = None;
                        self.enrollment.status_message =
                            Some((format!("Failed to save the template: {e}"), true));
                    }
                },
                StoreTaskOutcome::Deleted { uid, result } => match result {
                    Ok(_) => {
                        self.profiles.status_message =
                            Some((format!("Template for UID {uid} removed."), false));
                        self.refresh_profiles();
                    }
                    Err(e) => {
                        self.profiles.status_message =
                            Some((format!("Failed to delete template: {e}"), true));
                    }
                },
            }
        }
    }

    /// Submits a direct-mode store mutation to the background store worker (GitHub #291).
    fn submit_store_task(&mut self, task: StoreTask) -> Result<(), String> {
        match self.store_tasks.as_mut() {
            Some(runner) => runner.submit(task).map_err(|e| e.to_string()),
            None => Err("no biometric store is available".to_string()),
        }
    }

    /// Logs camera status transitions (kind and failure count only, never frame data).
    fn track_camera_status(&mut self, status: CameraStatus) {
        if self.last_camera_status == Some(status) {
            return;
        }
        if let CameraStatus::Error { kind, failures } = status {
            tracing::warn!(%kind, failures, "camera source reported an error");
        } else {
            tracing::info!(?status, "camera source status changed");
        }
        self.last_camera_status = Some(status);
    }

    /// Renders the brand chrome (blue band, pale body), the wordmark, the tab bar, the daemon
    /// panel and the frame statistics. Returns the body content rectangle for the pages.
    fn render_header(&mut self, ui: &mut egui::Ui, latest_frame: Option<&LatestFrameData>) -> Rect {
        let full = ui.max_rect();
        let metrics = theme::Metrics::for_width(full.width());
        let band_h = theme::HEADER_H;
        header::paint_chrome(ui.painter(), full);

        // Wordmark, 18 points high, vertically centered in the band.
        let wordmark = Rect::from_min_size(
            Pos2::new(full.min.x + metrics.pad, full.min.y + (band_h - 18.0) / 2.0),
            Vec2::new(18.0 * 514.0 / 64.0, 18.0),
        );
        brand::paint_wordmark(ui, wordmark, theme::PALE);

        // Daemon panel hanging from the top edge, aligned with the right column.
        let panel = Rect::from_min_max(
            Pos2::new(full.max.x - metrics.pad - metrics.col_w, full.min.y),
            Pos2::new(full.max.x - metrics.pad, full.min.y + band_h - 3.0),
        );

        // Tabs between the wordmark and the daemon panel.
        let tabs = [
            (AppTab::LiveInspection, header::TabIcon::Search),
            (AppTab::GuidedEnrollment, header::TabIcon::Person),
            (AppTab::Profiles, header::TabIcon::Profiles),
        ];
        let tab_font = egui::FontId::proportional(theme::F_TAB);
        let label_widths: Vec<f32> = header::TAB_LABELS
            .iter()
            .map(|label| {
                ui.painter()
                    .layout_no_wrap((*label).to_owned(), tab_font.clone(), theme::INK)
                    .size()
                    .x
                    + 1.0
            })
            .collect();
        let active_index = tabs
            .iter()
            .position(|(tab, _)| *tab == self.current_tab)
            .unwrap_or(0);
        let tabs_x0 = wordmark.max.x + 34.0 * metrics.k.max(0.75);
        let layout = header::layout_tabs(&label_widths, active_index, tabs_x0, panel.min.x - 16.0);
        let spacing = if layout.iter().all(|(_, shown)| *shown)
            && layout.iter().map(|(w, _)| *w).sum::<f32>()
                >= label_widths
                    .iter()
                    .map(|w| header::tab_width(*w, header::TAB_SPACING_FULL))
                    .sum::<f32>()
        {
            header::TAB_SPACING_FULL
        } else {
            header::TAB_SPACING_TIGHT
        };
        let mut x = tabs_x0;
        let mut rects = Vec::with_capacity(tabs.len());
        for (width, show_label) in layout {
            rects.push((
                Rect::from_min_size(Pos2::new(x, full.min.y), Vec2::new(width, band_h)),
                show_label,
            ));
            x += width;
        }
        // The active tab is painted first so its concave fillets never cover a neighbor's icon.
        let order =
            std::iter::once(active_index).chain((0..tabs.len()).filter(|i| *i != active_index));
        for i in order {
            let (Some((tab, icon)), Some(label), Some((rect, show_label))) =
                (tabs.get(i), header::TAB_LABELS.get(i), rects.get(i))
            else {
                continue;
            };
            let active = self.current_tab == *tab;
            if header::tab(ui, *rect, label, *icon, active, *show_label, spacing).clicked() {
                self.current_tab = *tab;
            }
        }

        // Lock-free read of the state published by the background monitor (GitHub #154).
        let daemon_state = self
            .daemon_monitor
            .as_ref()
            .map_or(DaemonState::Unknown, DaemonMonitor::state);
        // The switch is disabled while a privileged action is pending (no stacked Polkit
        // dialogs) and submits exactly the old Pause/Resume actions.
        let switch = header::DaemonSwitch::new(daemon_state, self.tasks.is_busy());
        if let Some(action) = header::daemon_panel(ui, panel, &switch) {
            self.submit_privileged(action);
        }

        // Frame statistics (or the camera status title) in the band, left of the panel.
        let stats = if let Some(frame) = latest_frame {
            format!(
                "{:.0} FPS · {:.1} ms latency · {}×{}",
                frame.fps, frame.pipeline_latency_ms, frame.width, frame.height
            )
        } else {
            camera_status_banner(&self.camera.status()).title
        };
        let stats_galley = ui.painter().layout_no_wrap(
            stats,
            egui::FontId::proportional(theme::F_SMALL),
            theme::PALE.gamma_multiply(0.8),
        );
        let stats_right = panel.min.x - 20.0;
        if stats_right - stats_galley.size().x > x + 24.0 {
            let pos = Pos2::new(
                stats_right - stats_galley.size().x,
                full.min.y + (band_h - stats_galley.size().y) / 2.0,
            );
            ui.painter()
                .galley(pos, stats_galley, theme::PALE.gamma_multiply(0.8));
        } else {
            // Too narrow for the band text: keep it reachable as the wordmark tooltip.
            let text = stats_galley.text().to_owned();
            let _ = ui
                .interact(wordmark, ui.id().with("band_stats"), egui::Sense::hover())
                .on_hover_text(text);
        }

        theme::content_rect(full, &metrics)
    }

    /// Renders the persistent notices (developer store, daemon control result) at the top of
    /// the body.
    fn render_notices(&self, ui: &mut egui::Ui) {
        let mut shown = false;
        if let Some(banner) = &self.store_banner {
            // Developer mode is never silent (GitHub #156).
            widgets::banner(ui, BannerKind::Warning, banner);
            shown = true;
        }
        if !self.tasks.is_busy() {
            if let Some((msg, is_err)) = &self.daemon_message {
                let kind = if *is_err {
                    BannerKind::Error
                } else {
                    BannerKind::Success
                };
                widgets::banner(ui, kind, msg);
                shown = true;
            }
        }
        if shown {
            ui.add_space(theme::GAP_CARD - ui.spacing().item_spacing.y);
        }
    }

    /// Renders Tab 1: Live camera view with authentic ONNX model overlays.
    ///
    /// Brand layout: a large rounded video panel on the left (overlay toggles float inside it so
    /// they never shrink the canvas), and a right column stacking the "Telemetry & Analysis"
    /// card, a big stat tile and the decorative star tile.
    fn render_live_inspection(&mut self, ui: &mut egui::Ui, frame: &LatestFrameData) {
        let area = ui.available_rect_before_wrap();
        let metrics = theme::Metrics::for_width(ui.ctx().content_rect().width());
        let (main, column) = metrics.split_columns(area);
        ui.allocate_rect(area, egui::Sense::hover());

        // Video panel: 4:3 (frame aspect) fit, centered horizontally at the top of the main area.
        let aspect_ratio = (frame.width as f32) / (frame.height as f32);
        let rect = theme::fit_video(main, aspect_ratio);
        let painter = ui.painter_at(rect);
        let full_uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));

        if let Some(texture) = &self.video_texture {
            // 1. Paint live camera image with rounded corners
            painter.add(
                egui::epaint::RectShape::filled(
                    rect,
                    egui::CornerRadius::same(theme::R_VIDEO),
                    Color32::WHITE,
                )
                .with_texture(texture.id(), full_uv),
            );

            // Rule-of-thirds guide lines
            for t in [1.0 / 3.0, 2.0 / 3.0] {
                let x = rect.min.x + rect.width() * t;
                painter.line_segment(
                    [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
                    Stroke::new(1.0, theme::GUIDE),
                );
            }

            let to_screen = |x: f32, y: f32| -> Pos2 {
                Pos2::new(
                    rect.min.x + (x / frame.width as f32) * rect.width(),
                    rect.min.y + (y / frame.height as f32) * rect.height(),
                )
            };

            // 2. Paint authentic ONNX detections
            for det in &frame.detections {
                // Liveness color comes only from the pipeline decision (GitHub #215).
                let box_color = if frame.pad_live {
                    theme::FACE_LIVE
                } else {
                    theme::FACE_SPOOF
                };

                // PAD 2.7x expanded bounding box context crop (drawn under the face box)
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
                        Stroke::new(1.0, theme::PAD_BOX),
                        egui::StrokeKind::Outside,
                    );
                }

                if self.show_bbox {
                    let p1 = to_screen(det.box_.x1, det.box_.y1);
                    let p2 = to_screen(det.box_.x2, det.box_.y2);
                    painter.rect_stroke(
                        Rect::from_two_pos(p1, p2),
                        3.0,
                        Stroke::new(2.0, box_color),
                        egui::StrokeKind::Outside,
                    );

                    // Confidence badge
                    let label = format!("Face: {:.1}%", det.score * 100.0);
                    widgets::backed_text(
                        &painter,
                        Pos2::new(p1.x + 7.0, p1.y + 5.0),
                        egui::Align2::LEFT_TOP,
                        &label,
                        egui::FontId::proportional(theme::F_SMALL),
                        box_color,
                    );
                }

                // 3. Paint authentic 5-point landmarks
                if self.show_landmarks {
                    if let Some(lmk) = &det.landmarks {
                        // Eye axis first so the dots sit on top of it.
                        painter.line_segment(
                            [
                                to_screen(lmk.left_eye.x, lmk.left_eye.y),
                                to_screen(lmk.right_eye.x, lmk.right_eye.y),
                            ],
                            Stroke::new(1.5, theme::EYE_AXIS),
                        );
                        let pts = [
                            (lmk.left_eye, theme::EYE),
                            (lmk.right_eye, theme::EYE),
                            (lmk.nose, theme::NOSE),
                            (lmk.mouth_left, theme::MOUTH),
                            (lmk.mouth_right, theme::MOUTH),
                        ];
                        for (pt, color) in pts {
                            let center = to_screen(pt.x, pt.y);
                            painter.circle_filled(center, 4.5, color);
                            painter.circle_stroke(
                                center,
                                5.0,
                                Stroke::new(1.0, Color32::from_rgba_unmultiplied(16, 24, 32, 200)),
                            );
                        }
                    }
                }
            }

            // 4. Inset preview: 112x112 aligned face crop, bottom-right
            if self.show_crop_inset {
                if let Some(crop_tex) = &self.aligned_crop_texture {
                    let size = (rect.width() * 0.085).clamp(88.0, 112.0);
                    let inset_rect =
                        Rect::from_min_size(rect.max - Vec2::splat(6.0 + size), Vec2::splat(size));
                    painter.rect_filled(inset_rect.expand(3.0), 5.0, theme::INK);
                    painter.add(
                        egui::epaint::RectShape::filled(
                            inset_rect,
                            egui::CornerRadius::same(2),
                            Color32::WHITE,
                        )
                        .with_texture(crop_tex.id(), full_uv),
                    );
                    widgets::backed_text(
                        &painter,
                        Pos2::new(inset_rect.min.x - 1.0, inset_rect.min.y - 8.0),
                        egui::Align2::LEFT_BOTTOM,
                        "SFace (112×112)",
                        egui::FontId::proportional(10.0),
                        theme::INSET_LABEL,
                    );
                }
            }
        } else {
            painter.rect_filled(rect, theme::R_VIDEO, theme::INK);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Acquiring video stream...",
                egui::FontId::proportional(theme::F_PLACEHOLDER),
                theme::PALE.gamma_multiply(0.67),
            );
        }

        // 5. Overlay toggles float over the top-left of the video (never shrink the canvas).
        widgets::overlay_chips(
            ui,
            rect.shrink(12.0),
            "live_overlay_chips",
            &mut [
                ("Bounding Box (SCRFD)", &mut self.show_bbox),
                ("5 Landmarks (SCRFD)", &mut self.show_landmarks),
                ("PAD Area (MiniFASNetV2)", &mut self.show_pad_crop),
                ("Aligned Crop (112×112)", &mut self.show_crop_inset),
                ("Pose & Angles", &mut self.show_pose_stats),
            ],
        );

        // A match reference whose profile left the list (deleted, or a refresh that no longer
        // lists it) is cleared, so no score is computed against a template that is gone.
        if let Some(uid) = self.profiles.selected_uid {
            if !self.profiles.profiles.iter().any(|p| p.uid == uid) {
                self.profiles.selected_uid = None;
                self.match_reference_note = crate::worker::select_match_reference(
                    &self.worker_input,
                    None,
                    soos_enrollment_cli::service::MODEL_ID_EMBEDDING,
                    self.pipeline.embedding_dimension(),
                );
            }
        }

        // Right column: telemetry card, stat tile, star tile.
        let card_needed = widgets::card_needed_height(ui, "live_telemetry_card", 300.0, 0.0);
        let (card_rect, stat_rect, star_rect) =
            widgets::column_stack_card_first(column, metrics.gap_card, card_needed);
        // One non-blocking read of the live score; a poisoned lock shows no score. A score is
        // only meaningful while its profile is still listed and selected.
        let live_score = self
            .profiles
            .selected_uid
            .filter(|uid| self.profiles.profiles.iter().any(|p| p.uid == *uid))
            .and_then(|_| {
                self.worker_input
                    .live_match_score
                    .lock()
                    .ok()
                    .and_then(|guard| *guard)
            });
        let match_threshold = self.pipeline.config().match_threshold;

        if let Some(stat_rect) = stat_rect {
            if let Some(score) = live_score {
                let is_match = score >= match_threshold;
                let value = format!("{:.0}%", (score * 100.0).clamp(0.0, 100.0));
                let sub = format!("Cosine {score:.4}");
                let dot = if is_match {
                    theme::SUCCESS
                } else {
                    theme::DANGER
                };
                widgets::stat_tile(
                    ui,
                    stat_rect,
                    "Match score",
                    Some(&value),
                    Some(&sub),
                    Some(dot),
                );
            } else if let Some(pad) = &frame.pad_result {
                let value = format!("{:.0}%", pad.score * 100.0);
                let dot = if frame.pad_live {
                    theme::SUCCESS
                } else {
                    theme::DANGER
                };
                widgets::stat_tile(
                    ui,
                    stat_rect,
                    "Liveness (PAD)",
                    Some(&value),
                    None,
                    Some(dot),
                );
            } else {
                widgets::stat_tile(ui, stat_rect, "Match score", None, None, None);
            }
        }
        if let Some(star_rect) = star_rect {
            widgets::star_tile(ui, star_rect);
        }

        widgets::card_in_rect(ui, card_rect, "live_telemetry_card", |ui| {
            widgets::card_title(ui, "Telemetry & Analysis");

            widgets::inner_table_frame().show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                widgets::table_row(
                    ui,
                    "Face Count:",
                    widgets::value_text(format!("{}", frame.detections.len()), theme::INK),
                );
                let det_score = frame.detections.first().map_or_else(
                    || "None".to_string(),
                    |det| format!("{:.2}%", det.score * 100.0),
                );
                widgets::table_row(
                    ui,
                    "Detection Score:",
                    widgets::value_text(det_score, theme::INK),
                );
                if let Some(pad) = &frame.pad_result {
                    let (kind, status, color) = if frame.pad_live {
                        (BannerKind::Success, "LIVE", theme::SUCCESS)
                    } else {
                        (BannerKind::Error, "SPOOF", theme::DANGER)
                    };
                    widgets::table_row_status(
                        ui,
                        "Anti-Spoof (PAD):",
                        Some(kind),
                        &format!("{status} ({:.2})", pad.score),
                        color,
                    );
                } else {
                    widgets::table_row_status(
                        ui,
                        "Anti-Spoof (PAD):",
                        None,
                        "Waiting...",
                        theme::INK_MUTED,
                    );
                }
                if let Some(pose) = &frame.pose {
                    for (label, angle) in [
                        ("Head Yaw:", pose.yaw),
                        ("Head Pitch:", pose.pitch),
                        ("Head Roll:", pose.roll),
                    ] {
                        widgets::table_row(
                            ui,
                            label,
                            widgets::value_text(format!("{angle:.1}°"), theme::INK),
                        );
                    }
                }
                widgets::table_row(
                    ui,
                    "Inference Time:",
                    widgets::value_text(format!("{:.1} ms", frame.pipeline_latency_ms), theme::INK),
                );
            });

            ui.add_space(18.0);
            widgets::section_title(ui, "Live 1-to-1 Match Test");
            ui.label(
                egui::RichText::new(
                    "Test real-time biometric PAM unlocking against enrolled profiles:",
                )
                .size(theme::F_SMALL)
                .color(theme::INK_MUTED),
            );

            if self.profiles.profiles.is_empty() {
                ui.label(
                    egui::RichText::new("No enrolled profiles available.")
                        .size(theme::F_BODY)
                        .color(theme::INK_MUTED),
                );
            } else {
                egui::ComboBox::from_label("Profile")
                    .width((ui.available_width() - 56.0).max(80.0))
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
                                let template =
                                    self.store.local().and_then(|s| s.get(p.uid).ok().flatten());
                                // Only a template of the loaded embedding model
                                // is a match reference (GitHub #278); a foreign
                                // or missing template clears the reference, the
                                // score and the note at once (GitHub #298).
                                self.match_reference_note = crate::worker::select_match_reference(
                                    &self.worker_input,
                                    template.as_ref(),
                                    soos_enrollment_cli::service::MODEL_ID_EMBEDDING,
                                    self.pipeline.embedding_dimension(),
                                );
                            }
                        }
                    });

                if let Some(note) = &self.match_reference_note {
                    ui.add_space(4.0);
                    widgets::banner(ui, BannerKind::Warning, note);
                }
                if let Some(score) = live_score {
                    let is_match = score >= match_threshold;
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("Cosine Match Score: {:.4}", score))
                            .size(theme::F_BODY)
                            .color(theme::INK),
                    );

                    let bar_color = if is_match {
                        theme::SUCCESS
                    } else {
                        theme::DANGER
                    };
                    let progress = (score / 1.0).clamp(0.0, 1.0);
                    widgets::progress_bar(
                        ui,
                        progress,
                        bar_color,
                        &format!("{:.1}%", score * 100.0),
                        false,
                    );

                    let (verdict, bg, fg) = if is_match {
                        (
                            "VERDICT: PAM_SUCCESS (UNLOCKED)",
                            theme::SUCCESS_BG,
                            theme::SUCCESS_TEXT,
                        )
                    } else {
                        (
                            "VERDICT: PAM_IGNORE (LOCKED)",
                            theme::DANGER_BG,
                            theme::DANGER_TEXT,
                        )
                    };
                    egui::Frame::new()
                        .fill(bg)
                        .corner_radius(12)
                        .inner_margin(egui::Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(verdict)
                                    .size(theme::F_BODY)
                                    .color(fg)
                                    .strong(),
                            );
                        });
                }
            }
        });
    }

    /// Renders Tab 2: Apple FaceID-style guided multi-step enrollment flow.
    ///
    /// Brand layout: a large rounded video panel with the oval reticle on the left, and a right
    /// column stacking the enrollment card (target, progress, step checklist, guidance, actions),
    /// a big progress stat tile and, when the height allows it, the decorative star tile.
    fn render_guided_enrollment(&mut self, ui: &mut egui::Ui, frame: &LatestFrameData) {
        let area = ui.available_rect_before_wrap();
        let metrics = theme::Metrics::for_width(ui.ctx().content_rect().width());
        let (main, column) = metrics.split_columns(area);
        ui.allocate_rect(area, egui::Sense::hover());

        // Session snapshot: one short lock per frame, never held across rendering.
        let (has_session, progress, current_step) =
            if let Ok(session_guard) = self.worker_input.enrollment_session.lock() {
                if let Some(session) = session_guard.as_ref() {
                    (true, session.progress_percent(), session.current_step())
                } else {
                    (false, 0.0, EnrollmentStep::Frontal)
                }
            } else {
                (false, 0.0, EnrollmentStep::Frontal)
            };

        // Live dynamic guidance: latest worker feedback.
        if let Ok(fb_guard) = self.worker_input.enrollment_feedback.lock() {
            if let Some(fb) = fb_guard.as_ref() {
                self.enrollment.last_feedback = Some(fb.clone());
            }
        }
        let feedback: Option<(&str, BannerKind)> =
            self.enrollment
                .last_feedback
                .as_ref()
                .map(|gui_fb| match gui_fb {
                    // GitHub #304: a multi-face frame is never sampled.
                    GuiEnrollmentFeedback::OneFaceOnly => (gui_fb.message(), BannerKind::Error),
                    GuiEnrollmentFeedback::Step(fb) => match fb {
                        EnrollmentStepFeedback::PromptCenterFace => (
                            "Please center your face inside the target frame.",
                            BannerKind::Warning,
                        ),
                        EnrollmentStepFeedback::PromptTurnLeft => (
                            "Slowly turn your head slightly to the left.",
                            BannerKind::Info,
                        ),
                        EnrollmentStepFeedback::PromptTurnRight => (
                            "Slowly turn your head slightly to the right.",
                            BannerKind::Info,
                        ),
                        EnrollmentStepFeedback::PromptTiltUp => {
                            ("Slowly tilt your head slightly upward.", BannerKind::Info)
                        }
                        EnrollmentStepFeedback::PromptHoldStill => (
                            "Hold still, capturing high-quality sample...",
                            BannerKind::Info,
                        ),
                        EnrollmentStepFeedback::SpoofDetected => {
                            ("Anti-spoof check: Live face required.", BannerKind::Error)
                        }
                        EnrollmentStepFeedback::SampleAccepted { .. } => {
                            ("Sample recorded!", BannerKind::Success)
                        }
                        EnrollmentStepFeedback::StepCompleted { .. } => (
                            "Step complete! Moving to next angle...",
                            BannerKind::Success,
                        ),
                        EnrollmentStepFeedback::AllStepsCompleted => (
                            "All angles captured! Composite template generated.",
                            BannerKind::Success,
                        ),
                        EnrollmentStepFeedback::InvalidEmbedding => (
                            "Sample rejected: invalid face features, hold still.",
                            BannerKind::Warning,
                        ),
                        EnrollmentStepFeedback::IdentityMismatch => (
                            "Sample rejected: only the enrolling user may face the camera.",
                            BannerKind::Error,
                        ),
                        EnrollmentStepFeedback::PoseOutOfRange => (
                            "Too far: rotate your head back slightly.",
                            BannerKind::Warning,
                        ),
                        EnrollmentStepFeedback::SessionAborted => (
                            "Enrollment aborted: repeated spoof detections. Cancel and restart.",
                            BannerKind::Error,
                        ),
                        EnrollmentStepFeedback::FaceQualityTooLow => (
                            "Face too small or blurred: move closer and hold still.",
                            BannerKind::Warning,
                        ),
                    },
                });

        // Main guided video panel: frame-aspect fit, centered horizontally at the top of the main area.
        let aspect_ratio = (frame.width as f32) / (frame.height as f32);
        let rect = theme::fit_video(main, aspect_ratio);
        let painter = ui.painter_at(rect);

        if let Some(texture) = &self.video_texture {
            painter.add(
                egui::epaint::RectShape::filled(
                    rect,
                    egui::CornerRadius::same(theme::R_VIDEO),
                    Color32::WHITE,
                )
                .with_texture(
                    texture.id(),
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                ),
            );

            // Guided enrollment oval reticle: pale when idle, blue while capturing, pink on a
            // rejected (error) sample, green once every angle is captured.
            let center = rect.center();
            let oval_w = rect.width() * 0.40;
            let oval_h = rect.height() * 0.60;
            let reticle_rect = Rect::from_center_size(center, Vec2::new(oval_w, oval_h));
            let rejected =
                self.enrollment.is_active && matches!(feedback, Some((_, BannerKind::Error)));
            let reticle_color = if !self.enrollment.is_active {
                None
            } else if current_step == EnrollmentStep::Completed {
                Some(theme::SUCCESS)
            } else if rejected {
                Some(theme::PINK)
            } else {
                Some(theme::BLUE)
            };
            match reticle_color {
                Some(color) => {
                    // Soft pale halo keeps the colored ring readable on any background.
                    painter.rect_stroke(
                        reticle_rect,
                        120.0,
                        Stroke::new(10.0, theme::PALE.gamma_multiply(0.35)),
                        egui::StrokeKind::Outside,
                    );
                    painter.rect_stroke(
                        reticle_rect,
                        120.0,
                        Stroke::new(4.0, color),
                        egui::StrokeKind::Outside,
                    );
                }
                None => {
                    painter.rect_stroke(
                        reticle_rect,
                        120.0,
                        Stroke::new(2.0, theme::PALE.gamma_multiply(0.67)),
                        egui::StrokeKind::Outside,
                    );
                }
            }

            // Draw guidance arrows if active
            if let Some(GuiEnrollmentFeedback::Step(fb)) = &self.enrollment.last_feedback {
                let arrow = match fb {
                    EnrollmentStepFeedback::PromptTurnLeft => Some(Vec2::new(-80.0, 0.0)),
                    EnrollmentStepFeedback::PromptTurnRight => Some(Vec2::new(80.0, 0.0)),
                    EnrollmentStepFeedback::PromptTiltUp => Some(Vec2::new(0.0, -70.0)),
                    _ => None,
                };
                if let Some(vec) = arrow {
                    painter.arrow(center, vec, Stroke::new(6.0, theme::PINK));
                }
            }

            // Caption pill: the current guidance message over the bottom of the video.
            if self.enrollment.is_active {
                if let Some((msg, _)) = feedback {
                    let font = egui::FontId::proportional(14.0);
                    let galley = painter.layout_no_wrap(msg.to_owned(), font, theme::PALE);
                    let pill_w = (galley.size().x + 36.0).min(rect.width() - 24.0);
                    let pill = Rect::from_center_size(
                        Pos2::new(rect.center().x, rect.max.y - 20.0 - 17.0),
                        Vec2::new(pill_w, 34.0),
                    );
                    painter.rect_filled(
                        pill,
                        17.0,
                        Color32::from_rgba_unmultiplied(16, 24, 32, 170),
                    );
                    painter
                        .with_clip_rect(pill.shrink2(Vec2::new(14.0, 0.0)))
                        .galley(
                            Pos2::new(
                                (pill.center().x - galley.size().x / 2.0).max(pill.min.x + 18.0),
                                pill.center().y - galley.size().y / 2.0,
                            ),
                            galley,
                            theme::PALE,
                        );
                }
            }
        } else {
            painter.rect_filled(rect, theme::R_VIDEO, theme::INK);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Acquiring video stream...",
                egui::FontId::proportional(theme::F_PLACEHOLDER),
                theme::PALE.gamma_multiply(0.67),
            );
        }

        // Fixed footer keeps Start / Cancel / Save visible without scrolling the card.
        let footer_h = if current_step == EnrollmentStep::Completed {
            80.0
        } else {
            36.0
        };
        // Right column: enrollment card, progress stat tile, star tile. Short columns use
        // compact checklist rows (hint in the tooltip) so all four steps stay visible.
        let compact_steps = column.height() < 720.0;
        let card_needed =
            widgets::card_needed_height(ui, "enrollment_card", 460.0, footer_h + 14.0);
        let (card_rect, stat_rect, star_rect) =
            widgets::column_stack_card_first(column, metrics.gap_card, card_needed);

        if let Some(stat_rect) = stat_rect {
            if has_session {
                let value = format!("{progress:.0}%");
                let sub = if current_step == EnrollmentStep::Completed {
                    "Complete".to_string()
                } else {
                    format!("Step {} of 4", (current_step as usize).min(3) + 1)
                };
                let dot = if current_step == EnrollmentStep::Completed {
                    theme::SUCCESS
                } else {
                    theme::BLUE
                };
                widgets::stat_tile(
                    ui,
                    stat_rect,
                    "Enrollment",
                    Some(&value),
                    Some(&sub),
                    Some(dot),
                );
            } else {
                widgets::stat_tile(ui, stat_rect, "Enrollment", None, Some("Not started"), None);
            }
        }
        if let Some(star_rect) = star_rect {
            widgets::star_tile(ui, star_rect);
        }

        let (_, footer_rect) =
            widgets::card_in_rect_with_footer(ui, card_rect, "enrollment_card", footer_h, |ui| {
                widgets::card_title(ui, "Guided Multi-Angle Enrollment");
                ui.label(
                egui::RichText::new(
                    "Captures high-quality embeddings across multiple head angles (Center, Left, \
                     Right, Up) to optimize real-time unlock accuracy.",
                )
                .size(theme::F_SMALL)
                .color(theme::INK_MUTED),
            );
                ui.add_space(12.0);

                // Target account: username and UID side by side.
                let gap = 8.0;
                let full_w = ui.available_width();
                let uid_w = (full_w * 0.36).clamp(64.0, 120.0);
                let name_w = (full_w - uid_w - gap).max(60.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(gap, 4.0);
                    ui.vertical(|ui| {
                        ui.set_width(name_w);
                        ui.label(
                            egui::RichText::new("Username")
                                .size(theme::F_SMALL)
                                .color(theme::INK_MUTED),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.enrollment.target_username)
                                .desired_width(name_w)
                                .margin(egui::Margin::symmetric(10, 7))
                                .text_color(theme::INK),
                        );
                    });
                    ui.vertical(|ui| {
                        ui.set_width(uid_w);
                        ui.label(
                            egui::RichText::new("Target UID")
                                .size(theme::F_SMALL)
                                .color(theme::INK_MUTED),
                        );
                        ui.visuals_mut().widgets.inactive.weak_bg_fill = theme::WHITE;
                        ui.visuals_mut().widgets.inactive.bg_fill = theme::WHITE;
                        ui.add_sized(
                            [uid_w, 30.0],
                            egui::DragValue::new(&mut self.enrollment.target_uid),
                        );
                    });
                });

                ui.add_space(14.0);

                // Progress meter
                ui.label(
                    egui::RichText::new(format!("Enrollment Progress: {:.0}%", progress))
                        .size(theme::F_BODY)
                        .color(theme::INK),
                );
                // Custom bar: no fill at 0 % and a label that stays readable at every value.
                widgets::progress_bar(
                    ui,
                    progress / 100.0,
                    theme::BLUE,
                    &format!("{:.0}%", progress),
                    self.enrollment.is_active,
                );

                ui.add_space(12.0);

                // Step checklist: done (green check), in progress (blue), waiting (hollow ring).
                widgets::inner_table_frame().show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let steps = [
                        ("1. Frontal Pose", "Look straight", EnrollmentStep::Frontal),
                        (
                            "2. Turn Left",
                            "Slowly turn head left",
                            EnrollmentStep::TurnLeft,
                        ),
                        (
                            "3. Turn Right",
                            "Slowly turn head right",
                            EnrollmentStep::TurnRight,
                        ),
                        ("4. Tilt Up", "Slowly tilt head up", EnrollmentStep::TiltUp),
                    ];
                    let rows: Vec<(&str, &str, widgets::StepStatus)> = steps
                        .iter()
                        .map(|(name, hint, step)| {
                            let status = if !has_session {
                                widgets::StepStatus::Waiting
                            } else if current_step == EnrollmentStep::Completed
                                || (current_step as usize) > (*step as usize)
                            {
                                widgets::StepStatus::Done
                            } else if current_step == *step {
                                widgets::StepStatus::InProgress
                            } else {
                                widgets::StepStatus::Waiting
                            };
                            (*name, *hint, status)
                        })
                        .collect();
                    widgets::step_list(ui, &rows, compact_steps);
                });

                ui.add_space(12.0);

                if let Some((msg, kind)) = feedback {
                    widgets::banner(ui, kind, msg);
                    ui.add_space(12.0);
                }

                if let Some((msg, is_err)) = &self.enrollment.status_message {
                    ui.add_space(10.0);
                    let kind = if *is_err {
                        BannerKind::Error
                    } else {
                        BannerKind::Success
                    };
                    widgets::banner(ui, kind, msg);
                }
            });
        ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
            // Action controls
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                if !self.enrollment.is_active {
                    if ui
                        .add(
                            widgets::BrandButton::primary("▶ Start Guided Enrollment").full_width(),
                        )
                        .clicked()
                    {
                        self.enrollment.is_active = true;
                        if let Ok(mut session_guard) = self.worker_input.enrollment_session.lock() {
                            *session_guard = Some(crate::worker::new_guided_enrollment_session());
                        }
                    }
                } else if ui
                    .add(widgets::BrandButton::secondary("⏹ Cancel").full_width())
                    .clicked()
                {
                    self.enrollment.is_active = false;
                    if let Ok(mut session_guard) = self.worker_input.enrollment_session.lock() {
                        *session_guard = None;
                    }
                }

                if current_step == EnrollmentStep::Completed
                    && ui
                        .add(
                            widgets::BrandButton::primary("💾 Save & Encrypt Biometric Template")
                                .full_width(),
                        )
                        .clicked()
                {
                    // A fusion error (inconsistent or invalid samples, GitHub #183) is
                    // surfaced instead of silently ignoring the Save click.
                    let fused_opt = match self.worker_input.enrollment_session.lock() {
                        Ok(session_guard) => match session_guard
                            .as_ref()
                            .map(|session| session.compute_composite_embedding())
                        {
                            Some(Ok(embedding)) => Some(embedding),
                            Some(Err(e)) => {
                                self.enrollment.status_message =
                                    Some((format!("Cannot build the template: {e}"), true));
                                None
                            }
                            None => None,
                        },
                        Err(_) => None,
                    };

                    if let Some(fused_embedding) = fused_opt {
                        let timestamp = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();

                        let res = BiometricTemplate::new(
                            self.enrollment.target_uid,
                            soos_enrollment_cli::service::MODEL_ID_EMBEDDING.to_string(),
                            soos_enrollment_cli::service::EMBEDDING_MODEL_VERSION.to_string(),
                            timestamp,
                            fused_embedding.clone(),
                        );

                        match res {
                            Ok(template) => {
                                if self.store.uses_polkit() {
                                    // The embedding is piped to `pkexec soos-enroll import
                                    // --file -` off the UI thread; no local copy and no
                                    // temporary file (GitHub #154 / #156).
                                    let action = PrivilegedAction::ImportTemplate {
                                        uid: self.enrollment.target_uid,
                                        embedding: fused_embedding.clone(),
                                    };
                                    match self.tasks.submit(action) {
                                        Ok(()) => {
                                            self.enrollment.status_message = Some((
                                                "Importing template into the system store; \
                                                 answer the Polkit prompt..."
                                                    .to_string(),
                                                false,
                                            ));
                                        }
                                        Err(e) => {
                                            self.enrollment.status_message = Some((
                                                format!("Cannot import template: {e}"),
                                                true,
                                            ));
                                        }
                                    }
                                } else {
                                    // The store write may wait for the store lock held
                                    // by `soos-enroll`: it runs on the store worker and
                                    // its outcome arrives in
                                    // `handle_store_task_outcomes` (GitHub #291).
                                    let username = self.enrollment.target_username.clone();
                                    match self.submit_store_task(StoreTask::Enroll(template)) {
                                        Ok(()) => {
                                            self.pending_store_username = Some(username);
                                            self.enrollment.status_message =
                                                Some(("Saving the template...".to_string(), false));
                                        }
                                        Err(e) => {
                                            self.enrollment.status_message = Some((
                                                format!("Cannot save the template: {e}"),
                                                true,
                                            ));
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                self.enrollment.status_message =
                                    Some((format!("Template creation error: {e}"), true));
                            }
                        }
                    }
                }
            });
        });
    }

    /// Renders Tab 3: Enrolled profiles and template deletion.
    fn render_profiles(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        let metrics = theme::Metrics::for_width(ui.ctx().content_rect().width());
        let (main, column) = metrics.split_columns(area);
        ui.allocate_rect(area, egui::Sense::hover());

        // Right column: template count tile, then the decorative star filling the rest of the
        // height, so the column ends level with the main card.
        let tile = column.width();
        let (stat_h, star) = widgets::profiles_column(column.height(), tile, metrics.gap_card);
        let stat = Rect::from_min_size(column.min, Vec2::new(tile, stat_h));
        // Polkit mode keeps no local copy: an empty list there means "not loaded yet".
        let count = (!(self.profiles.profiles.is_empty() && self.store.uses_polkit()))
            .then(|| self.profiles.profiles.len().to_string());
        widgets::stat_tile(
            ui,
            stat,
            "TEMPLATES",
            count.as_deref(),
            Some("AES-256-GCM at rest"),
            None,
        );
        if let Some(star_h) = star {
            let star = Rect::from_min_size(
                Pos2::new(column.min.x, stat.max.y + metrics.gap_card),
                Vec2::new(tile, star_h),
            );
            widgets::star_tile(ui, star);
        }

        // Main card hosting the title row, the store notice and the profiles table.
        ui.painter().rect(
            main,
            egui::CornerRadius::same(theme::R_CARD),
            theme::PALE,
            Stroke::new(theme::STROKE_CARD, theme::BLUE),
            egui::StrokeKind::Inside,
        );
        let inner = Rect::from_min_max(
            main.min + Vec2::new(24.0, 20.0),
            Pos2::new(
                (main.max.x - 24.0).max(main.min.x + 24.0),
                (main.max.y - 20.0).max(main.min.y + 20.0),
            ),
        );
        let (notice_kind, notice) = match &self.store {
            GuiStore::Polkit => (
                BannerKind::Info,
                "System store (Polkit): templates live in the root-protected store used by PAM. \
                 Listing and deleting them asks for administrator authorization.",
            ),
            GuiStore::System(_) => (
                BannerKind::Info,
                "System store: templates live in the root-protected store used by PAM.",
            ),
            GuiStore::Developer { .. } => (
                BannerKind::Warning,
                "Developer store: templates listed here are NOT used by PAM.",
            ),
        };

        let mut refresh = false;
        let mut uid_to_delete = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            // Title row: left-aligned faux-bold title, Refresh on the right.
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                Vec2::new(width, 38.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_size(Vec2::new(width, 38.0));
                    let title = "Enrolled Biometric Profiles";
                    let font = egui::FontId::proportional(theme::F_CARD_TITLE);
                    let title_w = ui
                        .painter()
                        .layout_no_wrap(title.to_owned(), font.clone(), theme::INK)
                        .size()
                        .x
                        + 2.0;
                    let (rect, _) = ui.allocate_exact_size(
                        Vec2::new(title_w.min((width - 140.0).max(0.0)), 26.0),
                        egui::Sense::hover(),
                    );
                    theme::paint_faux_bold(
                        &ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())),
                        rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        title,
                        font,
                        theme::INK,
                        2,
                        0.6,
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(widgets::BrandButton::secondary("🔄 Refresh"))
                            .clicked()
                        {
                            refresh = true;
                        }
                    });
                },
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Biometric templates stored securely on disk, encrypted with AES-256-GCM. \
                         Deleting a template overwrites and removes it (best effort; see Docs).",
                    )
                    .size(theme::F_SMALL)
                    .color(theme::INK_MUTED),
                )
                .wrap(),
            );
            ui.add_space(10.0);
            widgets::banner(ui, notice_kind, notice);
            ui.add_space(16.0);

            egui::ScrollArea::vertical()
                .id_salt("profiles_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width((ui.available_width() - 4.0).max(0.0));
                    if self.profiles.profiles.is_empty() {
                        if Self::render_profiles_empty_state(ui, self.store.uses_polkit()) {
                            refresh = true;
                        }
                    } else {
                        uid_to_delete = Self::render_profiles_table(ui, &self.profiles.profiles);
                    }

                    if let Some((msg, is_err)) = &self.profiles.status_message {
                        ui.add_space(12.0);
                        let kind = if *is_err {
                            BannerKind::Error
                        } else {
                            BannerKind::Success
                        };
                        widgets::banner(ui, kind, msg);
                    }
                });
        });

        if refresh {
            self.refresh_profiles();
        }
        if let Some(uid) = uid_to_delete {
            self.profiles.confirm_delete_uid = Some(uid);
        }

        // Confirmation modal (two-step delete), only reachable from a non-empty table.
        if self.profiles.profiles.is_empty() {
            return;
        }
        if let Some(uid) = self.profiles.confirm_delete_uid {
            // A real modal: the backdrop dims the page and absorbs every click behind it, so
            // no other row, tab or the daemon switch can be used while the question is open.
            let modal_frame = egui::Frame::new()
                .fill(theme::PALE)
                .stroke(Stroke::new(theme::STROKE_CARD, theme::BLUE))
                .corner_radius(egui::CornerRadius::same(theme::R_CARD))
                .inner_margin(egui::Margin::same(24))
                .shadow(ui.visuals().window_shadow);
            let question =
                format!("Are you sure you want to delete the biometric template for UID {uid}?");
            let mut confirmed = false;
            let mut cancelled = false;
            let modal = egui::Modal::new(egui::Id::new("Confirm Deletion"))
                .frame(modal_frame)
                .backdrop_color(Color32::from_rgba_unmultiplied(16, 24, 32, 90))
                .show(ui.ctx(), |ui| {
                    ui.set_width(380.0);
                    widgets::card_title(ui, "Confirm Deletion");
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(question.as_str())
                                .size(14.0)
                                .color(theme::INK),
                        )
                        .wrap(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("This operation cannot be undone.")
                            .size(theme::F_BODY)
                            .color(theme::DANGER_TEXT),
                    );
                    ui.add_space(16.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), 36.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            confirmed = ui
                                .add(widgets::BrandButton::danger("Yes, Delete Template"))
                                .clicked();
                            cancelled = ui.add(widgets::BrandButton::secondary("Cancel")).clicked();
                        },
                    );
                });
            // Escape or a click on the backdrop is a Cancel, never a confirmation.
            if !confirmed && modal.should_close() {
                cancelled = true;
            }
            if confirmed {
                if self.store.uses_polkit() {
                    // Runs off the UI thread; the outcome arrives in
                    // `handle_task_outcomes` (GitHub #154).
                    if let Err(e) = self.tasks.submit(PrivilegedAction::DeleteTemplate { uid }) {
                        self.profiles.status_message =
                            Some((format!("Cannot delete template for UID {uid}: {e}"), true));
                    }
                } else if let Err(e) = self.submit_store_task(StoreTask::Delete { uid }) {
                    // The deletion runs on the store worker; its outcome
                    // arrives in `handle_store_task_outcomes` (GitHub #291).
                    self.profiles.status_message =
                        Some((format!("Cannot delete template: {e}"), true));
                }
                self.profiles.confirm_delete_uid = None;
            }
            if cancelled {
                self.profiles.confirm_delete_uid = None;
            }
        }
    }

    /// Empty profiles state: star mark, message and (Polkit mode) the load button.
    ///
    /// Returns `true` when the user asked to load the profiles.
    fn render_profiles_empty_state(ui: &mut egui::Ui, uses_polkit: bool) -> bool {
        let mut load = false;
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            let (star, _) = ui.allocate_exact_size(Vec2::splat(72.0), egui::Sense::hover());
            brand::paint_star(ui, star, theme::BLUE);
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("No biometric templates currently loaded.")
                    .size(theme::F_SECTION)
                    .color(theme::INK),
            );
            ui.add_space(6.0);
            if uses_polkit {
                ui.label(
                    egui::RichText::new(
                        "Click below to query the root-protected system biometric store via Polkit:",
                    )
                    .size(theme::F_BODY)
                    .color(theme::INK_MUTED),
                );
                ui.add_space(12.0);
                if ui
                    .add(widgets::BrandButton::primary(
                        "🔑 Load Profiles via System Authorization",
                    ))
                    .clicked()
                {
                    load = true;
                }
            } else {
                ui.label(
                    egui::RichText::new("No biometric templates currently enrolled on this system.")
                        .size(theme::F_BODY)
                        .color(theme::INK_MUTED),
                );
            }
        });
        load
    }

    /// Profiles table: blue header row, 40-point body rows with a separator and hover tint,
    /// and a danger-outline Delete button per row. Returns the UID whose Delete was clicked.
    fn render_profiles_table(ui: &mut egui::Ui, profiles: &[EnrolledUserSummary]) -> Option<u32> {
        const HEADERS: [&str; 7] = [
            "UID",
            "Username",
            "Model ID",
            "Version",
            "Timestamp",
            "Dim",
            "Actions",
        ];
        // Relative column widths; the Actions column keeps a fixed width for its button.
        const WEIGHTS: [f32; 6] = [60.0, 150.0, 190.0, 76.0, 112.0, 50.0];
        const ACTIONS_W: f32 = 104.0;
        const PAD_X: f32 = 14.0;
        const HEADER_H: f32 = 36.0;
        const ROW_H: f32 = 40.0;

        let mut uid_to_delete = None;
        egui::Frame::new()
            .fill(theme::PALE_2)
            .corner_radius(egui::CornerRadius::same(theme::R_TABLE))
            .show(ui, |ui| {
                let width = ui.available_width();
                ui.set_width(width);
                let flexible = (width - ACTIONS_W - 2.0 * PAD_X).max(0.0);
                let total: f32 = WEIGHTS.iter().sum();
                let mut widths: Vec<f32> = WEIGHTS.iter().map(|w| flexible * w / total).collect();
                widths.push(ACTIONS_W);

                // Text clip of a cell: a small right gutter, a little slack on the left so the
                // first glyph's side bearing is never cut.
                let cell_clip = |cell: &Rect| -> Rect {
                    Rect::from_min_max(
                        Pos2::new(cell.min.x - 2.0, cell.min.y),
                        Pos2::new((cell.max.x - 8.0).max(cell.min.x), cell.max.y),
                    )
                };
                let columns = |row: Rect| -> Vec<Rect> {
                    let mut x = row.min.x + PAD_X;
                    widths
                        .iter()
                        .map(|w| {
                            let cell = Rect::from_min_max(
                                Pos2::new(x, row.min.y),
                                Pos2::new(x + w, row.max.y),
                            );
                            x += w;
                            cell
                        })
                        .collect()
                };

                // Header row.
                let (header, _) =
                    ui.allocate_exact_size(Vec2::new(width, HEADER_H), egui::Sense::hover());
                ui.painter().rect_filled(
                    header,
                    egui::CornerRadius {
                        nw: theme::R_TABLE,
                        ne: theme::R_TABLE,
                        sw: 0,
                        se: 0,
                    },
                    theme::BLUE,
                );
                let header_font = egui::FontId::proportional(theme::F_SMALL);
                for (cell, text) in columns(header).iter().zip(HEADERS) {
                    theme::paint_faux_bold(
                        &ui.painter()
                            .with_clip_rect(cell_clip(cell).intersect(ui.clip_rect())),
                        cell.left_center(),
                        egui::Align2::LEFT_CENTER,
                        &text.to_uppercase(),
                        header_font.clone(),
                        theme::PALE,
                        2,
                        0.5,
                    );
                }

                // Body rows.
                let body_font = egui::FontId::proportional(theme::F_BODY);
                for (index, p) in profiles.iter().enumerate() {
                    let (row, _) =
                        ui.allocate_exact_size(Vec2::new(width, ROW_H), egui::Sense::hover());
                    if ui.rect_contains_pointer(row) {
                        ui.painter().rect_filled(row, 0.0, theme::PALE_3);
                    }
                    if index > 0 {
                        ui.painter().hline(
                            row.x_range().shrink(8.0),
                            row.min.y,
                            Stroke::new(1.0, theme::LINE),
                        );
                    }
                    let values = [
                        p.uid.to_string(),
                        p.username.clone(),
                        p.model_id.clone(),
                        p.model_version.clone(),
                        p.enrollment_timestamp.to_string(),
                        p.embedding_dim.to_string(),
                    ];
                    let cells = columns(row);
                    for (cell, value) in cells.iter().zip(values) {
                        let clip = cell_clip(cell).intersect(ui.clip_rect());
                        ui.painter().with_clip_rect(clip).text(
                            cell.left_center(),
                            egui::Align2::LEFT_CENTER,
                            value,
                            body_font.clone(),
                            theme::INK,
                        );
                    }
                    if let Some(actions) = cells.last() {
                        let clicked = ui
                            .scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(*actions)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                |ui| ui.add(widgets::BrandButton::danger_outline("🗑 Delete")),
                            )
                            .inner
                            .clicked();
                        if clicked {
                            uid_to_delete = Some(p.uid);
                        }
                    }
                }
                ui.add_space(4.0);
            });
        uid_to_delete
    }
}

impl eframe::App for SoosApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Apply finished background privileged operations without blocking (GitHub #154).
        self.handle_task_outcomes();
        self.handle_store_task_outcomes();
        self.sync_camera_source_generation();
        let camera_notice = self.current_camera_notice();
        let camera_status = self.camera.status();
        self.track_camera_status(camera_status);

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
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let content = self.render_header(ui, frame_ref);
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                self.render_notices(ui);

                if let Some(frame) = frame_ref {
                    match self.current_tab {
                        AppTab::LiveInspection => self.render_live_inspection(ui, frame),
                        AppTab::GuidedEnrollment => self.render_guided_enrollment(ui, frame),
                        AppTab::Profiles => self.render_profiles(ui),
                    }
                } else if let Some(notice) = camera_notice.as_deref() {
                    ui.add_space(60.0);
                    widgets::status_card(ui, |ui| {
                        widgets::card_title(ui, "Camera unavailable");
                        widgets::banner(ui, BannerKind::Warning, notice);
                    });
                } else {
                    // Distinct, actionable camera state instead of a generic spinner (GitHub #155).
                    ui.add_space(60.0);
                    render_status_banner(ui, &camera_status_banner(&camera_status));
                }
            });
        });
    }
}

impl Drop for SoosApp {
    fn drop(&mut self) {
        self.worker_running.store(false, Ordering::Release);
    }
}

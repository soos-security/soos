//! Layout dimension verification tests for SOOS GUI.
//!
//! Validates:
//! - Camera video view area expands to fill the available panel height and width
//! - Sizing does not collapse to default interact height (18px)
//! - Checkboxes and controls do not push video canvas out-of-bounds in windowed mode
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

mod common;

use common::{wait_until, SETTLE_TIMEOUT};
use eframe::egui::{self, Vec2};

#[test]
fn test_live_inspection_layout_allocates_large_canvas() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Emulate window size 1120x780 with header rendered
            let total_avail = Vec2::new(1120.0, 740.0);
            let sidebar_width = (total_avail.x * 0.32).clamp(280.0, 380.0);
            let main_width = (total_avail.x - sidebar_width - 16.0).max(300.0);
            let content_height = total_avail.y;

            let mut measured_target_size = Vec2::ZERO;

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(main_width, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut dummy = true;
                            ui.checkbox(&mut dummy, "Bounding Box (SCRFD)");
                            ui.checkbox(&mut dummy, "5 Landmarks (SCRFD)");
                            ui.checkbox(&mut dummy, "PAD Area (MiniFASNetV2)");
                            ui.checkbox(&mut dummy, "Aligned Crop (112×112)");
                            ui.checkbox(&mut dummy, "Pose & Angles");
                        });
                        ui.add_space(4.0);

                        let avail_size = ui.available_size();
                        let aspect_ratio = 640.0 / 480.0; // 1.333
                        let target_w = avail_size.x.min(avail_size.y * aspect_ratio);
                        let target_h = target_w / aspect_ratio;
                        measured_target_size = Vec2::new(target_w, target_h);
                    },
                );

                ui.separator();
                ui.allocate_ui_with_layout(
                    Vec2::new(sidebar_width, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |_ui| {},
                );
            });

            // Must NOT collapse to 18px/24px
            assert!(
                measured_target_size.x >= 500.0,
                "Target width {} must be >= 500px",
                measured_target_size.x
            );
            assert!(
                measured_target_size.y >= 350.0,
                "Target height {} must be >= 350px",
                measured_target_size.y
            );
            let ratio = measured_target_size.x / measured_target_size.y;
            assert!(
                (ratio - (4.0 / 3.0)).abs() < 1e-3,
                "Aspect ratio must be 4:3"
            );
        });
    });
}

#[test]
fn test_windowed_mode_live_inspection_layout_with_checkboxes() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Emulate compact windowed mode (900x600 minimum inner size)
            let total_avail = Vec2::new(900.0, 560.0);
            let sidebar_width = (total_avail.x * 0.32).clamp(280.0, 380.0);
            let main_width = (total_avail.x - sidebar_width - 16.0).max(300.0);
            let content_height = total_avail.y;

            let mut measured_target_size = Vec2::ZERO;

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(main_width, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut dummy = true;
                            ui.checkbox(&mut dummy, "Bounding Box (SCRFD)");
                            ui.checkbox(&mut dummy, "5 Landmarks (SCRFD)");
                            ui.checkbox(&mut dummy, "PAD Area (MiniFASNetV2)");
                            ui.checkbox(&mut dummy, "Aligned Crop (112×112)");
                            ui.checkbox(&mut dummy, "Pose & Angles");
                        });
                        ui.add_space(4.0);

                        let avail_size = ui.available_size();
                        let aspect_ratio = 640.0 / 480.0; // 1.333
                        let target_w = avail_size.x.min(avail_size.y * aspect_ratio);
                        let target_h = target_w / aspect_ratio;
                        measured_target_size = Vec2::new(target_w, target_h);
                    },
                );

                ui.separator();
                ui.allocate_ui_with_layout(
                    Vec2::new(sidebar_width, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |_ui| {},
                );
            });

            assert!(
                measured_target_size.x >= 450.0,
                "In 900px windowed mode, target width {} must be >= 450px",
                measured_target_size.x
            );
            assert!(
                measured_target_size.y >= 300.0,
                "In 600px windowed mode, target height {} must be >= 300px",
                measured_target_size.y
            );
            let ratio = measured_target_size.x / measured_target_size.y;
            assert!(
                (ratio - (4.0 / 3.0)).abs() < 1e-3,
                "Aspect ratio must be 4:3"
            );
        });
    });
}

#[test]
fn test_guided_enrollment_layout_allocates_large_canvas() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let total_avail = Vec2::new(1120.0, 740.0);
            let sidebar_width = (total_avail.x * 0.35).clamp(300.0, 420.0);
            let main_width = (total_avail.x - sidebar_width - 16.0).max(300.0);
            let content_height = total_avail.y;

            let mut measured_target_size = Vec2::ZERO;

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(main_width, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let avail_size = ui.available_size();
                        let aspect_ratio = 640.0 / 360.0; // 16:9
                        let target_w = (avail_size.x * 0.95).min(avail_size.y * aspect_ratio);
                        let target_h = target_w / aspect_ratio;
                        measured_target_size = Vec2::new(target_w, target_h);
                    },
                );

                ui.separator();
                ui.allocate_ui_with_layout(
                    Vec2::new(sidebar_width, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |_ui| {},
                );
            });

            assert!(
                measured_target_size.x >= 450.0,
                "Target width {} must be >= 450px",
                measured_target_size.x
            );
            assert!(
                measured_target_size.y >= 250.0,
                "Target height {} must be >= 250px",
                measured_target_size.y
            );
            let ratio = measured_target_size.x / measured_target_size.y;
            assert!(
                (ratio - (16.0 / 9.0)).abs() < 1e-3,
                "Aspect ratio must be 16:9"
            );
        });
    });
}

#[test]
fn test_enrolled_user_summary_json_roundtrip() {
    let json_data = r#"[
        {"uid": 1000, "username": "hadrien", "model_id": "arcface_w600k_mbf", "model_version": "2.0.0", "enrollment_timestamp": 1790426080, "embedding_dim": 512}
    ]"#;
    let summaries: Vec<soos_enrollment_cli::service::EnrolledUserSummary> =
        serde_json::from_str(json_data).unwrap_or_default();
    assert_eq!(summaries.len(), 1);
    if let Some(first) = summaries.first() {
        assert_eq!(first.uid, 1000);
        assert_eq!(first.username, "hadrien");
        assert_eq!(first.model_id, "arcface_w600k_mbf");
        assert_eq!(first.model_version, "2.0.0");
        assert_eq!(first.enrollment_timestamp, 1790426080);
        assert_eq!(first.embedding_dim, 512);
    }
}

#[test]
fn test_ipc_camera_manager_receives_persistent_frames() {
    use soos_camera_v4l::CameraManager;
    use soos_protocol::codec::encode_preview;
    use soos_protocol::types::{PreviewResponse, CURRENT_VERSION};
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    let sock_path = std::env::temp_dir().join(format!("gui_test_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path).unwrap();

    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // Serve 3 frames on the SAME stream
        for seq in 0..3 {
            let mut len_bytes = [0u8; 4];
            stream.read_exact(&mut len_bytes).unwrap();
            let len = u32::from_be_bytes(len_bytes) as usize;
            let mut req_buf = vec![0u8; len];
            stream.read_exact(&mut req_buf).unwrap();

            let resp = PreviewResponse {
                version: CURRENT_VERSION,
                sequence: seq,
                width: 640,
                height: 480,
                format: 0, // Rgb24
                timestamp_monotonic_ns: 1000 + seq,
                data: vec![128u8; 640 * 480 * 3],
            };
            let encoded = encode_preview(&resp).unwrap();
            stream.write_all(&encoded).unwrap();
            stream.flush().unwrap();
        }
    });

    let manager = soos_gui::IpcCameraManager::spawn(&sock_path);

    // Wait for manager to receive frames and become ready
    wait_until(SETTLE_TIMEOUT, || manager.is_ready());

    assert!(manager.is_ready(), "IPC Camera Manager must become ready");
    let frame = manager.latest_frame().expect("Frame must exist");
    assert_eq!(frame.width, 640);
    assert_eq!(frame.height, 480);

    server.join().unwrap();
    let _ = std::fs::remove_file(&sock_path);
}

#[test]
fn test_gui_worker_fallback_renders_raw_rgb_on_pipeline_error() {
    use arc_swap::ArcSwapOption;
    use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat};
    use soos_inference_ort::{MockEmbeddingExtractor, MockPadDetector};
    use soos_vision::{VisionPipeline, VisionPipelineConfig};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let camera_config = CameraConfigBuilder::new()
        .resolution(640, 480)
        .format(PixelFormat::Rgb24)
        .fps(30)
        .warmup_frames(0)
        .build();
    let camera = Arc::new(MockCameraManager::new(camera_config));

    struct AlwaysFailDetector;
    impl soos_inference_ort::FaceDetector for AlwaysFailDetector {
        fn detect(
            &self,
            _rgb: &[u8],
            _w: u32,
            _h: u32,
        ) -> Result<Vec<soos_inference_ort::FaceDetection>, soos_inference_ort::InferenceError>
        {
            Err(soos_inference_ort::InferenceError::DetectionFailed(
                "Continuous failure".to_string(),
            ))
        }
    }

    let detector = Arc::new(AlwaysFailDetector);
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let pipeline = Arc::new(VisionPipeline::new(
        detector.clone(),
        pad,
        extractor,
        VisionPipelineConfig::default(),
    ));

    let latest_frame_slot = Arc::new(ArcSwapOption::empty());
    let worker_input = Arc::new(soos_gui::worker::WorkerSharedInput::default());
    let running = Arc::new(AtomicBool::new(true));
    let egui_ctx = egui::Context::default();

    let _handle = soos_gui::worker::spawn_vision_worker(
        camera.clone(),
        pipeline.clone(),
        latest_frame_slot.clone(),
        worker_input.clone(),
        running.clone(),
        egui_ctx,
    );

    // Wait for the worker to process a frame
    wait_until(SETTLE_TIMEOUT, || latest_frame_slot.load().is_some());

    running.store(false, Ordering::Release);
    camera.stop();

    let frame = latest_frame_slot.load();
    assert!(
        frame.is_some(),
        "Worker must populate latest_frame_slot with fallback RGB even on pipeline error"
    );
    let data = frame.as_ref().unwrap();
    assert_eq!(data.width, 640);
    assert_eq!(data.height, 480);
    assert!(!data.rgb.is_empty());
    assert!(data.detections.is_empty());
}

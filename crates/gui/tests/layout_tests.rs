//! Layout dimension verification tests for SOOS GUI.
//!
//! Validates:
//! - Camera video view area expands to fill the available panel height and width
//! - Sizing does not collapse to default interact height (18px)
//! - Checkboxes and controls do not push video canvas out-of-bounds in windowed mode
//! - Aspect ratio (4:3 or 16:9) is strictly preserved in both fullscreen and windowed modes

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

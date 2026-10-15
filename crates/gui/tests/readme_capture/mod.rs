//! Shared README capture harness (GitHub #381; spec `AI/architect_spec_readme_capture.md` §3.2):
//! drives the real `eframe::App::ui` of a capture scene headlessly with a fixed `RawInput`,
//! tessellates the output and rasterizes it on the CPU. Test code only.

#![allow(
    dead_code,
    reason = "Shared harness: each test target uses the helpers it needs"
)]

pub mod png_io;
pub mod raster;

use eframe::egui::epaint::{ClippedShape, Shape};
use eframe::egui::{self, Pos2, Rect, Vec2, ViewportId, ViewportInfo};
use eframe::App as _;
use soos_gui::app::capture::{capture_app, CaptureScene, CAPTURE_WINDOW_POINTS};
use soos_test_fixtures::readme::create_placeholder_face_frame;

use raster::{Rasterizer, RgbaImage};

/// Pixels per point of every capture (spec §3.2).
pub const CAPTURE_PIXELS_PER_POINT: f32 = 1.0;
/// UI passes per capture; the last one is rendered (layout settles in pass 2).
pub const CAPTURE_PASSES: usize = 3;
/// Size of the synthetic camera frame (the mock camera default).
pub const CAPTURE_FRAME_SIZE: [u32; 2] = [640, 480];

/// One rendered scene and every text it shows.
pub struct Capture {
    /// Rendered pixels.
    pub image: RgbaImage,
    /// Every text galley of the last pass.
    pub texts: Vec<String>,
}

fn collect_texts(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Text(text) => out.push(text.galley.text().to_owned()),
        Shape::Vec(shapes) => {
            for inner in shapes {
                collect_texts(inner, out);
            }
        }
        _ => {}
    }
}

fn raw_input(pass: usize) -> egui::RawInput {
    let [w, h] = CAPTURE_WINDOW_POINTS;
    let mut input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h))),
        max_texture_side: Some(raster::MAX_SIDE),
        time: Some(pass as f64),
        predicted_dt: 1.0 / 60.0,
        focused: true,
        ..Default::default()
    };
    input.viewports.insert(
        ViewportId::ROOT,
        ViewportInfo {
            native_pixels_per_point: Some(CAPTURE_PIXELS_PER_POINT),
            ..Default::default()
        },
    );
    input
}

/// Renders `scene` from the synthetic placeholder face.
///
/// # Panics
///
/// If the scene cannot be built (test failure).
pub fn render_scene(scene: CaptureScene) -> Capture {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let [fw, fh] = CAPTURE_FRAME_SIZE;
    let frame = create_placeholder_face_frame(fw, fh);
    let mut app = capture_app(&cc, scene, &frame).expect("capture scene");
    let mut eframe_frame = eframe::Frame::_new_kittest();
    let mut rasterizer = Rasterizer::default();

    let mut last: Option<(Vec<ClippedShape>, f32, egui::TexturesDelta)> = None;
    for pass in 0..CAPTURE_PASSES {
        if let Some((_, _, delta)) = last.take() {
            rasterizer.free(&delta);
        }
        let output = ctx.run_ui(raw_input(pass), |ui| app.ui(ui, &mut eframe_frame));
        rasterizer.apply(&output.textures_delta);
        last = Some((
            output.shapes,
            output.pixels_per_point,
            output.textures_delta,
        ));
    }
    let (shapes, ppp, _) = last.expect("at least one pass");
    let mut texts = Vec::new();
    for clipped in &shapes {
        collect_texts(&clipped.shape, &mut texts);
    }
    let primitives = ctx.tessellate(shapes, ppp);
    let [w, h] = CAPTURE_WINDOW_POINTS;
    let size = [(w * ppp).round() as u32, (h * ppp).round() as u32];
    let background = ctx.global_style().visuals.panel_fill;
    let image = rasterizer.render(&primitives, ppp, size, background);
    Capture { image, texts }
}

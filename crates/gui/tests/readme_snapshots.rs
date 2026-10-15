//! README screenshot capture of `soos-gui` (GitHub #381, PR 1 of #356; ADR 2026-10-13 "README
//! Assets Are Generated From Synthetic Data, Never Hand-Edited"; spec
//! `AI/architect_spec_readme_capture.md` §3 and §8; matrix RAC1–RAC6).
//!
//! Renders the real application screens headlessly from synthetic data and compares them with
//! the committed `Docs/assets/screenshots/gui-*.png`. Regenerate with
//! `scripts/capture_readme_assets.sh` (it sets `SOOS_UPDATE_README_ASSETS=1`).

#![cfg(feature = "readme-capture")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod readme_capture;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::epaint::{ClippedPrimitive, Mesh, Primitive, Vertex};
use eframe::egui::{
    self, Color32, ColorImage, ImageData, Pos2, Rect, TextureId, TextureOptions, TexturesDelta,
};
use readme_capture::png_io::{decode_png, diff_fraction, encode_png, png_chunk_types};
use readme_capture::raster::{Rasterizer, RgbaImage};
use readme_capture::{render_scene, CAPTURE_PIXELS_PER_POINT};
use soos_frame::{Frame, PixelFormat};
use soos_gui::app::capture::{
    capture_app, shown_identities, CaptureError, CaptureScene, CAPTURE_UID, CAPTURE_USERNAME,
    CAPTURE_WINDOW_POINTS,
};

/// Per-channel difference ignored by the drift comparison (spec §4).
/// Absorbs ±1 anti-aliasing differences of the platform `powf` (spec §10, F-3).
const CAPTURE_DIFF_CHANNEL_TOLERANCE: u8 = 16;
/// Fraction of differing pixels tolerated by the drift comparison (spec §10, F-3): at most 17
/// of the 873 600 pixels of a capture.
const CAPTURE_MAX_DIFF_FRACTION: f64 = 0.000_02;
/// Login names that are ordinary words of the UI text: the free-text scan of RAC3 is skipped
/// for them (spec §10, F-1); the exact state checks always run.
const ORDINARY_WORD_LOGINS: [&str; 7] = ["alice", "root", "user", "admin", "test", "guest", "soos"];
/// Environment switch of the regeneration mode.
const UPDATE_ENV: &str = "SOOS_UPDATE_README_ASSETS";

fn screenshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Docs/assets/screenshots")
}

// ---------------------------------------------------------------------------------------
// RAC1–RAC4: capture scenes
// ---------------------------------------------------------------------------------------

/// RAC1: each scene matches its committed screenshot (or is rewritten in update mode).
#[test]
fn test_rac_capture_scenes_match_committed_assets() {
    let update = std::env::var(UPDATE_ENV).is_ok_and(|v| v == "1");
    assert!(
        !(update && std::env::var_os("CI").is_some()),
        "{UPDATE_ENV}=1 is refused in CI: regenerate locally with scripts/capture_readme_assets.sh"
    );
    let dir = screenshots_dir();
    let mut failures = Vec::new();
    for scene in CaptureScene::ALL {
        let capture = render_scene(scene);
        let [w, h] = CAPTURE_WINDOW_POINTS;
        assert_eq!(
            (capture.image.width, capture.image.height),
            (
                (w * CAPTURE_PIXELS_PER_POINT) as u32,
                (h * CAPTURE_PIXELS_PER_POINT) as u32
            )
        );
        let path = dir.join(format!("{}.png", scene.file_stem()));
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, encode_png(&capture.image)).unwrap();
            continue;
        }
        let committed = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{} is missing ({e}); run scripts/capture_readme_assets.sh",
                path.display()
            )
        });
        let committed = decode_png(&committed).expect("committed PNG decodes");
        match diff_fraction(&capture.image, &committed, CAPTURE_DIFF_CHANNEL_TOLERANCE) {
            Some(fraction) if fraction <= CAPTURE_MAX_DIFF_FRACTION => {}
            other => failures.push(format!("{}: {other:?}", scene.file_stem())),
        }
    }
    assert!(
        failures.is_empty(),
        "the GUI changed: README screenshots are stale ({failures:?}); regenerate them (and \
         Docs/assets/MANIFEST.toml) with scripts/capture_readme_assets.sh"
    );
}

/// RAC2: rendering is deterministic (two independent renders are byte-identical).
#[test]
fn test_rac_capture_render_is_deterministic() {
    let first = render_scene(CaptureScene::LiveInspection);
    let second = render_scene(CaptureScene::LiveInspection);
    assert_eq!(first.image, second.image);
    assert_eq!(encode_png(&first.image), encode_png(&second.image));
    // The image is not a blank canvas: the brand chrome, the video and text are drawn.
    let distinct: std::collections::BTreeSet<&[u8; 4]> =
        first.image.pixels.as_chunks::<4>().0.iter().collect();
    assert!(
        distinct.len() > 64,
        "only {} colours rendered",
        distinct.len()
    );
}

/// Whole words of `text` (split on anything but ASCII alphanumerics, `_`, `-` and `.`).
fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .filter(|w| !w.is_empty())
        .collect()
}

/// RAC3: only the synthetic identity is shown; no host account, path or developer banner.
#[test]
fn test_rac_capture_scenes_show_only_the_synthetic_identity() {
    let host_login = nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()
        .map(|u| u.name);
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx);
    let [fw, fh] = readme_capture::CAPTURE_FRAME_SIZE;
    let frame = soos_test_fixtures::readme::create_placeholder_face_frame(fw, fh);
    for scene in CaptureScene::ALL {
        // Exact state checks, independent of the host account.
        let app = capture_app(&cc, scene, &frame).expect("capture scene");
        let ids = shown_identities(&app);
        assert_eq!(ids.enrollment_username, CAPTURE_USERNAME, "{scene:?}");
        assert_eq!(ids.enrollment_uid, CAPTURE_UID, "{scene:?}");
        assert!(
            ids.profile_usernames.iter().all(|u| u == CAPTURE_USERNAME),
            "{scene:?}: {:?}",
            ids.profile_usernames
        );
        assert_eq!(
            ids.profile_usernames.len(),
            usize::from(scene == CaptureScene::Profiles),
            "{scene:?}: one synthetic profile on the profiles screen only"
        );
        drop(app);

        // Rendered text.
        let capture = render_scene(scene);
        let all = capture.texts.join("\n");
        assert!(!all.is_empty(), "{scene:?} shows text");
        if let Some(login) = &host_login {
            if !ORDINARY_WORD_LOGINS.contains(&login.as_str()) {
                assert!(
                    !words(&all).contains(&login.as_str()),
                    "{scene:?} shows the host login"
                );
            }
        }
        for forbidden in [
            "/tmp",
            "/home/",
            "DEVELOPER STORE",
            "@",
            "/var/lib/soos",
            "/run/",
        ] {
            assert!(
                !all.contains(forbidden),
                "{scene:?} shows `{forbidden}`:\n{all}"
            );
        }
        if scene != CaptureScene::LiveInspection {
            assert!(
                words(&all).contains(&CAPTURE_USERNAME),
                "{scene:?} names {CAPTURE_USERNAME}:\n{all}"
            );
        }
        // Fixed telemetry, never a measured value (the header band moves the frame
        // statistics to a tooltip at this width; the telemetry card shows the latency).
        if scene == CaptureScene::LiveInspection {
            assert!(all.contains("12.5 ms"), "{scene:?}:\n{all}");
        }
        let measured: Vec<&str> = all
            .lines()
            .filter(|line| line.contains(" ms") && !line.contains("12.5 ms"))
            .collect();
        assert!(
            measured.is_empty(),
            "{scene:?} shows a measured latency: {measured:?}"
        );
    }
    assert_eq!(
        words("a/b alice-x c.d e"),
        ["a", "b", "alice-x", "c.d", "e"]
    );
}

/// RAC4: only an RGB24 frame with a consistent buffer is accepted.
#[test]
fn test_rac_capture_app_rejects_non_rgb_frames() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx);
    let yuyv = Frame::new(vec![0; 16 * 16 * 2], 16, 16, 1, PixelFormat::Yuyv, 1);
    let short = Frame::new(vec![0; 10], 16, 16, 1, PixelFormat::Rgb24, 1);
    let empty = Frame::new(Vec::new(), 0, 0, 1, PixelFormat::Rgb24, 1);
    for frame in [yuyv, short, empty] {
        assert!(matches!(
            capture_app(&cc, CaptureScene::LiveInspection, &frame),
            Err(CaptureError::UnsupportedFrame)
        ));
    }
    let names: Vec<&str> = CaptureScene::ALL.iter().map(|s| s.file_stem()).collect();
    assert_eq!(
        names,
        [
            "gui-live-inspection",
            "gui-guided-enrollment",
            "gui-profiles"
        ]
    );
}

// ---------------------------------------------------------------------------------------
// RAC5: rasterizer
// ---------------------------------------------------------------------------------------

fn white_texture(r: &mut Rasterizer, id: TextureId, image: ColorImage, options: TextureOptions) {
    let mut delta = TexturesDelta::default();
    delta.set.push((
        id,
        egui::epaint::ImageDelta::full(ImageData::Color(Arc::new(image)), options),
    ));
    r.apply(&delta);
}

fn quad(rect: Rect, color: Color32, texture: TextureId) -> ClippedPrimitive {
    let mut mesh = Mesh::with_texture(texture);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    mesh.add_rect_with_uv(rect, uv, color);
    ClippedPrimitive {
        clip_rect: Rect::EVERYTHING,
        primitive: Primitive::Mesh(mesh),
    }
}

fn px(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.pixels[i],
        img.pixels[i + 1],
        img.pixels[i + 2],
        img.pixels[i + 3],
    ]
}

fn plain_rasterizer() -> (Rasterizer, TextureId) {
    let mut r = Rasterizer::default();
    let id = TextureId::Managed(0);
    white_texture(
        &mut r,
        id,
        ColorImage::filled([1, 1], Color32::WHITE),
        TextureOptions::LINEAR,
    );
    (r, id)
}

/// RAC5: pixel-centre coverage, clip rectangle and premultiplied blending.
#[test]
fn test_rac_rasterizer_covers_clips_and_blends() {
    let (r, id) = plain_rasterizer();
    let red = quad(
        Rect::from_min_max(Pos2::new(1.0, 1.0), Pos2::new(3.0, 3.0)),
        Color32::RED,
        id,
    );
    let img = r.render(std::slice::from_ref(&red), 1.0, [4, 4], Color32::BLACK);
    assert_eq!(px(&img, 1, 1), [255, 0, 0, 255]);
    assert_eq!(px(&img, 2, 2), [255, 0, 0, 255]);
    assert_eq!(px(&img, 0, 0), [0, 0, 0, 255]);
    assert_eq!(px(&img, 3, 3), [0, 0, 0, 255]);

    // Clip to the left column of the quad.
    let mut clipped = red;
    clipped.clip_rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(2.0, 4.0));
    let img = r.render(&[clipped], 1.0, [4, 4], Color32::BLACK);
    assert_eq!(px(&img, 1, 1), [255, 0, 0, 255]);
    assert_eq!(px(&img, 2, 1), [0, 0, 0, 255]);

    // Half-transparent premultiplied white over blue: 0.5 * white + 0.5 * blue.
    let half = Color32::from_rgba_premultiplied(128, 128, 128, 128);
    let img = r.render(
        &[quad(
            Rect::from_min_max(Pos2::ZERO, Pos2::new(4.0, 4.0)),
            half,
            id,
        )],
        1.0,
        [4, 4],
        Color32::BLUE,
    );
    assert_eq!(px(&img, 2, 2), [128, 128, 255, 255]);

    // Pixels per point scales geometry.
    let img = r.render(
        &[quad(
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::RED,
            id,
        )],
        2.0,
        [4, 4],
        Color32::BLACK,
    );
    assert_eq!(px(&img, 1, 1), [255, 0, 0, 255]);
    assert_eq!(px(&img, 2, 2), [0, 0, 0, 255]);
}

/// RAC5: two triangles sharing an edge through pixel centres paint each pixel exactly once
/// (top-left rule): a half-transparent square stays uniform.
#[test]
fn test_rac_rasterizer_top_left_rule_paints_shared_edges_once() {
    let (r, id) = plain_rasterizer();
    let half = Color32::from_rgba_premultiplied(128, 0, 0, 128);
    let v = |x: f32, y: f32| Vertex {
        pos: Pos2::new(x, y),
        uv: Pos2::new(0.5, 0.5),
        color: half,
    };
    let mut mesh = Mesh::with_texture(id);
    mesh.vertices = vec![v(0.0, 0.0), v(4.0, 0.0), v(4.0, 4.0), v(0.0, 4.0)];
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    let prim = ClippedPrimitive {
        clip_rect: Rect::EVERYTHING,
        primitive: Primitive::Mesh(mesh),
    };
    let img = r.render(&[prim], 1.0, [4, 4], Color32::BLACK);
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(px(&img, x, y), [128, 0, 0, 255], "({x},{y}) painted once");
        }
    }
}

/// RAC5: bilinear texture fetch and partial texture updates.
#[test]
fn test_rac_rasterizer_samples_textures_and_applies_partial_deltas() {
    let mut r = Rasterizer::default();
    let id = TextureId::Managed(7);
    let image = ColorImage::new([2, 1], vec![Color32::BLACK, Color32::WHITE]);
    white_texture(&mut r, id, image, TextureOptions::NEAREST);
    let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(2.0, 1.0));
    let img = r.render(&[quad(full, Color32::WHITE, id)], 1.0, [2, 1], Color32::RED);
    assert_eq!(px(&img, 0, 0), [0, 0, 0, 255]);
    assert_eq!(px(&img, 1, 0), [255, 255, 255, 255]);

    // Linear filtering half-way between two texels.
    let mut r_lin = Rasterizer::default();
    let image = ColorImage::new([2, 1], vec![Color32::BLACK, Color32::WHITE]);
    white_texture(&mut r_lin, id, image, TextureOptions::LINEAR);
    let wide = Rect::from_min_max(Pos2::ZERO, Pos2::new(4.0, 1.0));
    let img = r_lin.render(&[quad(wide, Color32::WHITE, id)], 1.0, [4, 1], Color32::RED);
    assert_eq!(px(&img, 0, 0), [0, 0, 0, 255], "clamped to edge");
    assert_eq!(px(&img, 3, 0), [255, 255, 255, 255], "clamped to edge");
    let mid = px(&img, 1, 0)[0];
    assert!((50..=80).contains(&mid), "bilinear blend, got {mid}");

    // Partial update replaces the left texel.
    let mut delta = TexturesDelta::default();
    delta.set.push((
        id,
        egui::epaint::ImageDelta::partial(
            [0, 0],
            ImageData::Color(Arc::new(ColorImage::filled([1, 1], Color32::GREEN))),
            TextureOptions::NEAREST,
        ),
    ));
    r.apply(&delta);
    let img = r.render(&[quad(full, Color32::WHITE, id)], 1.0, [2, 1], Color32::RED);
    assert_eq!(px(&img, 0, 0), [0, 255, 0, 255]);
    // Freed textures are gone.
    let mut free = TexturesDelta::default();
    free.free.push(id);
    r.free(&free);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        r.render(&[quad(full, Color32::WHITE, id)], 1.0, [2, 1], Color32::RED)
    }));
    assert!(result.is_err(), "a freed texture cannot be painted");
}

// ---------------------------------------------------------------------------------------
// RAC6: PNG writer
// ---------------------------------------------------------------------------------------

/// RAC6: the writer emits only IHDR, IDAT and IEND (no text, time or EXIF chunk).
#[test]
fn test_rac_png_writer_emits_only_critical_chunks() {
    let image = RgbaImage {
        width: 3,
        height: 2,
        pixels: vec![255; 3 * 2 * 4],
    };
    let bytes = encode_png(&image);
    let chunks = png_chunk_types(&bytes);
    assert_eq!(chunks.first().map(String::as_str), Some("IHDR"));
    assert_eq!(chunks.last().map(String::as_str), Some("IEND"));
    assert!(
        chunks
            .iter()
            .all(|c| matches!(c.as_str(), "IHDR" | "IDAT" | "IEND")),
        "{chunks:?}"
    );
}

/// RAC6: encode → decode is lossless for opaque images and the diff detects changes.
#[test]
fn test_rac_png_round_trip() {
    let pixels: Vec<u8> = (0..(7 * 5))
        .flat_map(|i: u32| [(i * 7) as u8, (i * 13) as u8, (i * 29) as u8, 255])
        .collect();
    let image = RgbaImage {
        width: 7,
        height: 5,
        pixels,
    };
    let decoded = decode_png(&encode_png(&image)).unwrap();
    assert_eq!(decoded, image);
    assert_eq!(diff_fraction(&image, &decoded, 0), Some(0.0));
    let mut changed = image.clone();
    changed.pixels[0] = changed.pixels[0].wrapping_add(100);
    assert_eq!(diff_fraction(&image, &changed, 8), Some(1.0 / 35.0));
    assert_eq!(diff_fraction(&image, &changed, 255), Some(0.0));
    let other = RgbaImage {
        width: 5,
        height: 7,
        pixels: image.pixels.clone(),
    };
    assert_eq!(diff_fraction(&image, &other, 0), None);
    assert!(decode_png(b"not a png").is_err());
}

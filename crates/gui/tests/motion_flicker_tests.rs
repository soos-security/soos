//! Contract tests for the two motion follow-ups of PR #449 (GitHub #438 = PR 3 of #358, matrix
//! GMO21–GMO22; `Docs/UI_MOTION.md` §1 principle 6, §4, §7 G4 and G7, §9).
//!
//! Contract:
//! - GMO21 (G4): a stat tile whose caption (or em dash / value presence) flickers at camera rate
//!   never stays faint. A change fades the value in only when the previous content had been on
//!   screen for at least twice the fade duration; a change during a running fade keeps that fade
//!   running (it never restarts it), and a change shortly after a fade is drawn at once. So the
//!   opacity never falls back once a flicker starts, reaches 1 within the fade duration of the
//!   first change, and motion code then requests no repaint while the flicker goes on.
//! - GMO22 (G7): a Warning or Error camera status is drawn at full opacity, in its own colour, on
//!   the frame it arrives (no fade, no title colour cross-fade); only Info states fade in.

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
    reason = "Contractual test suite utilizes direct assertions, unwrap, indexing and exact float checks"
)]

use std::time::Duration;

use eframe::egui::{self, pos2, vec2, Color32, Id, Rect};
use soos_camera_v4l::{CameraErrorKind, CameraStatus};
use soos_gui::camera_status::{camera_status_banner, render_status_banner, BannerSeverity};
use soos_gui::motion::{self, MotionPrefs};
use soos_gui::theme;
use soos_gui::widgets;

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

const DT: f32 = 1.0 / 60.0;

/// One headless pass at 60 Hz; returns the painted shapes and the repaint delay egui asks for
/// afterwards (`Duration::MAX` = no repaint requested).
fn run(
    ctx: &egui::Context,
    mut f: impl FnMut(&mut egui::Ui),
) -> (Vec<egui::epaint::ClippedShape>, Duration) {
    let input = egui::RawInput {
        predicted_dt: DT,
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| f(ui));
    let delay = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map_or(Duration::MAX, |v| v.repaint_delay);
    (output.shapes, delay)
}

/// A context with the brand theme, `prefs` installed and a few warm-up passes done, so the
/// first-frame repaints egui itself requests are out of the way.
fn ctx_with(prefs: MotionPrefs) -> egui::Context {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    motion::install_prefs(&ctx, prefs);
    for _ in 0..5 {
        let _ = run(&ctx, |_| {});
    }
    ctx
}

/// Every text shape painted, with its text.
fn text_shapes(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::epaint::TextShape)> {
    shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.clone())),
            _ => None,
        })
        .collect()
}

/// Number of 60 Hz frames a fade of `seconds` needs (plus one frame of slack).
fn frames_for(seconds: f32) -> usize {
    (seconds / DT).ceil() as usize + 1
}

/// Opacity returned by `fade_in_on_stable_change` for one pass showing `key`.
fn stable_fade(ctx: &egui::Context, id: Id, key: u64) -> (f32, Duration) {
    let mut o = f32::NAN;
    let (_, delay) = run(ctx, |ui| {
        o = motion::fade_in_on_stable_change(ui.ctx(), id, key, theme::T_SHORT);
    });
    (o, delay)
}

// ---------------------------------------------------------------------------------------------
// GMO21 — a flickering stat tile never stays faint
// ---------------------------------------------------------------------------------------------

/// GMO21: first sight is at rest; a change after a stable period fades in over the duration and
/// then stops repainting, like `fade_in_on_change`.
#[test]
fn test_gmo21_stable_change_fades_in_and_settles() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let id = Id::new("gmo21_stable");
    let (o, _) = stable_fade(&ctx, id, 1);
    assert_eq!(o, 1.0, "first sight is at rest");
    for _ in 0..frames_for(2.0 * theme::T_SHORT) {
        let (o, delay) = stable_fade(&ctx, id, 1);
        assert_eq!(o, 1.0);
        assert_eq!(delay, Duration::MAX, "an unchanged key requests no repaint");
    }
    let (o, delay) = stable_fade(&ctx, id, 2);
    assert!(o < 1.0, "a change after a stable period fades in, got {o}");
    assert_eq!(delay, Duration::ZERO, "a running fade repaints every frame");
    let mut last = o;
    let mut settled = None;
    for frame in 0..frames_for(theme::T_SHORT) {
        let (o, delay) = stable_fade(&ctx, id, 2);
        assert!(o >= last, "the fade never goes back ({o} < {last})");
        last = o;
        if o == 1.0 && delay == Duration::MAX {
            settled = Some(frame);
            break;
        }
    }
    assert!(
        settled.is_some(),
        "the fade reaches 1 within T_SHORT and stops repainting"
    );
}

/// GMO21: a key flipping every frame (camera-rate face detection flicker) after a stable period
/// starts one fade only: the opacity never falls back, reaches 1 within `T_SHORT` of the first
/// flip and stays there, and motion code then requests no repaint although the key keeps
/// flipping.
#[test]
fn test_gmo21_flicker_every_frame_never_restarts_the_fade() {
    for period in [1_usize, 2, 3, 6, 12] {
        let ctx = ctx_with(MotionPrefs::FULL);
        let id = Id::new(("gmo21_flicker", period));
        for _ in 0..frames_for(2.0 * theme::T_SHORT) {
            let _ = stable_fade(&ctx, id, 10);
        }
        let mut last = 0.0_f32;
        let mut full_at = None;
        let total = frames_for(2.0);
        for frame in 0..total {
            // Flip between two keys every `period` frames, starting with a change.
            let key = if (frame / period).is_multiple_of(2) {
                11
            } else {
                10
            };
            let (o, delay) = stable_fade(&ctx, id, key);
            assert!(
                o >= last,
                "period {period}, frame {frame}: the fade restarted ({o} < {last})"
            );
            last = o;
            if o == 1.0 && full_at.is_none() {
                full_at = Some(frame);
            }
            if let Some(at) = full_at {
                assert_eq!(o, 1.0, "period {period}, frame {frame}: faint again");
                if frame > at {
                    assert_eq!(
                        delay,
                        Duration::MAX,
                        "period {period}, frame {frame}: the flicker keeps repainting"
                    );
                }
            }
        }
        let at = full_at.expect("the flickering value never reached full opacity");
        assert!(
            at <= frames_for(theme::T_SHORT),
            "period {period}: full opacity only after {at} frames"
        );
    }
}

/// GMO21: once a change is drawn, the next change fades again only after the new content was
/// stable for twice the fade duration; a change sooner than that is drawn at once.
#[test]
fn test_gmo21_change_soon_after_a_fade_is_drawn_at_once_and_late_change_fades() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let id = Id::new("gmo21_hold");
    for _ in 0..frames_for(2.0 * theme::T_SHORT) {
        let _ = stable_fade(&ctx, id, 1);
    }
    let (o, _) = stable_fade(&ctx, id, 2);
    assert!(o < 1.0);
    // Let the fade finish but stay short of the stable hold (2 x T_SHORT).
    for _ in 0..frames_for(theme::T_SHORT) {
        let _ = stable_fade(&ctx, id, 2);
    }
    let (o, delay) = stable_fade(&ctx, id, 3);
    assert_eq!(o, 1.0, "a change soon after a fade is drawn at once");
    assert_eq!(delay, Duration::MAX);
    // Now hold key 3 long enough: the next change fades again.
    for _ in 0..frames_for(2.0 * theme::T_SHORT) {
        let _ = stable_fade(&ctx, id, 3);
    }
    let (o, _) = stable_fade(&ctx, id, 4);
    assert!(
        o < 1.0,
        "a change after a stable period fades again, got {o}"
    );
}

/// GMO21: reduced motion draws every change at rest and requests no repaint.
#[test]
fn test_gmo21_reduced_motion_is_at_rest() {
    let ctx = ctx_with(MotionPrefs::REDUCED);
    let id = Id::new("gmo21_reduced");
    for frame in 0..40_u64 {
        let (o, delay) = stable_fade(&ctx, id, frame % 3);
        assert_eq!(o, 1.0);
        assert_eq!(delay, Duration::MAX);
    }
}

/// GMO21 wiring: a stat tile whose caption flips every frame (Match score / Liveness) draws its
/// value at full opacity at most `T_SHORT` after the first flip, and keeps it there.
#[test]
fn test_gmo21_flickering_stat_tile_value_reaches_full_opacity() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let rect = Rect::from_min_size(pos2(20.0, 20.0), vec2(260.0, 260.0));
    let draw = |frame: usize| {
        move |ui: &mut egui::Ui| {
            if frame.is_multiple_of(2) {
                widgets::stat_tile(
                    ui,
                    rect,
                    "Match score",
                    Some("81%"),
                    Some("Cosine 0.8100"),
                    None,
                );
            } else {
                widgets::stat_tile(ui, rect, "Liveness (PAD)", Some("97%"), None, None);
            }
        }
    };
    let value_opacity = |shapes: &[egui::epaint::ClippedShape]| -> f32 {
        let values: Vec<f32> = text_shapes(shapes)
            .into_iter()
            .filter(|(text, _)| text == "81%" || text == "97%")
            .map(|(_, t)| t.opacity_factor)
            .collect();
        // egui paints nothing at opacity 0: an absent value is fully transparent.
        if values.is_empty() {
            return 0.0;
        }
        values.into_iter().fold(1.0_f32, f32::min)
    };
    // Stable first: the "Match score" tile is on screen long enough.
    for _ in 0..frames_for(2.0 * theme::T_SHORT) {
        let (shapes, _) = run(&ctx, draw(0));
        assert_eq!(value_opacity(&shapes), 1.0);
    }
    let mut full_at = None;
    for frame in 1..frames_for(2.0) {
        let (shapes, _) = run(&ctx, draw(frame));
        let o = value_opacity(&shapes);
        if o == 1.0 && full_at.is_none() {
            full_at = Some(frame);
        }
        if full_at.is_some() {
            assert_eq!(o, 1.0, "frame {frame}: the flickering tile is faint again");
        }
    }
    let at = full_at.expect("the flickering tile never reached full opacity");
    assert!(
        at <= frames_for(theme::T_SHORT),
        "full opacity only after {at} frames"
    );
}

// ---------------------------------------------------------------------------------------------
// GMO22 — warnings and errors are drawn at full opacity on arrival
// ---------------------------------------------------------------------------------------------

/// Renders `status` for `frames` passes and returns the shapes of the last one.
fn banner_frames(
    ctx: &egui::Context,
    status: &CameraStatus,
    frames: usize,
) -> Vec<egui::epaint::ClippedShape> {
    let banner = camera_status_banner(status);
    let mut last = Vec::new();
    for _ in 0..frames {
        let (shapes, _) = run(ctx, |ui| render_status_banner(ui, &banner));
        last = shapes;
    }
    last
}

/// Asserts that the arrival frame of `status` draws every text at full opacity and the title in
/// `color`.
fn assert_drawn_at_once(
    shapes: &[egui::epaint::ClippedShape],
    status: &CameraStatus,
    color: Color32,
) {
    let banner = camera_status_banner(status);
    let texts = text_shapes(shapes);
    assert!(!texts.is_empty(), "the banner painted no text");
    for (text, t) in &texts {
        assert_eq!(
            t.opacity_factor, 1.0,
            "{:?}: '{text}' is faded on its arrival frame",
            banner.severity
        );
    }
    let title: Vec<_> = texts
        .iter()
        .filter(|(text, _)| !text.trim().is_empty() && banner.title.contains(text.trim()))
        .collect();
    assert!(
        !title.is_empty(),
        "the title '{}' was not painted",
        banner.title
    );
    for (text, t) in title {
        assert_eq!(
            t.fallback_color, color,
            "{:?}: title line '{text}' is not in its own colour on arrival",
            banner.severity
        );
    }
}

/// GMO22: an Error arriving after a stable Info state is drawn at full opacity in `DANGER` on
/// that same frame, for every error kind.
#[test]
fn test_gmo22_error_status_is_drawn_at_full_opacity_on_arrival() {
    for kind in CameraErrorKind::ALL {
        let ctx = ctx_with(MotionPrefs::FULL);
        let _ = banner_frames(&ctx, &CameraStatus::Ready, frames_for(1.0));
        let error = CameraStatus::Error { kind, failures: 1 };
        assert_eq!(camera_status_banner(&error).severity, BannerSeverity::Error);
        let shapes = banner_frames(&ctx, &error, 1);
        assert_drawn_at_once(&shapes, &error, theme::DANGER);
    }
}

/// GMO22: a Warning arriving after an Info state, and an Error arriving after a Warning, are
/// drawn at full opacity in their own colour on that frame.
#[test]
fn test_gmo22_warning_status_is_drawn_at_full_opacity_on_arrival() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let _ = banner_frames(&ctx, &CameraStatus::Starting, frames_for(1.0));
    for warning in [CameraStatus::Suspended, CameraStatus::Stopped] {
        assert_eq!(
            camera_status_banner(&warning).severity,
            BannerSeverity::Warning
        );
        let shapes = banner_frames(&ctx, &warning, 1);
        assert_drawn_at_once(&shapes, &warning, theme::WARN_TEXT);
        let _ = banner_frames(&ctx, &warning, frames_for(1.0));
    }
    let error = CameraStatus::Error {
        kind: CameraErrorKind::DeviceNotFound,
        failures: 3,
    };
    let shapes = banner_frames(&ctx, &error, 1);
    assert_drawn_at_once(&shapes, &error, theme::DANGER);
}

/// GMO22: Info states keep the G7 fade: an Info state arriving after an Error fades in (some text
/// below full opacity on its arrival frame) and is at rest once the fade has run.
#[test]
fn test_gmo22_info_status_still_fades_in() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let error = CameraStatus::Error {
        kind: CameraErrorKind::DeviceNotFound,
        failures: 1,
    };
    let _ = banner_frames(&ctx, &error, frames_for(1.0));
    let ready = camera_status_banner(&CameraStatus::Ready);
    let shapes = banner_frames(&ctx, &CameraStatus::Ready, 1);
    // egui paints nothing at opacity 0, so a fading title is either absent or below 1.
    assert!(
        !text_shapes(&shapes).iter().any(|(text, t)| {
            !text.trim().is_empty() && ready.title.contains(text.trim()) && t.opacity_factor == 1.0
        }),
        "an Info state fades in on its arrival frame"
    );
    let shapes = banner_frames(&ctx, &CameraStatus::Ready, frames_for(theme::T_SHORT));
    for (text, t) in text_shapes(&shapes) {
        assert_eq!(t.opacity_factor, 1.0, "'{text}' still faded after T_SHORT");
    }
}

/// GMO22: with reduced motion an Error is drawn at once as well.
#[test]
fn test_gmo22_reduced_motion_error_at_once() {
    let ctx = ctx_with(MotionPrefs::REDUCED);
    let _ = banner_frames(&ctx, &CameraStatus::Ready, 3);
    let error = CameraStatus::Error {
        kind: CameraErrorKind::PermissionDenied,
        failures: 1,
    };
    let shapes = banner_frames(&ctx, &error, 1);
    assert_drawn_at_once(&shapes, &error, theme::DANGER);
}

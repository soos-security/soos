//! Brand theme contract tests for `soos-gui` (matrix GUX1..GUX8).
//!
//! Validates:
//! - The palette tokens equal the brand hex values of the design direction
//! - The installed egui visuals use the pale body, ink text and blue selection
//! - The responsive metrics follow the 1512-wide reference and the 900-wide minimum
//! - The daemon switch maps the daemon state and the busy flag exactly like the old
//!   Pause/Resume buttons (same privileged actions, disabled while busy, inert when unknown)
//! - The wordmark and star geometry stay inside their target rectangles
//! - The procedural window icon has the requested size and the brand colors
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::float_cmp,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use eframe::egui::{self, pos2, vec2, Color32, Rect};
use soos_gui::brand;
use soos_gui::daemon_control::DaemonState;
use soos_gui::header::DaemonSwitch;
use soos_gui::privileged::PrivilegedAction;
use soos_gui::theme;

fn rgb(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

#[test]
fn test_gux1_palette_tokens_match_brand_hex_values() {
    assert_eq!(rgb(theme::BLUE), [0x00, 0x47, 0xBB], "brand blue #0047BB");
    assert_eq!(
        rgb(theme::PALE),
        [0xED, 0xF1, 0xFF],
        "Brilliant White #EDF1FF"
    );
    assert_eq!(rgb(theme::INK), [0x10, 0x18, 0x20], "ink #101820");
    assert_eq!(rgb(theme::PINK), [0xE5, 0x9B, 0xDC], "pink #E59BDC");
    for c in [theme::BLUE, theme::PALE, theme::INK, theme::PINK] {
        assert_eq!(c.a(), 255, "brand tokens are opaque");
    }
}

#[test]
fn test_gux2_visuals_use_pale_body_ink_text_and_blue_selection() {
    let v = theme::visuals();
    assert!(!v.dark_mode, "the brand theme is a light theme");
    assert_eq!(v.panel_fill, theme::PALE);
    assert_eq!(v.window_fill, theme::PALE);
    assert_eq!(v.selection.bg_fill, theme::BLUE);
    assert_eq!(v.widgets.noninteractive.fg_stroke.color, theme::INK);
    assert_eq!(v.widgets.inactive.fg_stroke.color, theme::INK);
    assert_eq!(v.window_stroke.color, theme::BLUE);
    assert!(v.window_corner_radius.nw >= 12, "windows are rounded");
}

#[test]
fn test_gux2_apply_installs_the_light_brand_theme() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let _ = ctx.run_ui(Default::default(), |ui| {
        assert_eq!(ui.visuals().panel_fill, theme::PALE);
        assert_eq!(ui.visuals().selection.bg_fill, theme::BLUE);
    });
}

#[test]
fn test_gux3_metrics_follow_reference_and_minimum_window() {
    let reference = theme::Metrics::for_width(1512.0);
    assert_eq!(reference.pad, 40.0);
    assert_eq!(reference.gap_main, 40.0);
    assert_eq!(reference.col_w, 259.0);
    assert_eq!(reference.header_h, 44.0);

    let default_window = theme::Metrics::for_width(1120.0);
    assert!((default_window.pad - 29.63).abs() < 0.1);
    assert_eq!(default_window.col_w, 250.0);

    let minimum = theme::Metrics::for_width(900.0);
    assert!(minimum.pad >= 20.0 && minimum.pad <= 25.0);
    assert!(minimum.col_w >= 250.0);

    let wide = theme::Metrics::for_width(3000.0);
    assert_eq!(wide.pad, 40.0, "the scale never exceeds the reference");
}

#[test]
fn test_gux4_daemon_switch_active_is_on_and_requests_pause() {
    let switch = DaemonSwitch::new(DaemonState::Active, false);
    assert_eq!(switch.position, Some(true));
    assert!(switch.interactive);
    assert!(!switch.busy);
    assert_eq!(switch.label, "Daemon Active");
    assert!(matches!(
        switch.click_action(),
        Some(PrivilegedAction::PauseDaemon)
    ));
}

#[test]
fn test_gux4_daemon_switch_inactive_is_off_and_requests_resume() {
    let switch = DaemonSwitch::new(DaemonState::Inactive, false);
    assert_eq!(switch.position, Some(false));
    assert!(switch.interactive);
    assert_eq!(switch.label, "Daemon Paused");
    assert!(matches!(
        switch.click_action(),
        Some(PrivilegedAction::ResumeDaemon)
    ));
}

#[test]
fn test_gux4_daemon_switch_unknown_is_indeterminate_and_inert() {
    for busy in [false, true] {
        let switch = DaemonSwitch::new(DaemonState::Unknown, busy);
        assert_eq!(switch.position, None, "unknown state shows no knob");
        assert!(!switch.interactive);
        assert!(switch.click_action().is_none(), "no action without a state");
    }
    assert_eq!(
        DaemonSwitch::new(DaemonState::Unknown, false).label,
        "Daemon status unknown"
    );
}

#[test]
fn test_gux4_daemon_switch_busy_is_disabled_and_announces_authorization() {
    for state in [DaemonState::Active, DaemonState::Inactive] {
        let switch = DaemonSwitch::new(state, true);
        assert!(switch.busy);
        assert!(!switch.interactive, "never stack Polkit dialogs");
        assert!(switch.click_action().is_none());
        assert_eq!(switch.label, "Waiting for authorization...");
    }
}

#[test]
fn test_gux5_wordmark_geometry_fits_inside_target_rect() {
    let target = Rect::from_min_size(pos2(40.0, 13.0), vec2(300.0, 18.0));
    let fitted = brand::fit_aspect(target, brand::WORDMARK_VIEWBOX);
    assert!(target.expand(0.01).contains_rect(fitted));
    assert!((fitted.height() - 18.0).abs() < 0.01, "height-bound fit");
    assert!((fitted.width() - 18.0 * 514.0 / 64.0).abs() < 0.05);

    let polylines = brand::wordmark_polylines(fitted);
    assert!(polylines.len() >= 6, "every wordmark subpath is flattened");
    for poly in &polylines {
        assert!(poly.len() >= 3);
        for p in poly {
            assert!(fitted.expand(0.01).contains(*p), "{p:?} escapes {fitted:?}");
        }
    }
}

#[test]
fn test_gux5_star_geometry_fits_inside_target_rect() {
    let target = Rect::from_min_size(pos2(10.0, 20.0), vec2(259.0, 300.0));
    let fitted = brand::fit_aspect(target, brand::STAR_VIEWBOX);
    assert!(target.expand(0.01).contains_rect(fitted));
    assert!((fitted.width() - 259.0).abs() < 0.01, "width-bound fit");
    for poly in brand::star_polylines(fitted) {
        for p in &poly {
            assert!(fitted.expand(0.01).contains(*p));
        }
    }
}

#[test]
fn test_gux6_rasterizer_covers_squares_exactly() {
    let full = vec![vec![
        pos2(0.0, 0.0),
        pos2(10.0, 0.0),
        pos2(10.0, 10.0),
        pos2(0.0, 10.0),
    ]];
    let cov = brand::rasterize(&[full], vec2(10.0, 10.0), 10, 10);
    assert_eq!(cov.len(), 100);
    assert!(cov.iter().all(|&c| c == 255));

    let left_half = vec![vec![
        pos2(0.0, 0.0),
        pos2(5.0, 0.0),
        pos2(5.0, 10.0),
        pos2(0.0, 10.0),
    ]];
    let cov = brand::rasterize(&[left_half], vec2(10.0, 10.0), 10, 10);
    assert_eq!(cov[0], 255);
    assert_eq!(cov[4], 255);
    assert_eq!(cov[5], 0);
    assert_eq!(cov[99], 0);
}

#[test]
fn test_gux6_wordmark_mask_has_ink_and_holes() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(257.0, 32.0));
    let paths = brand::wordmark_paths(rect);
    let cov = brand::rasterize(&paths, vec2(257.0, 32.0), 257, 32);
    let covered = cov.iter().filter(|&&c| c > 200).count();
    let empty = cov.iter().filter(|&&c| c == 0).count();
    assert!(covered > 257 * 32 / 4, "the letters are filled");
    assert!(empty > 257 * 32 / 8, "the counters stay open");
}

#[test]
fn test_gux7_window_icon_has_requested_size_and_brand_colors() {
    let icon = brand::window_icon(128);
    assert_eq!(icon.width, 128);
    assert_eq!(icon.height, 128);
    assert_eq!(icon.rgba.len(), 128 * 128 * 4);
    let px = |x: usize, y: usize| {
        let i = (y * 128 + x) * 4;
        [
            icon.rgba[i],
            icon.rgba[i + 1],
            icon.rgba[i + 2],
            icon.rgba[i + 3],
        ]
    };
    let blue = [0x00, 0x47, 0xBB, 255];
    let pale = [0xED, 0xF1, 0xFF, 255];
    assert_eq!(px(20, 20), blue, "top-left square");
    assert_eq!(px(108, 108), blue, "bottom-right square");
    assert_eq!(px(124, 3), blue, "top-right quarter disc");
    assert_eq!(px(3, 124), blue, "bottom-left quarter disc");
    assert_eq!(px(70, 58), pale, "top-right star spike");
    assert_eq!(px(58, 70), pale, "bottom-left star spike");
    assert!(icon.rgba.iter().skip(3).step_by(4).all(|&a| a == 255));
}

#[test]
fn test_gux8_header_tab_labels_fix_the_mockup_typo() {
    assert_eq!(
        soos_gui::header::TAB_LABELS,
        [
            "Live Model Diagnostic",
            "Guided Enrollment",
            "Biometric Profiles"
        ]
    );
}

#[test]
fn test_gux8_brand_widgets_render_in_a_bare_context() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let _ = ctx.run_ui(Default::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(259.0, 259.0));
            soos_gui::widgets::stat_tile(ui, rect, "MATCH SCORE", Some("80%"), None, None);
            soos_gui::widgets::star_tile(ui, rect.translate(vec2(0.0, 270.0)));
            soos_gui::widgets::banner(ui, soos_gui::widgets::BannerKind::Warning, "DEVELOPER");
            brand::paint_wordmark(
                ui,
                Rect::from_min_size(pos2(0.0, 0.0), vec2(146.0, 18.0)),
                theme::PALE,
            );
        });
    });
}

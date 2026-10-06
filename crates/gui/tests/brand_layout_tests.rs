//! Brand redesign layout and rendering-helper contracts (VERIFICATION_MATRIX GUX9..GUX13).
//!
//! - GUX9: faux-bold passes land on distinct whole physical pixels at any scale.
//! - GUX10: the brand progress bar paints no fill at 0 % and keeps its label readable.
//! - GUX11: the right column shrinks the stat tile before the card scrolls; the Profiles
//!   column always ends level with the main card.
//! - GUX12: the real brand geometry (header chrome, `Metrics::split_columns`, centered video
//!   fit) keeps the CLP4 / GARP3 minimum canvas sizes, developer banner included.
//! - GUX13: the brand path mapping never panics on an inverted or non-finite rectangle.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::float_cmp,
    reason = "Contractual integration tests use assertions, indexing and exact float checks"
)]

use eframe::egui::{pos2, vec2, Rect};
use soos_gui::{brand, theme, widgets};

/// Height allowance of the developer-store banner plus its gap (two wrapped lines).
const DEV_BANNER_ALLOWANCE: f32 = 80.0;

fn page_main_area(window_w: f32, window_h: f32) -> Rect {
    let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(window_w, window_h));
    let metrics = theme::Metrics::for_width(window_w);
    let mut content = theme::content_rect(full, &metrics);
    content.min.y += DEV_BANNER_ALLOWANCE;
    metrics.split_columns(content).0
}

#[test]
fn test_gux9_faux_bold_offsets_are_whole_distinct_pixels() {
    for ppp in [1.0_f32, 1.25, 1.5, 2.0] {
        for (passes, step) in [(2_u8, 0.4_f32), (2, 0.6), (6, 0.6), (6, 1.8)] {
            let offsets = theme::faux_bold_offsets(passes, step, ppp);
            assert!(
                offsets.len() >= 2,
                "{passes} passes at {step} must stay bold"
            );
            assert_eq!(offsets[0], 0.0);
            for pair in offsets.windows(2) {
                assert!(pair[1] > pair[0], "offsets must be strictly increasing");
            }
            for o in &offsets {
                let px = o * ppp;
                assert!(
                    (px - px.round()).abs() < 1e-4,
                    "offset {o} not on the pixel grid"
                );
            }
        }
    }
    assert_eq!(theme::faux_bold_offsets(0, 0.5, 1.0), vec![0.0]);
    assert_eq!(
        theme::faux_bold_offsets(2, f32::NAN, f32::NAN),
        vec![0.0, 1.0]
    );
}

#[test]
fn test_gux10_progress_bar_has_no_fill_at_zero_and_a_readable_label() {
    assert_eq!(widgets::progress_fill_width(0.0, 200.0, 18.0), 0.0);
    assert_eq!(widgets::progress_fill_width(f32::NAN, 200.0, 18.0), 0.0);
    assert_eq!(widgets::progress_fill_width(-1.0, 200.0, 18.0), 0.0);
    assert_eq!(widgets::progress_fill_width(0.01, 200.0, 18.0), 18.0);
    assert_eq!(widgets::progress_fill_width(0.5, 200.0, 18.0), 100.0);
    assert_eq!(widgets::progress_fill_width(2.0, 200.0, 18.0), 200.0);
    assert_eq!(
        widgets::progress_label_placement(0.0, 24.0),
        widgets::ProgressLabel::TrackEnd
    );
    assert_eq!(
        widgets::progress_label_placement(30.0, 24.0),
        widgets::ProgressLabel::TrackEnd
    );
    assert_eq!(
        widgets::progress_label_placement(100.0, 24.0),
        widgets::ProgressLabel::InsideFill
    );
}

#[test]
fn test_gux11_column_stack_shrinks_the_stat_tile_before_the_card_scrolls() {
    let column = Rect::from_min_size(pos2(0.0, 0.0), vec2(250.0, 600.0));
    // Plenty of room: square stat tile, star tile shown.
    let tall = Rect::from_min_size(pos2(0.0, 0.0), vec2(250.0, 900.0));
    let (card, stat, star) = widgets::column_stack(tall, 14.0, 300.0);
    assert!(star.is_some());
    assert_eq!(stat.unwrap().height(), 250.0);
    assert!(card.height() >= 300.0);
    // Card needs 400: the stat tile gives up height (600 - 400 - 14 = 186), no star.
    let (card, stat, star) = widgets::column_stack(column, 14.0, 400.0);
    assert!(star.is_none());
    assert_eq!(stat.unwrap().height(), 186.0);
    assert!(card.height() >= 400.0);
    // Card needs more than the column can give: stat tile at its 140-point floor.
    let (_, stat, _) = widgets::column_stack(column, 14.0, 900.0);
    assert_eq!(stat.unwrap().height(), widgets::STAT_TILE_MIN);
    // Stack always ends at the column bottom.
    let (_, stat, _) = widgets::column_stack(column, 14.0, 400.0);
    assert!((stat.unwrap().max.y - column.max.y).abs() < 0.01);
}

#[test]
fn test_gux11_profiles_column_ends_level_with_the_main_card() {
    // Tall column: square stat tile and a star tile filling the rest.
    let (stat, star) = widgets::profiles_column(800.0, 259.0, 14.0);
    assert_eq!(stat, 259.0);
    assert_eq!(stat + 14.0 + star.unwrap(), 800.0);
    // Short column: the stat tile stretches over the whole height.
    let (stat, star) = widgets::profiles_column(330.0, 250.0, 14.0);
    assert!(star.is_none());
    assert_eq!(stat, 330.0);
}

#[test]
fn test_gux12_live_canvas_keeps_clp4_minimums_with_the_dev_banner() {
    let live = theme::fit_video(page_main_area(1120.0, 780.0), 4.0 / 3.0);
    assert!(
        live.width() >= 500.0 && live.height() >= 350.0,
        "1120x780: {live:?}"
    );
    let small = theme::fit_video(page_main_area(900.0, 600.0), 4.0 / 3.0);
    assert!(
        small.width() >= 450.0 && small.height() >= 300.0,
        "900x600: {small:?}"
    );
}

#[test]
fn test_gux12_enrollment_canvas_keeps_garp3_minimums_with_the_dev_banner() {
    for (w, h) in [(1120.0, 780.0), (900.0, 600.0)] {
        let canvas = theme::fit_video(page_main_area(w, h), 16.0 / 9.0);
        assert!(
            canvas.width() >= 450.0 && canvas.height() >= 250.0,
            "{w}x{h}: {canvas:?}"
        );
    }
}

#[test]
fn test_gux12_video_is_centered_in_the_main_area_and_never_overflows() {
    for (w, h) in [
        (900.0, 600.0),
        (1120.0, 780.0),
        (1512.0, 982.0),
        (1800.0, 700.0),
    ] {
        let main = page_main_area(w, h);
        let video = theme::fit_video(main, 4.0 / 3.0);
        assert!((video.center().x - main.center().x).abs() < 0.01);
        assert!(main.expand(0.01).contains_rect(video), "{w}x{h}");
    }
    // Degenerate aspect falls back to 4:3 instead of producing NaN.
    let main = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    assert_eq!(theme::fit_video(main, f32::NAN).size(), vec2(400.0, 300.0));
}

#[test]
fn test_gux13_brand_paths_tolerate_inverted_and_non_finite_rects() {
    let inverted = Rect::from_min_max(pos2(100.0, 50.0), pos2(0.0, 0.0));
    let normal = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 50.0));
    for poly in brand::star_polylines(inverted) {
        for p in poly {
            assert!(normal.expand(0.01).contains(p));
        }
    }
    assert!(!brand::wordmark_polylines(inverted).is_empty());
    let nan = Rect::from_min_max(pos2(f32::NAN, 0.0), pos2(10.0, 10.0));
    assert!(brand::wordmark_paths(nan).is_empty());
    assert!(brand::star_paths(nan).is_empty());
}

#[test]
fn test_gux11_card_first_stack_drops_the_tiles_before_the_card_scrolls() {
    let column = Rect::from_min_size(pos2(0.0, 0.0), vec2(250.0, 600.0));
    // 600 < 500 + 140 + 14: the card takes the whole column.
    let (card, stat, star) = widgets::column_stack_card_first(column, 14.0, 500.0);
    assert_eq!(card, column);
    assert!(stat.is_none() && star.is_none());
    // Room for the card and a shrunk stat tile: same as column_stack.
    assert_eq!(
        widgets::column_stack_card_first(column, 14.0, 400.0),
        widgets::column_stack(column, 14.0, 400.0)
    );
}

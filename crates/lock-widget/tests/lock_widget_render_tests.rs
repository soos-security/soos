//! Contract tests of the widget geometry, memory bounds, CPU drawing and painting (GitHub #401;
//! matrix LKA9, LKA10).

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
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_lock_widget::font::{glyph, text_width, GLYPH_GAP, GLYPH_H, GLYPH_W};
use soos_lock_widget::layout::{
    buffer_len, Layout, Rect, BUFFERS_PER_SURFACE, MAX_OUTPUTS, MAX_SURFACE_DIM,
    MAX_SURFACE_PIXELS, MAX_TOTAL_PIXELS, WIDGET_CENTER_Y_FRACTION,
};
use soos_lock_widget::palette::{
    mix, Rgb, BACKGROUND, BLUE, LABEL, PALE, PALE_4, PINK, RING_IDLE, RING_SCANNING, STAR,
    WARN_BORDER,
};
use soos_lock_widget::raster::{star_contains, Canvas};
use soos_lock_widget::render::{paint_full, paint_widget};
use soos_lock_widget::scene::FrameSpec;
use soos_lockstatus::format::{NOT_RECOGNIZED_TEXT, SCANNING_TEXT};
use soos_lockstatus::motion::SLIDE_PX;

const MAGENTA: Rgb = Rgb(0xFF, 0x00, 0xFF);

fn buffer(w: u32, h: u32) -> Vec<u8> {
    vec![0u8; buffer_len(w, h).unwrap()]
}

fn full(w: u32, h: u32) -> Rect {
    Rect { x: 0, y: 0, w, h }
}

#[test]
fn test_lka11_palette_values_and_roles() {
    assert_eq!(BLUE, Rgb(0x00, 0x47, 0xBB));
    assert_eq!(PALE, Rgb(0xED, 0xF1, 0xFF));
    assert_eq!(PALE_4, Rgb(0xC9, 0xD6, 0xFA));
    assert_eq!(PINK, Rgb(0xE5, 0x9B, 0xDC));
    assert_eq!(WARN_BORDER, Rgb(0xF5, 0xB5, 0x47));
    assert_eq!(BACKGROUND, BLUE);
    assert_eq!(STAR, PINK);
    assert_eq!(LABEL, PALE);
}

#[test]
fn test_lka10_mix_endpoints_rounding_and_sanitizing() {
    let a = Rgb(0, 100, 255);
    let b = Rgb(255, 0, 0);
    assert_eq!(mix(a, b, 0.0), a);
    assert_eq!(mix(a, b, 1.0), b);
    assert_eq!(mix(a, b, 0.5), Rgb(128, 50, 128));
    assert_eq!(mix(a, b, -2.0), a);
    assert_eq!(mix(a, b, 9.0), b);
    assert_eq!(mix(a, b, f32::NAN), a);
    assert_eq!(mix(a, b, f32::INFINITY), a);
}

#[test]
fn test_lka9_memory_bounds() {
    assert_eq!(MAX_OUTPUTS, 8);
    assert_eq!(MAX_SURFACE_DIM, 8192);
    assert_eq!(MAX_SURFACE_PIXELS, 33_554_432);
    assert_eq!(MAX_TOTAL_PIXELS, 67_108_864);
    assert_eq!(BUFFERS_PER_SURFACE, 2);
    assert_eq!(buffer_len(1, 1), Some(4));
    assert_eq!(buffer_len(1920, 1080), Some(1920 * 1080 * 4));
    assert_eq!(buffer_len(7680, 4320), Some(7680 * 4320 * 4));
    assert_eq!(buffer_len(0, 10), None);
    assert_eq!(buffer_len(10, 0), None);
    assert_eq!(buffer_len(8193, 10), None);
    assert_eq!(buffer_len(10, 8193), None);
    assert_eq!(
        buffer_len(8192, 8192),
        None,
        "67 Mpx is over the per-surface budget"
    );
    assert_eq!(buffer_len(u32::MAX, u32::MAX), None);
}

#[test]
fn test_lka9_layout_geometry() {
    assert!((WIDGET_CENTER_Y_FRACTION - 0.78).abs() < f32::EPSILON);
    assert_eq!(Layout::for_surface(0, 100), None);
    assert_eq!(Layout::for_surface(100, 0), None);
    let l = Layout::for_surface(1920, 1080).unwrap();
    assert_eq!((l.width, l.height), (1920, 1080));
    assert_eq!(l.cx, 960.0);
    assert!((l.cy - 1080.0 * 0.78).abs() < 0.5);
    assert_eq!(l.radius, 49.0); // round(1080 * 0.045) = 48.6 -> 49
    assert_eq!(l.stroke, 4.0); // round(49 / 12) = 4
    assert!((l.star_half - 0.55 * 49.0).abs() < 0.01);
    assert_eq!(l.label_scale, 4); // clamp(49 / 12, 2, 6)
    assert!((l.label_top - (l.cy + l.radius + 0.5 * l.radius)).abs() < 0.5);
    // Bounds: radius clamped to [20, 72], scale to [2, 6], stroke at least 2.
    let small = Layout::for_surface(320, 200).unwrap();
    assert_eq!(small.radius, 20.0);
    assert_eq!(small.stroke, 2.0);
    assert_eq!(small.label_scale, 2);
    let huge = Layout::for_surface(7680, 4320).unwrap();
    assert_eq!(huge.radius, 72.0);
    assert_eq!(huge.stroke, 6.0);
    assert_eq!(huge.label_scale, 6);
}

#[test]
fn test_lka9_widget_box_encloses_everything_at_every_shake_offset() {
    for (w, h) in [(1920, 1080), (1366, 768), (3840, 2160), (800, 600)] {
        let l = Layout::for_surface(w, h).unwrap();
        let b = l.widget;
        assert!(b.x + b.w <= w && b.y + b.h <= h, "clipped to the surface");
        let reach = l.radius + l.stroke / 2.0 + SLIDE_PX;
        assert!(b.x as f32 <= l.cx - reach && (b.x + b.w) as f32 >= l.cx + reach);
        assert!(b.y as f32 <= l.cy - l.radius - l.stroke / 2.0);
        let label_w = text_width(NOT_RECOGNIZED_TEXT, l.label_scale) as f32;
        assert!(b.x as f32 <= l.cx - label_w / 2.0 - SLIDE_PX);
        assert!((b.x + b.w) as f32 >= l.cx + label_w / 2.0 + SLIDE_PX);
        assert!((b.y + b.h) as f32 >= l.label_top + (GLYPH_H * l.label_scale) as f32);
    }
}

#[test]
fn test_lka9_tiny_surfaces_are_safe() {
    for (w, h) in [(1, 1), (2, 3), (17, 9), (40, 1000), (1000, 40)] {
        let l = Layout::for_surface(w, h).unwrap();
        assert!(l.widget.x + l.widget.w <= w && l.widget.y + l.widget.h <= h);
        let mut data = buffer(w, h);
        let mut c = Canvas::new(&mut data, w, h).unwrap();
        let frame = FrameSpec {
            ring: RING_SCANNING,
            star_angle_deg: 33.0,
            offset_x: -SLIDE_PX,
            label: NOT_RECOGNIZED_TEXT,
            animating: true,
        };
        paint_full(&mut c, &l, &frame);
        paint_widget(&mut c, &l, &frame);
    }
}

#[test]
fn test_lka10_canvas_requires_a_large_enough_buffer() {
    let mut small = vec![0u8; 15];
    assert!(Canvas::new(&mut small, 2, 2).is_none());
    let mut zero: Vec<u8> = Vec::new();
    assert!(Canvas::new(&mut zero, 0, 0).is_none());
    let mut ok = vec![0u8; 16];
    let c = Canvas::new(&mut ok, 2, 2).unwrap();
    assert_eq!((c.width(), c.height()), (2, 2));
    assert_eq!(c.pixel(2, 0), None);
    assert_eq!(c.pixel(0, 2), None);
}

#[test]
fn test_lka10_pixel_format_is_argb8888_little_endian_opaque() {
    let mut data = buffer(3, 2);
    {
        let mut c = Canvas::new(&mut data, 3, 2).unwrap();
        c.fill_rect(full(3, 2), Rgb(0x11, 0x22, 0x33));
        c.fill_rect(
            Rect {
                x: 2,
                y: 1,
                w: 50,
                h: 50,
            },
            Rgb(0xAA, 0xBB, 0xCC),
        );
        assert_eq!(c.pixel(0, 0), Some(Rgb(0x11, 0x22, 0x33)));
        assert_eq!(c.pixel(2, 1), Some(Rgb(0xAA, 0xBB, 0xCC)));
    }
    assert_eq!(&data[0..4], &[0x33, 0x22, 0x11, 0xFF]);
    assert_eq!(&data[20..24], &[0xCC, 0xBB, 0xAA, 0xFF]);
}

#[test]
fn test_lka10_ring_geometry() {
    let (w, h) = (101, 101);
    let mut data = buffer(w, h);
    let mut c = Canvas::new(&mut data, w, h).unwrap();
    c.fill_rect(full(w, h), BLUE);
    c.ring(50.5, 50.5, 30.0, 4.0, PALE, full(w, h));
    // On the ring (pixel centres at distance 30): full colour.
    assert_eq!(c.pixel(80, 50), Some(PALE));
    assert_eq!(c.pixel(50, 20), Some(PALE));
    // Centre and far outside untouched.
    assert_eq!(c.pixel(50, 50), Some(BLUE));
    assert_eq!(c.pixel(95, 50), Some(BLUE));
    assert_eq!(c.pixel(50, 76), Some(BLUE));
    // Anti-aliased edge: a pixel half covered is a mix, neither colour.
    let edge = c.pixel(82, 50).unwrap();
    assert!(edge != PALE && edge != BLUE, "{edge:?}");
}

#[test]
fn test_lka10_ring_respects_clip_and_rejects_bad_geometry() {
    let (w, h) = (64, 64);
    let mut data = buffer(w, h);
    let mut c = Canvas::new(&mut data, w, h).unwrap();
    c.fill_rect(full(w, h), BLUE);
    let clip = Rect {
        x: 0,
        y: 0,
        w: 32,
        h: 64,
    };
    c.ring(32.0, 32.0, 20.0, 4.0, PALE, clip);
    assert_eq!(c.pixel(12, 32), Some(PALE));
    assert_eq!(c.pixel(52, 32), Some(BLUE), "outside the clip");
    for (r, s) in [
        (f32::NAN, 2.0),
        (10.0, f32::NAN),
        (f32::INFINITY, 2.0),
        (-5.0, 2.0),
        (10.0, 0.0),
        (10.0, -1.0),
    ] {
        c.ring(32.0, 32.0, r, s, MAGENTA, full(w, h));
    }
    c.ring(f32::NAN, 32.0, 10.0, 2.0, MAGENTA, full(w, h));
    c.ring(1e30, -1e30, 10.0, 2.0, MAGENTA, full(w, h));
    for y in 0..h {
        for x in 0..w {
            assert_ne!(c.pixel(x, y), Some(MAGENTA), "{x},{y}");
        }
    }
}

#[test]
fn test_lka10_star_shape_matches_the_brand_spikes() {
    // Inside: the top-right and bottom-left spikes near the centre.
    assert!(star_contains(0.52, 0.45));
    assert!(star_contains(0.48, 0.55));
    // Near the tips the spikes are thin: at v = 0.05 the top spike spans u in [0.5, 0.5025].
    assert!(star_contains(0.501, 0.05));
    assert!(!star_contains(0.51, 0.05));
    assert!(star_contains(0.95, 0.499));
    assert!(!star_contains(0.95, 0.49));
    // Outside: the other two quadrants and the carved quarter discs.
    assert!(!star_contains(0.25, 0.25));
    assert!(!star_contains(0.75, 0.75));
    assert!(!star_contains(0.75, 0.25));
    assert!(!star_contains(0.25, 0.75));
    assert!(!star_contains(1.5, 0.2));
    assert!(!star_contains(f32::NAN, 0.4));
}

fn star_pixels(angle: f32) -> Vec<Option<Rgb>> {
    let (w, h) = (80, 80);
    let mut data = buffer(w, h);
    let mut c = Canvas::new(&mut data, w, h).unwrap();
    c.fill_rect(full(w, h), BLUE);
    c.star(40.0, 40.0, 30.0, angle, PINK, full(w, h));
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| c.pixel(x, y))
        .collect()
}

#[test]
fn test_lka10_star_rotation_symmetry_is_half_turn() {
    let zero = star_pixels(0.0);
    let ninety = star_pixels(90.0);
    let half = star_pixels(180.0);
    assert!(zero.contains(&Some(PINK)));
    assert_ne!(zero, ninety, "a quarter turn mirrors the two spikes");
    let differing = zero.iter().zip(&half).filter(|(a, b)| a != b).count();
    assert!(
        differing <= 8,
        "a half turn reads the same ({differing} pixels differ)"
    );
}

#[test]
fn test_lka10_star_rotates_clockwise() {
    // Box [10, 70] (half 30 around 40). The unrotated point (0.6, 0.4) of the top-right spike,
    // turned 45 degrees clockwise on screen (y down), lands near (0.641, 0.5): pixel (48, 39).
    // Turned counter-clockwise (equivalently 135 degrees clockwise) it would land elsewhere.
    let draw = |angle: f32| {
        let (w, h) = (80, 80);
        let mut data = buffer(w, h);
        let mut c = Canvas::new(&mut data, w, h).unwrap();
        c.fill_rect(full(w, h), BLUE);
        c.star(40.0, 40.0, 30.0, angle, PINK, full(w, h));
        c.pixel(48, 39)
    };
    assert_eq!(draw(45.0), Some(PINK));
    assert_eq!(draw(135.0), Some(BLUE));
}

#[test]
fn test_lka10_font_covers_the_labels_and_text_draws() {
    assert_eq!((GLYPH_W, GLYPH_H, GLYPH_GAP), (5, 7, 1));
    for c in "ScanningNot recognized".chars().filter(|c| *c != ' ') {
        let g = glyph(c).unwrap_or_else(|| panic!("missing glyph {c}"));
        assert!(g.iter().any(|row| *row != 0), "{c} is blank");
        assert!(g.iter().all(|row| *row < 32), "{c} wider than 5 columns");
    }
    assert_eq!(glyph(' '), Some([0; 7]));
    assert_eq!(glyph('#'), None);
    assert_eq!(text_width("", 3), 0);
    assert_eq!(text_width("S", 3), 15);
    assert_eq!(text_width(SCANNING_TEXT, 2), (8 * 6 - 1) * 2);
    assert_eq!(text_width(NOT_RECOGNIZED_TEXT, 1), 14 * 6 - 1);
    assert_eq!(text_width(NOT_RECOGNIZED_TEXT, u32::MAX), u32::MAX);

    let (w, h) = (120, 30);
    let mut data = buffer(w, h);
    let mut c = Canvas::new(&mut data, w, h).unwrap();
    c.fill_rect(full(w, h), BLUE);
    c.text(60.0, 5.0, 2, SCANNING_TEXT, PALE, full(w, h));
    let lit = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|(x, y)| c.pixel(*x, *y) == Some(PALE))
        .count();
    assert!(lit > 50, "{lit}");
    // Centred: nothing left of cx - width/2 or right of cx + width/2.
    let half = (text_width(SCANNING_TEXT, 2) / 2) as u32;
    for y in 0..h {
        assert_ne!(c.pixel(60 - half - 2, y), Some(PALE));
        assert_ne!(c.pixel(60 + half + 2, y), Some(PALE));
    }
}

#[test]
fn test_lka10_paint_full_and_widget_only_writes_inside_the_box() {
    let (w, h) = (640, 480);
    let l = Layout::for_surface(w, h).unwrap();
    let frame = FrameSpec {
        ring: RING_IDLE,
        star_angle_deg: 0.0,
        offset_x: 0.0,
        label: "",
        animating: false,
    };
    let mut data = buffer(w, h);
    let mut c = Canvas::new(&mut data, w, h).unwrap();
    c.fill_rect(full(w, h), MAGENTA);
    paint_full(&mut c, &l, &frame);
    assert_eq!(c.pixel(0, 0), Some(BACKGROUND));
    assert_eq!(c.pixel(w - 1, h - 1), Some(BACKGROUND));
    assert_eq!(
        c.pixel(l.cx as u32 + l.radius as u32, l.cy as u32),
        Some(RING_IDLE)
    );
    // The star sits at the ring centre.
    let star_lit = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|(x, y)| c.pixel(*x, *y) == Some(STAR))
        .count();
    assert!(star_lit > 20, "{star_lit}");

    // paint_widget over a magenta canvas: every pixel outside the box stays magenta.
    c.fill_rect(full(w, h), MAGENTA);
    let shaken = FrameSpec {
        ring: RING_SCANNING,
        star_angle_deg: 120.0,
        offset_x: SLIDE_PX,
        label: NOT_RECOGNIZED_TEXT,
        animating: true,
    };
    paint_widget(&mut c, &l, &shaken);
    for y in 0..h {
        for x in 0..w {
            let inside = l.widget.contains(x, y);
            let p = c.pixel(x, y).unwrap();
            if inside {
                assert_ne!(p, MAGENTA, "{x},{y} inside the box must be repainted");
            } else {
                assert_eq!(p, MAGENTA, "{x},{y} outside the box was written");
            }
        }
    }
    let label_lit = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|(x, y)| c.pixel(*x, *y) == Some(LABEL))
        .count();
    assert!(label_lit > 50, "{label_lit}");
}

#[test]
fn test_lka10_rect_contains() {
    let r = Rect {
        x: 2,
        y: 3,
        w: 4,
        h: 5,
    };
    assert!(r.contains(2, 3));
    assert!(r.contains(5, 7));
    assert!(!r.contains(6, 7));
    assert!(!r.contains(5, 8));
    assert!(!r.contains(1, 3));
    let empty = Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };
    assert!(!empty.contains(0, 0));
    let edge = Rect {
        x: u32::MAX,
        y: u32::MAX,
        w: 5,
        h: 5,
    };
    assert!(edge.contains(u32::MAX, u32::MAX));
}

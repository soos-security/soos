//! Brand theme of `soos-gui`: palette tokens, layout metrics and the global egui style.
//!
//! The palette comes from the soos design direction: brand blue `#0047BB` (Pantone 2728C),
//! Brilliant White `#EDF1FF`, ink `#101820` (Black 6C) and pink `#E59BDC` (Pantone 244C).
//! Every other tone is a tint of these four or a semantic state color. Only the egui
//! default fonts are used; headings get their weight from [`paint_faux_bold`].

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    reason = "GUI coordinate and metric calculations on bounded f32 values"
)]

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, Shadow, Stroke, Vec2, Visuals,
};

// ---------------------------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------------------------

/// Brand blue `#0047BB`: header band, borders, primary buttons, stat tiles.
pub const BLUE: Color32 = Color32::from_rgb(0x00, 0x47, 0xBB);
/// Hover tone of blue surfaces.
pub const BLUE_HOVER: Color32 = Color32::from_rgb(0x1D, 0x5C, 0xC3);
/// Pressed tone of blue surfaces.
pub const BLUE_PRESSED: Color32 = Color32::from_rgb(0x00, 0x3A, 0x99);
/// Brilliant White `#EDF1FF`: body, cards, active tab, text on blue.
pub const PALE: Color32 = Color32::from_rgb(0xED, 0xF1, 0xFF);
/// Inner tables and inactive widget fill.
pub const PALE_2: Color32 = Color32::from_rgb(0xE4, 0xEA, 0xFD);
/// Hovered widget fill.
pub const PALE_3: Color32 = Color32::from_rgb(0xDC, 0xE4, 0xFC);
/// Active (pressed) widget fill.
pub const PALE_4: Color32 = Color32::from_rgb(0xC9, 0xD6, 0xFA);
/// Subtle borders, separators, disabled fill and the switch-off track.
pub const LINE: Color32 = Color32::from_rgb(0xC9, 0xD0, 0xE1);
/// Ink `#101820`: primary text.
pub const INK: Color32 = Color32::from_rgb(0x10, 0x18, 0x20);
/// Secondary text and descriptions.
pub const INK_MUTED: Color32 = Color32::from_rgb(0x5A, 0x62, 0x70);
/// Disabled text.
pub const INK_WEAK: Color32 = Color32::from_rgb(0x7A, 0x81, 0x8E);
/// Pink `#E59BDC`: brand accent (star, active reticle, guidance arrows).
pub const PINK: Color32 = Color32::from_rgb(0xE5, 0x9B, 0xDC);
/// Pure white, reserved for text-edit backgrounds and text on danger buttons.
pub const WHITE: Color32 = Color32::WHITE;

/// Success (live PAD, match accepted, completed steps).
pub const SUCCESS: Color32 = Color32::from_rgb(0x29, 0xB5, 0x3B);
/// Daemon switch track when the service runs.
pub const SUCCESS_TOGGLE: Color32 = Color32::from_rgb(0x46, 0xBE, 0x82);
/// Success banner background.
pub const SUCCESS_BG: Color32 = Color32::from_rgb(0xE6, 0xF6, 0xEA);
/// Success banner text.
pub const SUCCESS_TEXT: Color32 = Color32::from_rgb(0x14, 0x6C, 0x2A);
/// Danger (errors, spoof, delete).
pub const DANGER: Color32 = Color32::from_rgb(0xD9, 0x2D, 0x45);
/// Hover tone of danger buttons.
pub const DANGER_HOVER: Color32 = Color32::from_rgb(0xB8, 0x1F, 0x35);
/// Danger banner background.
pub const DANGER_BG: Color32 = Color32::from_rgb(0xFD, 0xE8, 0xEB);
/// Danger banner text.
pub const DANGER_TEXT: Color32 = Color32::from_rgb(0x8A, 0x14, 0x26);
/// Warning accent (developer store, caution states).
pub const WARN: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
/// Warning banner border.
pub const WARN_BORDER: Color32 = Color32::from_rgb(0xF5, 0xB5, 0x47);
/// Warning banner background.
pub const WARN_BG: Color32 = Color32::from_rgb(0xFF, 0xF4, 0xE0);
/// Warning banner text.
pub const WARN_TEXT: Color32 = Color32::from_rgb(0x8A, 0x4B, 0x00);

/// Face box of a live face on the video.
pub const FACE_LIVE: Color32 = Color32::from_rgb(0x11, 0xDC, 0x79);
/// Face box of a spoofed face on the video.
pub const FACE_SPOOF: Color32 = Color32::from_rgb(0xFF, 0x17, 0x44);
/// Eye landmarks.
pub const EYE: Color32 = Color32::from_rgb(0x2E, 0xE2, 0xF8);
/// Nose landmark.
pub const NOSE: Color32 = Color32::from_rgb(0xFF, 0xEA, 0x00);
/// Mouth landmarks.
pub const MOUTH: Color32 = Color32::from_rgb(0xFF, 0x91, 0x00);
/// Eye axis line (semi-transparent light cyan).
pub const EYE_AXIS: Color32 = Color32::from_rgba_premultiplied(132, 180, 188, 200);
/// Composition guide lines over the video (white, about 35 % alpha).
pub const GUIDE: Color32 = Color32::from_rgba_premultiplied(90, 90, 90, 90);
/// PAD crop area outline.
pub const PAD_BOX: Color32 = Color32::from_rgba_premultiplied(130, 132, 140, 140);
/// Caption above the aligned-crop inset.
pub const INSET_LABEL: Color32 = Color32::from_rgb(0xD0, 0xD3, 0xD8);
/// Drop shadow of windows and popups.
pub const SHADOW: Color32 = Color32::from_rgba_premultiplied(3, 4, 5, 40);

// ---------------------------------------------------------------------------------------------
// Sizes (reference values at a 1512-point-wide window)
// ---------------------------------------------------------------------------------------------

/// Height of the blue header band.
pub const HEADER_H: f32 = 44.0;
/// Corner radius of the pale body under the header.
pub const BODY_TOP_RADIUS: u8 = 20;
/// Gap between stacked cards and tiles.
pub const GAP_CARD: f32 = 14.0;
/// Card corner radius.
pub const R_CARD: u8 = 20;
/// Video corner radius.
pub const R_VIDEO: u8 = 16;
/// Inner table corner radius.
pub const R_TABLE: u8 = 14;
/// Pill button corner radius.
pub const R_BUTTON: u8 = 18;
/// Text input corner radius.
pub const R_INPUT: u8 = 10;
/// Card border width (drawn inside the card).
pub const STROKE_CARD: f32 = 2.0;
/// Height of the caption strip of a stat tile (border included).
pub const STRIP_H: f32 = 50.0;
/// Height of one inner-table row.
pub const TABLE_ROW_H: f32 = 26.0;

/// Tab label size.
pub const F_TAB: f32 = 14.0;
/// Card title size.
pub const F_CARD_TITLE: f32 = 19.0;
/// Section heading size.
pub const F_SECTION: f32 = 15.0;
/// Body text size.
pub const F_BODY: f32 = 13.0;
/// Small text size.
pub const F_SMALL: f32 = 12.0;
/// Placeholder text size on the video.
pub const F_PLACEHOLDER: f32 = 18.0;

/// Responsive layout metrics derived from the window width.
///
/// The reference mockup is 1512 points wide; narrower windows scale the paddings down to the
/// 900-point minimum window without ever shrinking the right column below 250 points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Scale factor relative to the 1512-point reference, in `[0.6, 1.0]`.
    pub k: f32,
    /// Body padding on the left, right, top and bottom.
    pub pad: f32,
    /// Gap between the main area (video) and the right column.
    pub gap_main: f32,
    /// Width of the right column (cards and tiles).
    pub col_w: f32,
    /// Gap between stacked cards.
    pub gap_card: f32,
    /// Height of the header band.
    pub header_h: f32,
}

impl Metrics {
    /// Computes the metrics for a window `width` points wide.
    pub fn for_width(width: f32) -> Self {
        let k = (width / 1512.0).clamp(0.6, 1.0);
        Self {
            k,
            pad: (40.0 * k).max(20.0),
            gap_main: (40.0 * k).max(20.0),
            col_w: (259.0 * k).clamp(250.0, 300.0),
            gap_card: GAP_CARD,
            header_h: HEADER_H,
        }
    }

    /// Splits `content` into the main area (left) and the right column.
    pub fn split_columns(&self, content: Rect) -> (Rect, Rect) {
        let col_left = (content.max.x - self.col_w).max(content.min.x);
        let main_right = (col_left - self.gap_main).max(content.min.x);
        let main = Rect::from_min_max(content.min, Pos2::new(main_right, content.max.y));
        let column = Rect::from_min_max(Pos2::new(col_left, content.min.y), content.max);
        (main, column)
    }
}

/// Builds the brand egui [`Visuals`] (light theme, pale body, ink text, blue selection).
pub fn visuals() -> Visuals {
    let mut v = Visuals::light();
    v.override_text_color = None;
    v.panel_fill = PALE;
    v.window_fill = PALE;
    v.extreme_bg_color = WHITE;
    v.text_edit_bg_color = Some(WHITE);
    v.faint_bg_color = PALE_2;
    v.code_bg_color = PALE_2;
    v.hyperlink_color = BLUE;
    v.warn_fg_color = WARN_TEXT;
    v.error_fg_color = DANGER;
    v.window_corner_radius = CornerRadius::same(20);
    v.window_stroke = Stroke::new(2.0, BLUE);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: SHADOW,
    };
    v.menu_corner_radius = CornerRadius::same(12);
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: SHADOW,
    };
    v.selection.bg_fill = BLUE;
    v.selection.stroke = Stroke::new(1.0, PALE);
    v.striped = false;

    let radius = CornerRadius::same(R_INPUT);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PALE;
    w.noninteractive.weak_bg_fill = PALE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = PALE_2;
    w.inactive.weak_bg_fill = PALE_2;
    w.inactive.bg_stroke = Stroke::new(1.0, LINE);
    w.inactive.fg_stroke = Stroke::new(1.0, INK);
    w.inactive.corner_radius = radius;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = PALE_3;
    w.hovered.weak_bg_fill = PALE_3;
    w.hovered.bg_stroke = Stroke::new(1.5, BLUE);
    w.hovered.fg_stroke = Stroke::new(1.5, BLUE);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = PALE_4;
    w.active.weak_bg_fill = PALE_4;
    w.active.bg_stroke = Stroke::new(2.0, BLUE);
    w.active.fg_stroke = Stroke::new(2.0, BLUE_PRESSED);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = PALE_2;
    w.open.weak_bg_fill = PALE_2;
    w.open.bg_stroke = Stroke::new(1.5, BLUE);
    w.open.fg_stroke = Stroke::new(1.0, INK);
    w.open.corner_radius = radius;
    v
}

/// Installs the brand theme on `ctx` for both the light and the dark system preference.
///
/// Called once at start-up; it only mutates egui style state and never blocks.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Light);
    let brand = visuals();
    ctx.all_styles_mut(|style| {
        style.visuals = brand.clone();
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 7.0);
        style.spacing.interact_size = Vec2::new(40.0, 28.0);
        style.spacing.indent = 18.0;
        style.spacing.window_margin = Margin::same(24);
        let mut scroll = egui::style::ScrollStyle::thin();
        scroll.bar_width = 6.0;
        scroll.floating = true;
        scroll.foreground_color = true;
        scroll.dormant_handle_opacity = 0.25;
        scroll.active_handle_opacity = 0.6;
        scroll.interact_handle_opacity = 0.8;
        style.spacing.scroll = scroll;
    });
}

/// Horizontal offsets (in points) of the faux-bold passes, snapped to the physical pixel grid.
///
/// egui rounds every galley position to whole pixels, so sub-pixel steps would collapse onto
/// the first pass and the text would render thin depending on where it lands. Each step is
/// therefore at least one physical pixel, every offset is rounded to the pixel grid and
/// duplicates are dropped. Always returns at least the zero offset.
pub fn faux_bold_offsets(passes: u8, step: f32, pixels_per_point: f32) -> Vec<f32> {
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
        pixels_per_point
    } else {
        1.0
    };
    let pixel = 1.0 / ppp;
    let step = if step.is_finite() {
        step.max(pixel)
    } else {
        pixel
    };
    let mut offsets: Vec<f32> = Vec::with_capacity(usize::from(passes.max(1)));
    for i in 0..passes.max(1) {
        let snapped = ((step * f32::from(i)) * ppp).round() / ppp;
        if offsets.last().is_none_or(|last| snapped > *last) {
            offsets.push(snapped);
        }
    }
    offsets
}

/// Paints `text` several times with small horizontal offsets to simulate a bold weight
/// (the egui default fonts ship a single weight). Returns the rectangle of the text.
///
/// The offsets come from [`faux_bold_offsets`], so the weight is the same wherever the text
/// lands on screen.
#[allow(
    clippy::too_many_arguments,
    reason = "A painter helper mirroring egui::Painter::text plus the two faux-bold knobs"
)]
pub fn paint_faux_bold(
    painter: &egui::Painter,
    pos: Pos2,
    align: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    passes: u8,
    step: f32,
) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), font, color);
    paint_galley_faux_bold(painter, pos, align, &galley, color, passes, step)
}

/// Faux-bold painting of an already laid-out `galley` (see [`paint_faux_bold`]).
#[allow(
    clippy::too_many_arguments,
    reason = "A painter helper mirroring egui::Painter::galley plus the two faux-bold knobs"
)]
pub fn paint_galley_faux_bold(
    painter: &egui::Painter,
    pos: Pos2,
    align: Align2,
    galley: &std::sync::Arc<egui::Galley>,
    color: Color32,
    passes: u8,
    step: f32,
) -> Rect {
    let ppp = painter.pixels_per_point();
    let offsets = faux_bold_offsets(passes, step, ppp);
    let extra = offsets.last().copied().unwrap_or(0.0);
    let size = galley.size() + Vec2::new(extra, 0.0);
    let rect = align.anchor_size(pos, size);
    // Snap the origin too, so every pass lands on its own pixel column.
    let snap = |v: f32| (v * ppp).round() / ppp;
    let origin = Pos2::new(snap(rect.min.x), snap(rect.min.y));
    for dx in offsets {
        painter.galley(origin + Vec2::new(dx, 0.0), galley.clone(), color);
    }
    rect
}

/// Splits `text` into lines no wider than `max_width` when laid out in `font` (greedy word
/// wrap; a single word wider than the limit stays on its own line).
pub fn wrap_words(
    painter: &egui::Painter,
    text: &str,
    font: &FontId,
    max_width: f32,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        let width = painter
            .layout_no_wrap(candidate.clone(), font.clone(), INK)
            .size()
            .x;
        if width > max_width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current = word.to_owned();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Body content rectangle under the header band for a window covering `full`.
///
/// Shared by the header renderer and the layout contract tests: left, right and bottom
/// paddings of `metrics.pad`, and `0.75 * pad` below the band.
pub fn content_rect(full: Rect, metrics: &Metrics) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            full.min.x + metrics.pad,
            full.min.y + metrics.header_h + metrics.pad * 0.75,
        ),
        Pos2::new(
            (full.max.x - metrics.pad).max(full.min.x + metrics.pad),
            (full.max.y - metrics.pad).max(full.min.y + metrics.header_h),
        ),
    )
}

/// Largest rectangle of `aspect` (width / height) inside `main`, centered horizontally and
/// anchored at the top (the video panel of the Live and Enrollment pages).
///
/// A non-finite or non-positive aspect falls back to 4:3.
pub fn fit_video(main: Rect, aspect: f32) -> Rect {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        4.0 / 3.0
    };
    let w = main.width().min(main.height() * aspect).max(0.0);
    let h = w / aspect;
    Rect::from_min_size(
        Pos2::new(main.center().x - w / 2.0, main.min.y),
        Vec2::new(w, h),
    )
}

/// Mixes `color` toward white by `amount` in `[0, 1]` (hover lightening).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Channel values are clamped to [0, 255] before the cast"
)]
pub fn lighten(color: Color32, amount: f32) -> Color32 {
    let t = amount.clamp(0.0, 1.0);
    let mix = |c: u8| -> u8 {
        let c = f32::from(c);
        (c + (255.0 - c) * t).round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgba_unmultiplied(mix(color.r()), mix(color.g()), mix(color.b()), color.a())
}

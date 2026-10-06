//! Reusable brand widgets: cards, inner tables, stat and star tiles, banners, pill buttons and
//! the toggle switch. Pure presentation: no widget here blocks, spawns or touches the store.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "GUI coordinate calculations on bounded f32 values; radii are clamped before casts"
)]

use eframe::egui::{
    self, vec2, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Layout, Margin,
    Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextStyle, TextWrapMode, Ui, Vec2,
    Widget, WidgetInfo, WidgetText, WidgetType,
};

use crate::brand;
use crate::theme::{self, paint_faux_bold};

// ---------------------------------------------------------------------------------------------
// Cards and tables
// ---------------------------------------------------------------------------------------------

/// Card frame: pale fill, 2-point blue border drawn inside, radius 20.
pub fn card_frame() -> Frame {
    Frame::new()
        .fill(theme::PALE)
        .stroke(Stroke::new(theme::STROKE_CARD, theme::BLUE))
        .corner_radius(CornerRadius::same(theme::R_CARD))
        .inner_margin(Margin {
            left: 22,
            right: 22,
            top: 26,
            bottom: 22,
        })
}

/// Inner table frame inside a card: slightly darker pale fill, radius 14.
pub fn inner_table_frame() -> Frame {
    Frame::new()
        .fill(theme::PALE_2)
        .corner_radius(CornerRadius::same(theme::R_TABLE))
        .inner_margin(Margin::symmetric(18, 10))
}

/// Paints a centered card title (faux bold) and reserves its height plus a gap below it.
///
/// A title too long for one line is word-wrapped and every line keeps the faux-bold weight,
/// so card titles look the same on every page.
pub fn card_title(ui: &mut Ui, text: &str) {
    card_title_colored(ui, text, theme::INK);
}

/// [`card_title`] in an explicit `color` (status cards tint their title by severity).
pub fn card_title_colored(ui: &mut Ui, text: &str, color: Color32) {
    let font = FontId::proportional(theme::F_CARD_TITLE);
    let width = ui.available_width();
    let lines = theme::wrap_words(ui.painter(), text, &font, (width - 2.0).max(1.0));
    let row_h = ui.fonts_mut(|f| f.row_height(&font));
    let rows = lines.len().max(1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, row_h * rows), Sense::hover());
    let painter = ui
        .painter()
        .with_clip_rect(rect.expand(2.0).intersect(ui.clip_rect()));
    for (index, line) in lines.iter().enumerate() {
        let y = rect.min.y + row_h * (index as f32 + 0.5);
        paint_faux_bold(
            &painter,
            Pos2::new(rect.center().x, y),
            Align2::CENTER_CENTER,
            line,
            font.clone(),
            color,
            2,
            0.6,
        );
    }
    ui.add_space(14.0);
}

/// Paints a left-aligned section heading (faux bold).
pub fn section_title(ui: &mut Ui, text: &str) {
    let font = FontId::proportional(theme::F_SECTION);
    let height = ui.fonts_mut(|f| f.row_height(&font));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    paint_faux_bold(
        ui.painter(),
        rect.left_center(),
        Align2::LEFT_CENTER,
        text,
        font,
        theme::INK,
        2,
        0.5,
    );
}

/// One inner-table row: `label` on the left, `value` right-aligned, 26 points high.
pub fn table_row(ui: &mut Ui, label: &str, value: impl Into<WidgetText>) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, theme::TABLE_ROW_H),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_size(vec2(width, theme::TABLE_ROW_H));
            ui.label(RichText::new(label).size(theme::F_BODY).color(theme::INK));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(value);
            });
        },
    );
}

/// Value text for [`table_row`] in body size and `color`.
pub fn value_text(text: impl Into<String>, color: Color32) -> RichText {
    RichText::new(text).size(theme::F_BODY).color(color)
}

/// Paints `text` on the video with a small translucent ink backing pill so it stays readable
/// on bright or busy footage. Returns the rectangle of the pill.
pub fn backed_text(
    painter: &egui::Painter,
    pos: Pos2,
    align: Align2,
    text: &str,
    font: FontId,
    color: Color32,
) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), font, color);
    let text_rect = align.anchor_size(pos, galley.size());
    let pill = text_rect.expand2(vec2(4.0, 2.0));
    painter.rect_filled(pill, 4.0, Color32::from_rgba_unmultiplied(16, 24, 32, 160));
    painter.galley(text_rect.min, galley, color);
    pill
}

/// Where the label of a [`progress_bar`] goes for a given fill width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressLabel {
    /// Inside the fill, right-aligned, in the pale on-fill color.
    InsideFill,
    /// At the right end of the track, in ink (the fill is too short to carry it).
    TrackEnd,
}

/// Fill width of a [`progress_bar`] `track_w` wide and `track_h` high at `fraction`.
///
/// Zero progress paints no fill at all; any positive progress is at least as wide as the
/// track is high (a full rounded cap) and never wider than the track.
pub fn progress_fill_width(fraction: f32, track_w: f32, track_h: f32) -> f32 {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if fraction <= 0.0 || track_w <= 0.0 {
        0.0
    } else {
        (track_w * fraction).max(track_h).min(track_w)
    }
}

/// Label placement of a [`progress_bar`]: inside the fill only when the fill is wider than the
/// label plus 12 points.
pub fn progress_label_placement(fill_w: f32, label_w: f32) -> ProgressLabel {
    if fill_w > label_w + 12.0 {
        ProgressLabel::InsideFill
    } else {
        ProgressLabel::TrackEnd
    }
}

/// Brand progress bar: pale track, `fill` colored bar and a readable percentage label.
///
/// `animate` smooths the fill toward its target and keeps repainting while it moves, like
/// `egui::ProgressBar::animate`.
pub fn progress_bar(
    ui: &mut Ui,
    fraction: f32,
    fill: Color32,
    label: &str,
    animate: bool,
) -> Response {
    let height = 18.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let target = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let shown = if animate {
        ui.ctx()
            .animate_value_with_time(response.id.with("progress"), target, 0.25)
    } else {
        target
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = CornerRadius::same(9);
        painter.rect_filled(rect, radius, theme::PALE_3);
        let fill_w = progress_fill_width(shown, rect.width(), rect.height());
        if fill_w > 0.0 {
            painter.rect_filled(
                Rect::from_min_size(rect.min, vec2(fill_w, rect.height())),
                radius,
                fill,
            );
        }
        let font = FontId::proportional(theme::F_SMALL);
        let galley = painter.layout_no_wrap(label.to_owned(), font, theme::INK);
        let y = rect.center().y - galley.size().y / 2.0;
        match progress_label_placement(fill_w, galley.size().x) {
            ProgressLabel::InsideFill => painter.galley(
                Pos2::new(rect.min.x + fill_w - 8.0 - galley.size().x, y),
                galley,
                theme::PALE,
            ),
            ProgressLabel::TrackEnd => painter.galley(
                Pos2::new(rect.max.x - 8.0 - galley.size().x, y),
                galley,
                theme::INK,
            ),
        }
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ProgressIndicator, true, label));
    response
}

// ---------------------------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------------------------

/// Big stat tile: pale caption strip on a blue body with a large pale value.
///
/// `value` of `None` shows an em dash. Not interactive.
pub fn stat_tile(
    ui: &Ui,
    rect: Rect,
    caption: &str,
    value: Option<&str>,
    sub: Option<&str>,
    status_dot: Option<Color32>,
) {
    let painter = ui.painter_at(rect.expand(1.0));
    let radius = theme::R_CARD;
    painter.rect_filled(rect, CornerRadius::same(radius), theme::BLUE);

    let strip_h = theme::STRIP_H.min(rect.height() * 0.3);
    let strip = Rect::from_min_max(
        rect.min + vec2(theme::STROKE_CARD, theme::STROKE_CARD),
        Pos2::new(rect.max.x - theme::STROKE_CARD, rect.min.y + strip_h),
    );
    painter.rect_filled(
        strip,
        CornerRadius {
            nw: radius.saturating_sub(2),
            ne: radius.saturating_sub(2),
            sw: 0,
            se: 0,
        },
        theme::PALE,
    );

    let caption_job = {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &caption.to_uppercase(),
            0.0,
            egui::TextFormat {
                font_id: FontId::proportional(theme::F_SMALL),
                color: theme::INK,
                extra_letter_spacing: 1.0,
                ..Default::default()
            },
        );
        job
    };
    let caption_galley = painter.layout_job(caption_job);
    let caption_pos = Pos2::new(
        strip.min.x + 16.0,
        strip.center().y - caption_galley.size().y / 2.0,
    );
    theme::paint_galley_faux_bold(
        &painter,
        caption_pos,
        Align2::LEFT_TOP,
        &caption_galley,
        theme::INK,
        2,
        0.5,
    );
    if let Some(dot) = status_dot {
        painter.circle_filled(Pos2::new(strip.max.x - 20.0, strip.center().y), 4.5, dot);
    }

    let body = Rect::from_min_max(Pos2::new(rect.min.x, strip.max.y), rect.max);
    let text = value.unwrap_or("\u{2014}");
    let max_w = rect.width() - 36.0;
    let mut size = (body.height() * 0.5).min(rect.height() * 0.40).max(40.0);
    let mut galley =
        painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), theme::PALE);
    while galley.size().x + 10.0 > max_w && size > 40.0 {
        size = (size - 4.0).max(40.0);
        galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), theme::PALE);
    }
    let sub_h = if sub.is_some() { 22.0 } else { 0.0 };
    let center = Pos2::new(body.center().x, body.center().y - sub_h / 2.0 - 4.0);
    // More, wider passes on large sizes: the default font has a single light weight.
    // Passes are snapped to whole pixels, so smaller numbers get fewer of them.
    let step = (size / 60.0).clamp(0.6, 1.8);
    let passes = (size / 14.0).round().clamp(2.0, 6.0) as u8;
    let number = paint_faux_bold(
        &painter,
        center,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(size),
        theme::PALE,
        passes,
        step,
    );
    if let Some(sub) = sub {
        painter.text(
            Pos2::new(body.center().x, number.max.y + 4.0),
            Align2::CENTER_TOP,
            sub,
            FontId::proportional(theme::F_SMALL),
            theme::PALE.gamma_multiply(0.8),
        );
    }
}

/// Decorative star tile: blue rounded square with the pink star mark.
pub fn star_tile(ui: &Ui, rect: Rect) {
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::R_CARD), theme::BLUE);
    brand::paint_star(ui, rect, theme::PINK);
}

/// Allocates a stack of right-column rectangles: the card gets what the tiles leave over.
///
/// Returns `(card, stat_tile, star_tile)`. `card_needed` is the height the card content asks
/// for (see [`card_needed_height`]). The star tile is only shown when the card, a square stat
/// tile and a square star tile all fit; otherwise the stat tile shrinks toward its 140-point
/// floor before the card has to scroll.
pub fn column_stack(
    column: Rect,
    gap: f32,
    card_needed: f32,
) -> (Rect, Option<Rect>, Option<Rect>) {
    let tile = column.width();
    let h = column.height();
    let star = h >= card_needed + 2.0 * tile + 2.0 * gap;
    let stat_h = if star {
        tile
    } else {
        (h - card_needed - gap).clamp(STAT_TILE_MIN.min(tile), tile.max(STAT_TILE_MIN))
    };
    let tiles_h = stat_h + gap + if star { tile + gap } else { 0.0 };
    let card = Rect::from_min_max(
        column.min,
        Pos2::new(column.max.x, (column.max.y - tiles_h).max(column.min.y)),
    );
    let stat = Rect::from_min_size(
        Pos2::new(column.min.x, card.max.y + gap),
        vec2(tile, stat_h),
    );
    let star_rect = star
        .then(|| Rect::from_min_size(Pos2::new(column.min.x, stat.max.y + gap), vec2(tile, tile)));
    (card, Some(stat), star_rect)
}

/// Right-column split of the Profiles page for a column `column_h` high and `tile` wide.
///
/// Returns the stat tile height and, when there is room for it (at least 120 points), the
/// height of the star tile that fills the rest of the column. Without a star tile the stat
/// tile stretches over the whole column, so the column always ends level with the main card.
pub fn profiles_column(column_h: f32, tile: f32, gap: f32) -> (f32, Option<f32>) {
    let column_h = column_h.max(0.0);
    let stat_h = tile.min(column_h);
    let rest = column_h - stat_h - gap;
    if rest >= 120.0 {
        (stat_h, Some(rest))
    } else {
        (column_h, None)
    }
}

/// Like [`column_stack`], but gives the whole column to the card (no tiles) when even a stat
/// tile at its [`STAT_TILE_MIN`] floor would make the card scroll. Used by the pages whose
/// stat tile only repeats a value the card already shows.
pub fn column_stack_card_first(
    column: Rect,
    gap: f32,
    card_needed: f32,
) -> (Rect, Option<Rect>, Option<Rect>) {
    if column.height() < card_needed + STAT_TILE_MIN + gap {
        (column, None, None)
    } else {
        column_stack(column, gap, card_needed)
    }
}

/// Smallest height of a right-column stat tile.
pub const STAT_TILE_MIN: f32 = 140.0;

/// Vertical chrome of [`card_in_rect`] around its scroll content (top and bottom margins).
pub const CARD_CHROME_H: f32 = 38.0;

/// Explicit id of a card's content scope: the parent's stable id and the card's `id_salt`
/// only, never the parent's auto-id counter, so the content ids do not shift with what was laid
/// out before the card (a stat tile shown on one egui pass and hidden on the next).
/// `id_salt` must therefore be unique among the cards drawn in the same parent `Ui`.
fn card_scope_id(ui: &Ui, id_salt: &str) -> egui::Id {
    ui.id().with(("soos_card_scope", id_salt))
}

fn content_height_id(id_salt: &str) -> egui::Id {
    egui::Id::new(("soos_card_content_height", id_salt))
}

/// Height the card `id_salt` needs to show its content without scrolling, measured on the
/// previous frame (`fallback` before the first frame), chrome and `extra` included.
pub fn card_needed_height(ui: &Ui, id_salt: &str, fallback: f32, extra: f32) -> f32 {
    ui.data(|d| d.get_temp::<f32>(content_height_id(id_salt)))
        .map_or(fallback, |content| content + CARD_CHROME_H + extra)
}

fn store_content_height(ui: &Ui, id_salt: &str, height: f32) {
    let id = content_height_id(id_salt);
    let previous = ui.data(|d| d.get_temp::<f32>(id));
    if previous.is_none_or(|p| (p - height).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(id, height));
        // The column layout depends on this height: settle it on the next frame.
        ui.ctx().request_repaint();
    }
}

// ---------------------------------------------------------------------------------------------
// Banners
// ---------------------------------------------------------------------------------------------

/// Semantic kind of a [`banner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    /// Neutral information.
    Info,
    /// Success confirmation.
    Success,
    /// Caution (developer store, re-enrollment required, ...).
    Warning,
    /// Failure.
    Error,
}

impl BannerKind {
    /// `(background, border, accent, text)` colors of the kind.
    pub fn colors(self) -> (Color32, Color32, Color32, Color32) {
        match self {
            Self::Info => (theme::PALE_2, theme::LINE, theme::BLUE, theme::INK),
            Self::Success => (
                theme::SUCCESS_BG,
                theme::SUCCESS,
                theme::SUCCESS,
                theme::SUCCESS_TEXT,
            ),
            Self::Warning => (
                theme::WARN_BG,
                theme::WARN_BORDER,
                theme::WARN,
                theme::WARN_TEXT,
            ),
            Self::Error => (
                theme::DANGER_BG,
                theme::DANGER,
                theme::DANGER,
                theme::DANGER_TEXT,
            ),
        }
    }

    /// Paints the kind's vector icon (default fonts lack most symbol glyphs) in `rect`.
    pub fn paint_icon(self, painter: &egui::Painter, rect: Rect, color: Color32) {
        let c = rect.center();
        let r = rect.width().min(rect.height()) / 2.0;
        let stroke = Stroke::new(1.8, color);
        match self {
            Self::Warning => {
                let top = Pos2::new(c.x, c.y - r);
                let left = Pos2::new(c.x - r, c.y + r * 0.85);
                let right = Pos2::new(c.x + r, c.y + r * 0.85);
                painter.add(egui::Shape::closed_line(vec![top, right, left], stroke));
                painter.line_segment(
                    [
                        Pos2::new(c.x, c.y - r * 0.3),
                        Pos2::new(c.x, c.y + r * 0.25),
                    ],
                    stroke,
                );
                painter.circle_filled(Pos2::new(c.x, c.y + r * 0.55), 1.1, color);
            }
            Self::Error => {
                painter.circle_stroke(c, r, stroke);
                let d = r * 0.42;
                painter.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
                painter.line_segment([c + vec2(d, -d), c + vec2(-d, d)], stroke);
            }
            Self::Success => {
                painter.circle_stroke(c, r, stroke);
                painter.add(egui::Shape::line(
                    vec![
                        c + vec2(-r * 0.45, 0.0),
                        c + vec2(-r * 0.1, r * 0.35),
                        c + vec2(r * 0.5, -r * 0.35),
                    ],
                    stroke,
                ));
            }
            Self::Info => {
                painter.circle_stroke(c, r, stroke);
                painter.circle_filled(Pos2::new(c.x, c.y - r * 0.45), 1.1, color);
                painter.line_segment(
                    [Pos2::new(c.x, c.y - r * 0.1), Pos2::new(c.x, c.y + r * 0.5)],
                    stroke,
                );
            }
        }
    }
}

/// Full-width banner with a colored accent bar, a glyph and wrapped text.
pub fn banner(ui: &mut Ui, kind: BannerKind, text: &str) -> Response {
    let (bg, border, accent, fg) = kind.colors();
    let inner = Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin {
            left: 18,
            right: 14,
            top: 9,
            bottom: 9,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (icon, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                kind.paint_icon(ui.painter(), icon.shrink(1.0), accent);
                ui.add(egui::Label::new(RichText::new(text).size(theme::F_BODY).color(fg)).wrap());
            });
        });
    let rect = inner.response.rect;
    // The accent bar follows the rounded left edge: paint the rounded frame clipped to 4 points.
    let bar = Rect::from_min_max(rect.min, Pos2::new(rect.min.x + 4.0, rect.max.y));
    ui.painter()
        .with_clip_rect(bar.intersect(ui.clip_rect()))
        .rect_filled(rect, CornerRadius::same(12), accent);
    inner.response
}

/// Centered status card (420 points wide at most) used when no frame can be shown.
pub fn status_card<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let avail = ui.available_width();
    let width = 420.0_f32.min(avail);
    ui.horizontal(|ui| {
        ui.add_space(((avail - width) / 2.0).max(0.0));
        Frame::new()
            .fill(theme::PALE)
            .stroke(Stroke::new(theme::STROKE_CARD, theme::BLUE))
            .corner_radius(CornerRadius::same(theme::R_CARD))
            .inner_margin(Margin::same(26))
            .show(ui, |ui| {
                ui.set_width((width - 52.0).max(0.0));
                ui.vertical_centered(add_contents).inner
            })
            .inner
    })
    .inner
}

// ---------------------------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------------------------

/// Visual role of a [`BrandButton`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// Blue pill, pale text (main action).
    Primary,
    /// Blue outline pill, blue text.
    Secondary,
    /// Red pill, white text (destructive confirmation).
    Danger,
    /// Small red outline pill (table-row destructive action).
    DangerOutline,
}

/// Pill-shaped brand button with hover, pressed and disabled states.
///
/// Use with `ui.add(...)` or `ui.add_enabled(enabled, ...)`; it reports clicks like
/// [`egui::Button`].
#[derive(Debug, Clone)]
pub struct BrandButton {
    text: String,
    kind: ButtonKind,
    full_width: bool,
}

impl BrandButton {
    /// Creates a button of `kind` labeled `text`.
    pub fn new(kind: ButtonKind, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind,
            full_width: false,
        }
    }

    /// Primary (blue) button.
    pub fn primary(text: impl Into<String>) -> Self {
        Self::new(ButtonKind::Primary, text)
    }

    /// Secondary (outline) button.
    pub fn secondary(text: impl Into<String>) -> Self {
        Self::new(ButtonKind::Secondary, text)
    }

    /// Danger (red) button.
    pub fn danger(text: impl Into<String>) -> Self {
        Self::new(ButtonKind::Danger, text)
    }

    /// Small danger outline button.
    pub fn danger_outline(text: impl Into<String>) -> Self {
        Self::new(ButtonKind::DangerOutline, text)
    }

    /// Stretches the button over the available width.
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }
}

impl Widget for BrandButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let small = self.kind == ButtonKind::DangerOutline;
        let (font_size, height, radius, pad_x) = if small {
            (13.0, 28.0, 14_u8, 12.0)
        } else {
            (14.0, 36.0, theme::R_BUTTON, 18.0)
        };
        let galley = WidgetText::from(RichText::new(self.text.as_str()).size(font_size))
            .into_galley(
                ui,
                Some(TextWrapMode::Truncate),
                (ui.available_width() - 2.0 * pad_x).max(24.0),
                TextStyle::Button,
            );
        let mut width = galley.size().x + 2.0 * pad_x;
        if self.full_width {
            width = width.max(ui.available_width());
        }
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        let enabled = ui.is_enabled();
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &self.text));
        if ui.is_rect_visible(rect) {
            let hovered = enabled && response.hovered();
            let pressed = enabled && response.is_pointer_button_down_on();
            let none = Color32::TRANSPARENT;
            let (fill, stroke, text) = match (self.kind, enabled) {
                (_, false) if !small => (theme::LINE, Stroke::NONE, theme::INK_WEAK),
                (_, false) => (none, Stroke::new(1.5, theme::LINE), theme::INK_WEAK),
                (ButtonKind::Primary, true) => (
                    if pressed {
                        theme::BLUE_PRESSED
                    } else if hovered {
                        theme::BLUE_HOVER
                    } else {
                        theme::BLUE
                    },
                    Stroke::NONE,
                    theme::PALE,
                ),
                (ButtonKind::Secondary, true) => (
                    if pressed {
                        theme::PALE_4
                    } else if hovered {
                        theme::PALE_3
                    } else {
                        none
                    },
                    Stroke::new(2.0, theme::BLUE),
                    theme::BLUE,
                ),
                (ButtonKind::Danger, true) => (
                    if hovered || pressed {
                        theme::DANGER_HOVER
                    } else {
                        theme::DANGER
                    },
                    Stroke::NONE,
                    theme::WHITE,
                ),
                (ButtonKind::DangerOutline, true) => (
                    if hovered || pressed {
                        theme::DANGER_BG
                    } else {
                        none
                    },
                    Stroke::new(1.5, theme::DANGER),
                    theme::DANGER,
                ),
            };
            let painter = ui.painter();
            painter.rect(
                rect,
                CornerRadius::same(radius),
                fill,
                stroke,
                StrokeKind::Inside,
            );
            painter.galley(rect.center() - galley.size() / 2.0, galley, text);
        }
        if enabled {
            response.on_hover_cursor(CursorIcon::PointingHand)
        } else {
            response
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Toggle switch
// ---------------------------------------------------------------------------------------------

/// Paints a pill toggle switch in `rect` and returns its response.
///
/// `position` is `Some(on)` for a known state and `None` for an indeterminate one (no knob).
/// The switch only senses clicks when `interactive` is true.
pub fn toggle_switch(
    ui: &Ui,
    rect: Rect,
    id: egui::Id,
    position: Option<bool>,
    interactive: bool,
) -> Response {
    let sense = if interactive {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    let painter = ui.painter();
    let radius = CornerRadius::same((rect.height() / 2.0).round().clamp(0.0, 255.0) as u8);
    match position {
        None => {
            painter.rect(
                rect,
                radius,
                theme::PALE_2,
                Stroke::new(1.0, theme::LINE),
                StrokeKind::Inside,
            );
        }
        Some(on) => {
            let t = ui.ctx().animate_bool_with_time(id, on, 0.15);
            let mut track = if on {
                theme::SUCCESS_TOGGLE
            } else {
                theme::LINE
            };
            if interactive && response.hovered() {
                track = theme::lighten(track, 0.1);
            }
            painter.rect_filled(rect, radius, track);
            let inset = 3.0;
            let knob_w = (rect.width() * 0.43).max(rect.height());
            let knob_h = rect.height() - 2.0 * inset;
            let x_off = rect.min.x + inset;
            let x_on = rect.max.x - inset - knob_w;
            let x = x_off + (x_on - x_off) * t;
            let knob = Rect::from_min_size(Pos2::new(x, rect.min.y + inset), vec2(knob_w, knob_h));
            let knob_color = if interactive {
                theme::BLUE
            } else {
                theme::INK_WEAK
            };
            painter.rect_filled(
                knob,
                CornerRadius::same((knob_h / 2.0).round().clamp(0.0, 255.0) as u8),
                knob_color,
            );
        }
    }
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::Checkbox,
            interactive,
            position.unwrap_or(false),
            "soos-daemon",
        )
    });
    if interactive {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// Size of a toggle switch at the reference scale.
pub const TOGGLE_SIZE: Vec2 = Vec2::new(76.0, 27.0);

// ---------------------------------------------------------------------------------------------
// Fixed-rectangle cards, status rows and video overlay chips
// ---------------------------------------------------------------------------------------------

/// Paints a brand card exactly over `rect` and runs `add_contents` in a vertically scrolling
/// child area inside its margins (content taller than the card scrolls, never overflows).
pub fn card_in_rect<R>(
    ui: &mut Ui,
    rect: Rect,
    id_salt: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.painter().rect(
        rect,
        CornerRadius::same(theme::R_CARD),
        theme::PALE,
        Stroke::new(theme::STROKE_CARD, theme::BLUE),
        StrokeKind::Inside,
    );
    let inner = Rect::from_min_max(
        rect.min + vec2(18.0, 24.0),
        Pos2::new(rect.max.x - 18.0, rect.max.y - 14.0),
    );
    let output = ui
        .scope_builder(
            egui::UiBuilder::new()
                .id(card_scope_id(ui, id_salt))
                .max_rect(inner),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(id_salt)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Keep a small right margin so the floating scroll bar never covers text.
                        ui.set_width((ui.available_width() - 4.0).max(0.0));
                        let top = ui.min_rect().min.y;
                        let result = add_contents(ui);
                        (result, ui.min_rect().max.y - top)
                    })
                    .inner
            },
        )
        .inner;
    store_content_height(ui, id_salt, output.1);
    output.0
}

/// Inner-table row whose value is preceded by a vector status icon (the default fonts lack
/// the check and cross glyphs). `icon` of `None` shows the value alone.
pub fn table_row_status(
    ui: &mut Ui,
    label: &str,
    icon: Option<BannerKind>,
    value: &str,
    color: Color32,
) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, theme::TABLE_ROW_H),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_size(vec2(width, theme::TABLE_ROW_H));
            let font = FontId::proportional(theme::F_BODY);
            let needed = [label, value]
                .iter()
                .map(|t| {
                    ui.painter()
                        .layout_no_wrap((*t).to_owned(), font.clone(), color)
                        .size()
                        .x
                })
                .sum::<f32>()
                + 36.0;
            // Narrow column: drop the icon rather than overlap the label.
            let icon = icon.filter(|_| needed <= width);
            ui.label(RichText::new(label).size(theme::F_BODY).color(theme::INK));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(value_text(value, color));
                if let Some(kind) = icon {
                    let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                    kind.paint_icon(ui.painter(), rect, color);
                }
            });
        },
    );
}

/// Height of a video overlay chip.
pub const CHIP_H: f32 = 26.0;

/// Floating overlay toggle chips laid out from the top-left of `area`, wrapping onto new rows.
///
/// Each chip flips its `bool` when clicked: blue with a pink dot when on, translucent ink with
/// a hollow dot when off. Returns the rectangle covered by the chips.
///
/// When the chips would not fit on one row they switch to a compact size (8-point padding,
/// 11-point text) first, and only wrap if even that is too wide.
pub fn overlay_chips(ui: &Ui, area: Rect, id_salt: &str, items: &mut [(&str, &mut bool)]) -> Rect {
    let painter = ui.painter_at(area);
    let (dot_r, dot_gap) = (4.0, 7.0);
    let row_width = |font: &FontId, gap: f32, pad_x: f32| -> f32 {
        let chips: f32 = items
            .iter()
            .map(|(label, _)| {
                pad_x
                    + 2.0 * dot_r
                    + dot_gap
                    + painter
                        .layout_no_wrap((*label).to_owned(), font.clone(), theme::PALE)
                        .size()
                        .x
                    + pad_x
            })
            .sum();
        chips + gap * (items.len().saturating_sub(1) as f32)
    };
    let regular = FontId::proportional(theme::F_SMALL);
    let (font, gap, pad_x) = if row_width(&regular, 6.0, 12.0) <= area.width() {
        (regular, 6.0, 12.0)
    } else {
        (FontId::proportional(11.0), 5.0, 8.0)
    };
    let mut cursor = area.min;
    let mut covered = Rect::from_min_size(area.min, Vec2::ZERO);
    for (index, (label, value)) in items.iter_mut().enumerate() {
        let galley = painter.layout_no_wrap((*label).to_owned(), font.clone(), theme::PALE);
        let width = pad_x + 2.0 * dot_r + dot_gap + galley.size().x + pad_x;
        if cursor.x > area.min.x && cursor.x + width > area.max.x {
            cursor = Pos2::new(area.min.x, cursor.y + CHIP_H + gap);
        }
        let rect = Rect::from_min_size(cursor, vec2(width, CHIP_H));
        let response = ui.interact(rect, egui::Id::new((id_salt, index)), Sense::click());
        if response.clicked() {
            **value = !**value;
        }
        let on = **value;
        let hovered = response.hovered();
        let fill = match (on, hovered) {
            (true, false) => theme::BLUE,
            (true, true) => theme::BLUE_HOVER,
            (false, false) => Color32::from_rgba_unmultiplied(16, 24, 32, 150),
            (false, true) => Color32::from_rgba_unmultiplied(16, 24, 32, 175),
        };
        painter.rect_filled(rect, CornerRadius::same(13), fill);
        let dot = Pos2::new(rect.min.x + pad_x + dot_r, rect.center().y);
        if on {
            painter.circle_filled(dot, dot_r, theme::PINK);
        } else {
            painter.circle_stroke(dot, dot_r - 0.6, Stroke::new(1.2, theme::PALE));
        }
        painter.galley(
            Pos2::new(
                dot.x + dot_r + dot_gap,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            theme::PALE,
        );
        response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, *label));
        let _ = response.on_hover_cursor(CursorIcon::PointingHand);
        covered = covered.union(rect);
        cursor.x += width + gap;
    }
    covered
}

// ---------------------------------------------------------------------------------------------
// Checklist rows
// ---------------------------------------------------------------------------------------------

/// State of one checklist step (guided enrollment angles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    /// Already captured.
    Done,
    /// Being captured now.
    InProgress,
    /// Not reached yet.
    Waiting,
}

impl StepStatus {
    /// Human-readable status word, also exposed as the row tooltip.
    pub fn label(self) -> &'static str {
        match self {
            Self::Done => "Done",
            Self::InProgress => "In Progress",
            Self::Waiting => "Waiting",
        }
    }

    /// Text color of the status word.
    pub fn color(self) -> Color32 {
        match self {
            Self::Done => theme::SUCCESS,
            Self::InProgress => theme::BLUE,
            Self::Waiting => theme::INK_MUTED,
        }
    }
}

/// Shared layout of a checklist drawn by [`step_list`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepLayout {
    /// Compact rows (30 points, hint in the tooltip only) instead of 40-point two-line rows.
    pub compact: bool,
    /// Whether the status word is shown at the right of every row (decided once per list).
    pub show_status: bool,
}

/// Height of a checklist row.
pub fn step_row_height(compact: bool) -> f32 {
    if compact {
        30.0
    } else {
        40.0
    }
}

/// Draws a whole checklist of `(name, hint, status)` rows with one [`StepLayout`] decision,
/// so the status word is shown on every row or on none.
pub fn step_list(ui: &mut Ui, steps: &[(&str, &str, StepStatus)], compact: bool) {
    let width = ui.available_width();
    let painter = ui.painter();
    let measure = |text: &str, size: f32| -> f32 {
        painter
            .layout_no_wrap(text.to_owned(), FontId::proportional(size), theme::INK)
            .size()
            .x
    };
    let text_w = steps
        .iter()
        .map(|(name, hint, _)| {
            let name_w = measure(name, theme::F_BODY);
            if compact {
                name_w
            } else {
                name_w.max(measure(hint, 11.0))
            }
        })
        .fold(0.0_f32, f32::max);
    let status_w = [
        StepStatus::Done,
        StepStatus::InProgress,
        StepStatus::Waiting,
    ]
    .iter()
    .map(|s| measure(s.label(), 11.0))
    .fold(0.0_f32, f32::max);
    let layout = StepLayout {
        compact,
        show_status: STEP_TEXT_X + text_w + 10.0 + status_w <= width,
    };
    for (name, hint, status) in steps {
        step_row(ui, name, hint, *status, layout);
    }
}

/// Left offset of the step text (status chip plus its gap).
const STEP_TEXT_X: f32 = 31.0;

/// One checklist row: a vector status chip, the step `name` (with a muted `hint` below it
/// unless the row is compact) and, when `layout.show_status`, the status word right-aligned.
/// The tooltip carries the status and, on compact rows, the hint.
pub fn step_row(
    ui: &mut Ui,
    name: &str,
    hint: &str,
    status: StepStatus,
    layout: StepLayout,
) -> Response {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, step_row_height(layout.compact)), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let chip_r = if layout.compact { 9.0 } else { 10.0 };
        let c = Pos2::new(rect.min.x + 11.0, rect.center().y);
        match status {
            StepStatus::Done => {
                painter.circle_filled(c, chip_r, theme::SUCCESS);
                painter.add(egui::Shape::line(
                    vec![
                        c + vec2(-4.5, 0.2),
                        c + vec2(-1.2, 3.6),
                        c + vec2(4.8, -3.4),
                    ],
                    Stroke::new(2.0, theme::PALE),
                ));
            }
            StepStatus::InProgress => {
                painter.circle_filled(c, chip_r, theme::BLUE);
                painter.circle_filled(c, 3.5, theme::PINK);
            }
            StepStatus::Waiting => {
                painter.circle_stroke(c, chip_r - 0.75, Stroke::new(1.5, theme::LINE));
            }
        }
        let text_x = rect.min.x + STEP_TEXT_X;
        let name_color = if status == StepStatus::Done {
            theme::INK_MUTED
        } else {
            theme::INK
        };
        let status_galley = painter.layout_no_wrap(
            status.label().to_owned(),
            FontId::proportional(11.0),
            status.color(),
        );
        let name_galley = painter.layout_no_wrap(
            name.to_owned(),
            FontId::proportional(theme::F_BODY),
            name_color,
        );
        let hint_galley = (!layout.compact).then(|| {
            painter.layout_no_wrap(
                hint.to_owned(),
                FontId::proportional(11.0),
                theme::INK_MUTED,
            )
        });
        let text_right = if layout.show_status {
            rect.max.x - status_galley.size().x - 10.0
        } else {
            rect.max.x
        };
        let text_clip = Rect::from_min_max(
            Pos2::new(text_x - 2.0, rect.min.y),
            Pos2::new(text_right.max(text_x), rect.max.y),
        )
        .intersect(ui.clip_rect());
        let clipped = painter.with_clip_rect(text_clip);
        let hint_h = hint_galley.as_ref().map_or(0.0, |g| g.size().y);
        let top = rect.center().y - (name_galley.size().y + hint_h) / 2.0;
        let name_h = name_galley.size().y;
        let passes = if status == StepStatus::InProgress {
            2
        } else {
            1
        };
        theme::paint_galley_faux_bold(
            &clipped,
            Pos2::new(text_x, top),
            Align2::LEFT_TOP,
            &name_galley,
            name_color,
            passes,
            0.5,
        );
        if let Some(hint_galley) = hint_galley {
            clipped.galley(
                Pos2::new(text_x, top + name_h),
                hint_galley,
                theme::INK_MUTED,
            );
        }
        if layout.show_status {
            painter.galley(
                Pos2::new(
                    rect.max.x - status_galley.size().x,
                    rect.center().y - status_galley.size().y / 2.0,
                ),
                status_galley,
                status.color(),
            );
        }
    }
    if layout.compact {
        response.on_hover_text(format!("{hint} ({})", status.label()))
    } else {
        response.on_hover_text(status.label())
    }
}

/// Like [`card_in_rect`], but keeps a fixed footer of `footer_h` points at the bottom of the
/// card (outside the scroll area) so its actions stay visible however long the content is.
///
/// Returns the scroll content's result and the footer rectangle, separated from the content
/// by a thin line; the caller lays its actions out inside that rectangle.
pub fn card_in_rect_with_footer<R>(
    ui: &mut Ui,
    rect: Rect,
    id_salt: &str,
    footer_h: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> (R, Rect) {
    let footer_h = footer_h.clamp(0.0, (rect.height() - 80.0).max(0.0));
    let footer = Rect::from_min_max(
        Pos2::new(rect.min.x + 18.0, rect.max.y - 16.0 - footer_h),
        Pos2::new(rect.max.x - 18.0, rect.max.y - 16.0),
    );
    ui.painter().rect(
        rect,
        CornerRadius::same(theme::R_CARD),
        theme::PALE,
        Stroke::new(theme::STROKE_CARD, theme::BLUE),
        StrokeKind::Inside,
    );
    let inner = Rect::from_min_max(
        rect.min + vec2(18.0, 24.0),
        Pos2::new(
            rect.max.x - 18.0,
            (footer.min.y - 12.0).max(rect.min.y + 24.0),
        ),
    );
    let (result, content_h) = ui
        .scope_builder(
            egui::UiBuilder::new()
                .id(card_scope_id(ui, id_salt))
                .max_rect(inner),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(id_salt)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Keep a small right margin so the floating scroll bar never covers text.
                        ui.set_width((ui.available_width() - 4.0).max(0.0));
                        let top = ui.min_rect().min.y;
                        let result = add_contents(ui);
                        (result, ui.min_rect().max.y - top)
                    })
                    .inner
            },
        )
        .inner;
    store_content_height(ui, id_salt, content_h);
    ui.painter().hline(
        footer.x_range(),
        footer.min.y - 5.0,
        Stroke::new(1.0, theme::LINE),
    );
    (result, footer)
}

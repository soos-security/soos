//! Surface geometry and memory bounds.

use soos_lockstatus::motion::SLIDE_PX;

/// Layer surfaces created at most (one per output).
pub const MAX_OUTPUTS: usize = 8;
/// Largest buffer side in pixels.
pub const MAX_SURFACE_DIM: u32 = 8192;
/// Largest buffer of one surface in pixels (an 8K output fits; 128 MiB per buffer).
pub const MAX_SURFACE_PIXELS: u64 = 33_554_432;
/// Largest sum of surface pixels across outputs.
pub const MAX_TOTAL_PIXELS: u64 = 67_108_864;
/// Buffers per surface (the compositor may hold one while the other is drawn).
pub const BUFFERS_PER_SURFACE: usize = 2;
/// Vertical position of the widget centre (below swaylock's centred indicator).
pub const WIDGET_CENTER_Y_FRACTION: f32 = 0.78;

/// Ring radius as a fraction of the shorter surface side.
const RADIUS_FRACTION: f32 = 0.045;
/// Smallest ring radius in pixels.
const MIN_RADIUS_PX: f32 = 20.0;
/// Largest ring radius in pixels.
const MAX_RADIUS_PX: f32 = 72.0;
/// Radius divisor giving the stroke width and the label scale.
const STROKE_DIVISOR: f32 = 12.0;
/// Thinnest ring stroke in pixels.
const MIN_STROKE_PX: f32 = 2.0;
/// Smallest integer label scale.
const MIN_LABEL_SCALE: u32 = 2;
/// Largest integer label scale.
const MAX_LABEL_SCALE: u32 = 6;
/// Star half-size as a fraction of the ring radius.
const STAR_HALF_FRACTION: f32 = 0.55;
/// Margin around everything the widget draws.
const BOX_MARGIN_PX: f32 = 2.0;

/// Integer pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// Whether `(x, y)` lies inside.
    #[must_use]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x
            && y >= self.y
            && x.saturating_sub(self.x) < self.w
            && y.saturating_sub(self.y) < self.h
    }

    /// Exclusive right edge (saturating).
    #[must_use]
    pub fn right(&self) -> u32 {
        self.x.saturating_add(self.w)
    }

    /// Exclusive bottom edge (saturating).
    #[must_use]
    pub fn bottom(&self) -> u32 {
        self.y.saturating_add(self.h)
    }

    /// Intersection with `other` (empty when disjoint).
    #[must_use]
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Rect {
            x,
            y,
            w: right.saturating_sub(x),
            h: bottom.saturating_sub(y),
        }
    }
}

/// Bytes of one ARGB8888 buffer of `width` × `height`; `None` when zero, above
/// [`MAX_SURFACE_DIM`] or [`MAX_SURFACE_PIXELS`], or overflowing.
#[must_use]
pub fn buffer_len(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || width > MAX_SURFACE_DIM || height > MAX_SURFACE_DIM {
        return None;
    }
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    if pixels > MAX_SURFACE_PIXELS {
        return None;
    }
    usize::try_from(pixels.checked_mul(4)?).ok()
}

/// Saturating float-to-pixel conversion (`NaN` and negatives → 0).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "`as` saturates to [0, u32::MAX] and maps NaN to 0: the documented semantics"
)]
pub(crate) fn to_px(value: f32) -> u32 {
    value as u32
}

/// Exact conversion of a pixel count to `f32` (every count used here is below 2^24).
#[allow(
    clippy::cast_precision_loss,
    reason = "surface sizes are at most MAX_SURFACE_DIM (8192) and label widths far below 2^24"
)]
pub(crate) fn px_f32(value: u32) -> f32 {
    value as f32
}

/// Widget geometry of one surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// Surface width.
    pub width: u32,
    /// Surface height.
    pub height: u32,
    /// Ring centre x.
    pub cx: f32,
    /// Ring centre y.
    pub cy: f32,
    /// Ring radius (centre of the stroke).
    pub radius: f32,
    /// Ring stroke width.
    pub stroke: f32,
    /// Half size of the star.
    pub star_half: f32,
    /// Integer label scale.
    pub label_scale: u32,
    /// Top of the label.
    pub label_top: f32,
    /// Everything the widget may draw, at every shake offset, clipped to the surface.
    pub widget: Rect,
}

impl Layout {
    /// Geometry for a `width` × `height` surface; `None` when either is zero.
    #[must_use]
    pub fn for_surface(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let (w, h) = (px_f32(width), px_f32(height));
        let radius = (w.min(h) * RADIUS_FRACTION)
            .round()
            .clamp(MIN_RADIUS_PX, MAX_RADIUS_PX);
        let stroke = (radius / STROKE_DIVISOR).round().max(MIN_STROKE_PX);
        let label_scale = to_px(radius / STROKE_DIVISOR).clamp(MIN_LABEL_SCALE, MAX_LABEL_SCALE);
        let cx = w / 2.0;
        let cy = h * WIDGET_CENTER_Y_FRACTION;
        let label_top = cy + radius * 1.5;
        let label_half = px_f32(crate::font::text_width(
            soos_lockstatus::format::NOT_RECOGNIZED_TEXT,
            label_scale,
        )) / 2.0;
        let reach_x = (radius + stroke / 2.0).max(label_half) + SLIDE_PX + BOX_MARGIN_PX;
        let top = cy - radius - stroke / 2.0 - BOX_MARGIN_PX;
        let bottom =
            label_top + px_f32(crate::font::GLYPH_H.saturating_mul(label_scale)) + BOX_MARGIN_PX;
        let x0 = to_px((cx - reach_x).floor()).min(width);
        let x1 = to_px((cx + reach_x).ceil()).min(width);
        let y0 = to_px(top.floor()).min(height);
        let y1 = to_px(bottom.ceil()).min(height);
        let widget = Rect {
            x: x0,
            y: y0,
            w: x1.saturating_sub(x0),
            h: y1.saturating_sub(y0),
        };
        Some(Self {
            width,
            height,
            cx,
            cy,
            radius,
            stroke,
            star_half: STAR_HALF_FRACTION * radius,
            label_scale,
            label_top,
            widget,
        })
    }
}

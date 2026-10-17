//! CPU drawing into one ARGB8888 little-endian buffer (bytes `B, G, R, A`, alpha `0xFF`).
//!
//! Every primitive clips to the canvas and to a caller rectangle, draws nothing for non-finite or
//! non-positive geometry, and allocates nothing.

use crate::font::{self, GLYPH_GAP, GLYPH_H, GLYPH_W};
use crate::layout::{buffer_len, px_f32, to_px, Rect};
use crate::palette::{mix, Rgb};

/// Subsamples per axis of the star coverage.
const STAR_SUPERSAMPLE: u32 = 4;
/// Upper bound of any coordinate accepted by a primitive (far outside any canvas).
const MAX_COORD: f32 = 1.0e6;

/// A borrowed drawing target of exactly `width` × `height` pixels.
#[derive(Debug)]
pub struct Canvas<'a> {
    data: &'a mut [u8],
    width: u32,
    height: u32,
}

impl<'a> Canvas<'a> {
    /// `None` unless `data` holds at least `buffer_len(width, height)` bytes.
    #[must_use]
    pub fn new(data: &'a mut [u8], width: u32, height: u32) -> Option<Self> {
        let needed = buffer_len(width, height)?;
        (data.len() >= needed).then_some(Self {
            data,
            width,
            height,
        })
    }

    /// Canvas width.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Canvas height.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    fn bounds(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height,
        }
    }

    fn offset(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = u64::from(y)
            .checked_mul(u64::from(self.width))?
            .checked_add(u64::from(x))?
            .checked_mul(4)?;
        usize::try_from(index).ok()
    }

    /// Colour of one pixel, `None` outside the canvas.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgb> {
        let at = self.offset(x, y)?;
        let px = self.data.get(at..at.checked_add(4)?)?;
        match px {
            [b, g, r, _] => Some(Rgb(*r, *g, *b)),
            _ => None,
        }
    }

    fn put(&mut self, x: u32, y: u32, color: Rgb) {
        let Some(at) = self.offset(x, y) else {
            return;
        };
        let Some(end) = at.checked_add(4) else {
            return;
        };
        if let Some(px) = self.data.get_mut(at..end) {
            px.copy_from_slice(&[color.2, color.1, color.0, 0xFF]);
        }
    }

    fn blend(&mut self, x: u32, y: u32, color: Rgb, coverage: f32) {
        // Also rejects NaN coverage.
        if coverage.is_nan() || coverage <= 0.0 {
            return;
        }
        if coverage >= 1.0 {
            self.put(x, y, color);
            return;
        }
        if let Some(old) = self.pixel(x, y) {
            self.put(x, y, mix(old, color, coverage));
        }
    }

    /// Fills `rect` (clipped to the canvas).
    pub fn fill_rect(&mut self, rect: Rect, color: Rgb) {
        let area = rect.intersect(&self.bounds());
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                self.put(x, y, color);
            }
        }
    }

    /// Pixel range covering `[lo, hi]` inside `clip` and the canvas.
    fn span(&self, lo_x: f32, hi_x: f32, lo_y: f32, hi_y: f32, clip: Rect) -> Rect {
        let area = clip.intersect(&self.bounds());
        let x0 = to_px(lo_x.floor()).max(area.x);
        let x1 = to_px(hi_x.ceil()).min(area.right());
        let y0 = to_px(lo_y.floor()).max(area.y);
        let y1 = to_px(hi_y.ceil()).min(area.bottom());
        Rect {
            x: x0,
            y: y0,
            w: x1.saturating_sub(x0),
            h: y1.saturating_sub(y0),
        }
    }

    /// Anti-aliased ring blended over the canvas inside `clip`; non-finite or non-positive
    /// geometry draws nothing.
    pub fn ring(&mut self, cx: f32, cy: f32, radius: f32, stroke: f32, color: Rgb, clip: Rect) {
        if !sane(cx) || !sane(cy) || !sane(radius) || !sane(stroke) {
            return;
        }
        if radius <= 0.0 || stroke <= 0.0 {
            return;
        }
        let reach = radius + stroke / 2.0 + 1.0;
        let area = self.span(cx - reach, cx + reach, cy - reach, cy + reach, clip);
        let half = stroke / 2.0;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let dx = px_f32(x) + 0.5 - cx;
                let dy = px_f32(y) + 0.5 - cy;
                let distance = dx.hypot(dy);
                let coverage = (half + 0.5 - (distance - radius).abs()).clamp(0.0, 1.0);
                self.blend(x, y, color, coverage);
            }
        }
    }

    /// The brand star of half-size `half` centred on `(cx, cy)`, turned `angle_deg` clockwise,
    /// 4 × 4 supersampled, inside `clip`.
    pub fn star(&mut self, cx: f32, cy: f32, half: f32, angle_deg: f32, color: Rgb, clip: Rect) {
        if !sane(cx) || !sane(cy) || !sane(half) || half <= 0.0 {
            return;
        }
        let angle = if angle_deg.is_finite() {
            angle_deg.to_radians()
        } else {
            0.0
        };
        let (sin, cos) = angle.sin_cos();
        let reach = half * std::f32::consts::SQRT_2 + 1.0;
        let area = self.span(cx - reach, cx + reach, cy - reach, cy + reach, clip);
        let step = 1.0 / px_f32(STAR_SUPERSAMPLE);
        let total = px_f32(STAR_SUPERSAMPLE * STAR_SUPERSAMPLE);
        let size = 2.0 * half;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let mut hits = 0u32;
                for sy in 0..STAR_SUPERSAMPLE {
                    for sx in 0..STAR_SUPERSAMPLE {
                        let px = px_f32(x) + (px_f32(sx) + 0.5) * step - cx;
                        let py = px_f32(y) + (px_f32(sy) + 0.5) * step - cy;
                        // Inverse of the clockwise screen rotation (y pointing down).
                        let ux = px * cos + py * sin;
                        let uy = -px * sin + py * cos;
                        if star_contains(0.5 + ux / size, 0.5 + uy / size) {
                            hits = hits.saturating_add(1);
                        }
                    }
                }
                self.blend(x, y, color, px_f32(hits) / total);
            }
        }
    }

    /// Bitmap text centred on `cx` with its top at `top`, integer `scale`, inside `clip`.
    pub fn text(&mut self, cx: f32, top: f32, scale: u32, text: &str, color: Rgb, clip: Rect) {
        if !sane(cx) || !sane(top) || scale == 0 || text.is_empty() {
            return;
        }
        let width = px_f32(font::text_width(text, scale));
        let left = (cx - width / 2.0).round();
        let top = top.round();
        let area = clip.intersect(&self.bounds());
        let cell = px_f32(GLYPH_W.saturating_add(GLYPH_GAP).saturating_mul(scale));
        let dot = px_f32(scale);
        for (index, c) in text.chars().enumerate() {
            let Some(rows) = font::glyph(c) else {
                continue;
            };
            let origin = left + px_f32(u32::try_from(index).unwrap_or(u32::MAX)) * cell;
            for (row, bits) in (0..GLYPH_H).zip(rows) {
                for col in 0..GLYPH_W {
                    let shift = GLYPH_W.saturating_sub(1).saturating_sub(col);
                    let mask = 1u8.checked_shl(shift).unwrap_or(0);
                    if bits & mask == 0 {
                        continue;
                    }
                    let x = origin + px_f32(col) * dot;
                    let y = top + px_f32(row) * dot;
                    if x < 0.0 || y < 0.0 {
                        continue;
                    }
                    let block = Rect {
                        x: to_px(x),
                        y: to_px(y),
                        w: scale,
                        h: scale,
                    }
                    .intersect(&area);
                    self.fill_rect(block, color);
                }
            }
        }
    }
}

/// Finite and within `±MAX_COORD`.
fn sane(value: f32) -> bool {
    value.is_finite() && value.abs() <= MAX_COORD
}

/// The brand star in the unit square (`v` pointing down): the top-right and bottom-left quadrants
/// minus the quarter discs of radius 0.5 centred on the outer corners (`STAR_SPIKES` of
/// `crates/gui/src/brand.rs`).
#[must_use]
pub fn star_contains(u: f32, v: f32) -> bool {
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return false;
    }
    let top_right = u >= 0.5 && v <= 0.5 && (1.0 - u).hypot(v) >= 0.5;
    let bottom_left = u <= 0.5 && v >= 0.5 && u.hypot(1.0 - v) >= 0.5;
    top_right || bottom_left
}

//! soos brand marks drawn without any image or SVG loader.
//!
//! The "SOOS" wordmark (`text_logo.svg`, view box 514 x 64) and the four-quadrant star mark
//! (`Icon_logo.svg`, view box 508 x 508) are kept here as their original absolute path
//! commands. Because egui only fills convex polygons, the paths are flattened and scan-converted
//! by a small anti-aliased rasterizer into an alpha mask, uploaded once as a texture and cached
//! per size, color and pixel density (never per frame). The window icon is computed
//! analytically from the same geometry.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "Rasterizer coordinate math on bounded, clamped f32 values and small pixel counts"
)]

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

/// One absolute SVG path command (only the commands used by the brand files).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    /// Move to `(x, y)`, starting a new subpath.
    M(f32, f32),
    /// Line to `(x, y)`.
    L(f32, f32),
    /// Horizontal line to `x`.
    H(f32),
    /// Vertical line to `y`.
    V(f32),
    /// Cubic Bezier with two control points and an end point.
    C(f32, f32, f32, f32, f32, f32),
    /// Close the current subpath.
    Z,
}

use Cmd::{C, H, L, M, V, Z};

/// View box of `text_logo.svg`.
pub const WORDMARK_VIEWBOX: Vec2 = Vec2::new(514.0, 64.0);
/// View box of `Icon_logo.svg`.
pub const STAR_VIEWBOX: Vec2 = Vec2::new(508.0, 508.0);

/// The six `<path>` elements of `text_logo.svg` (each one filled with the nonzero rule).
pub const WORDMARK: [&[Cmd]; 6] = [
    &[
        M(128.5, 32.0),
        H(112.438),
        V(16.0),
        H(58.3913),
        C(54.7967, 16.0001, 53.9151, 20.9841, 57.2932, 22.2078),
        L(128.5, 48.0),
        V(56.0),
        C(128.5, 60.4183, 124.904, 64.0, 120.469, 64.0),
        H(0.0),
        V(32.0),
        H(16.0625),
        V(48.0),
        H(70.1087),
        C(73.7034, 48.0, 74.585, 43.0159, 71.2068, 41.7922),
        L(0.0, 16.0),
        V(8.0),
        C(0.0, 3.58172, 3.59571, 0.0, 8.03125, 0.0),
        H(128.5),
        V(32.0),
        Z,
    ],
    &[
        M(514.0, 32.0),
        H(497.938),
        V(16.0),
        H(443.891),
        C(440.297, 16.0001, 439.415, 20.9841, 442.793, 22.2078),
        L(514.0, 48.0),
        V(56.0),
        C(514.0, 60.4183, 510.404, 64.0, 505.969, 64.0),
        H(385.5),
        V(32.0),
        H(401.562),
        V(48.0),
        H(455.609),
        C(459.203, 48.0, 460.085, 43.0159, 456.707, 41.7922),
        L(385.5, 16.0),
        V(8.0),
        C(385.5, 3.58172, 389.096, 0.0, 393.531, 0.0),
        H(514.0),
        V(32.0),
        Z,
    ],
    &[
        M(257.0, 48.0),
        C(257.0, 56.8366, 249.809, 64.0, 240.938, 64.0),
        H(144.562),
        C(135.691, 64.0, 128.5, 56.8366, 128.5, 48.0),
        H(257.0),
        Z,
        M(240.938, 0.0),
        C(249.809, 0.0, 257.0, 7.16344, 257.0, 16.0),
        V(32.0),
        H(240.938),
        V(24.0),
        C(240.938, 19.5817, 237.342, 16.0, 232.906, 16.0),
        H(152.594),
        C(148.158, 16.0, 144.562, 19.5817, 144.562, 24.0),
        V(32.0),
        H(128.5),
        V(16.0),
        C(128.5, 7.16344, 135.691, 0.0, 144.562, 0.0),
        H(240.938),
        Z,
    ],
    &[
        M(240.938, 48.0),
        H(257.0),
        C(257.0, 39.1634, 249.809, 32.0, 240.938, 32.0),
        V(48.0),
        Z,
    ],
    &[
        M(257.0, 32.0),
        H(273.062),
        V(40.0),
        C(273.062, 44.4183, 276.658, 48.0, 281.094, 48.0),
        H(361.406),
        C(365.842, 48.0, 369.438, 44.4183, 369.438, 40.0),
        V(32.0),
        H(385.5),
        V(48.0),
        C(385.5, 56.8366, 378.309, 64.0, 369.438, 64.0),
        H(273.062),
        C(264.191, 64.0, 257.0, 56.8366, 257.0, 48.0),
        V(32.0),
        Z,
        M(369.438, 0.0),
        C(378.309, 0.0, 385.5, 7.16344, 385.5, 16.0),
        H(363.013),
        V(16.1594),
        C(362.494, 16.0545, 361.956, 16.0, 361.406, 16.0),
        H(257.0),
        C(257.0, 7.16344, 264.191, 0.0, 273.062, 0.0),
        H(369.438),
        Z,
    ],
    &[
        M(273.062, 16.0),
        H(257.0),
        C(257.0, 24.8366, 264.191, 32.0, 273.062, 32.0),
        V(16.0),
        Z,
    ],
];

/// The two concave spikes of the star mark: the parts of `Icon_logo.svg` left uncovered by the
/// blue squares and quarter discs (top-right and bottom-left quadrants).
pub const STAR_SPIKES: [&[Cmd]; 2] = [
    &[
        M(254.0, 0.0),
        C(254.0, 140.28, 367.72, 254.0, 508.0, 254.0),
        L(254.0, 254.0),
        Z,
    ],
    &[
        M(0.0, 254.0),
        C(140.28, 254.0, 254.0, 367.72, 254.0, 508.0),
        V(254.0),
        H(0.0),
        Z,
    ],
];

/// Number of line segments per flattened cubic Bezier.
const CUBIC_SEGMENTS: u16 = 16;

/// Flattens `cmds` into closed polylines (one per subpath) in view-box coordinates.
pub fn flatten(cmds: &[Cmd]) -> Vec<Vec<Pos2>> {
    let mut out = Vec::new();
    let mut current: Vec<Pos2> = Vec::new();
    let mut pen = Pos2::ZERO;
    let mut start = Pos2::ZERO;
    let finish = |current: &mut Vec<Pos2>, out: &mut Vec<Vec<Pos2>>| {
        if current.len() >= 3 {
            out.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };
    for cmd in cmds {
        match *cmd {
            M(x, y) => {
                finish(&mut current, &mut out);
                pen = Pos2::new(x, y);
                start = pen;
                current.push(pen);
            }
            L(x, y) => {
                pen = Pos2::new(x, y);
                current.push(pen);
            }
            H(x) => {
                pen = Pos2::new(x, pen.y);
                current.push(pen);
            }
            V(y) => {
                pen = Pos2::new(pen.x, y);
                current.push(pen);
            }
            C(x1, y1, x2, y2, x, y) => {
                let p0 = pen;
                let p1 = Pos2::new(x1, y1);
                let p2 = Pos2::new(x2, y2);
                let p3 = Pos2::new(x, y);
                for i in 1..=CUBIC_SEGMENTS {
                    let t = f32::from(i) / f32::from(CUBIC_SEGMENTS);
                    let u = 1.0 - t;
                    let w0 = u * u * u;
                    let w1 = 3.0 * u * u * t;
                    let w2 = 3.0 * u * t * t;
                    let w3 = t * t * t;
                    current.push(Pos2::new(
                        w0 * p0.x + w1 * p1.x + w2 * p2.x + w3 * p3.x,
                        w0 * p0.y + w1 * p1.y + w2 * p2.y + w3 * p3.y,
                    ));
                }
                pen = p3;
            }
            Z => {
                finish(&mut current, &mut out);
                pen = start;
            }
        }
    }
    finish(&mut current, &mut out);
    out
}

/// Returns the largest rectangle of aspect `view` centered inside `target`.
pub fn fit_aspect(target: Rect, view: Vec2) -> Rect {
    if view.x <= 0.0 || view.y <= 0.0 {
        return Rect::from_center_size(target.center(), Vec2::ZERO);
    }
    let scale = (target.width() / view.x)
        .min(target.height() / view.y)
        .max(0.0);
    Rect::from_center_size(target.center(), view * scale)
}

fn map_shapes(shapes: &[&[Cmd]], view: Vec2, rect: Rect) -> Vec<Vec<Vec<Pos2>>> {
    // An inverted rect is normalized and a non-finite one maps to nothing, so the clamping
    // below always has `min <= max` (`f32::clamp` would panic otherwise).
    if !(rect.min.x.is_finite()
        && rect.min.y.is_finite()
        && rect.max.x.is_finite()
        && rect.max.y.is_finite())
    {
        return Vec::new();
    }
    let rect = Rect::from_two_pos(rect.min, rect.max);
    let sx = rect.width() / view.x;
    let sy = rect.height() / view.y;
    shapes
        .iter()
        .map(|cmds| {
            flatten(cmds)
                .into_iter()
                .map(|poly| {
                    poly.into_iter()
                        .map(|p| {
                            Pos2::new(
                                (rect.min.x + p.x * sx).max(rect.min.x).min(rect.max.x),
                                (rect.min.y + p.y * sy).max(rect.min.y).min(rect.max.y),
                            )
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}

/// Wordmark shapes (one entry per SVG `<path>`, each with its subpaths) mapped onto `rect`.
pub fn wordmark_paths(rect: Rect) -> Vec<Vec<Vec<Pos2>>> {
    map_shapes(&WORDMARK, WORDMARK_VIEWBOX, rect)
}

/// All wordmark subpaths mapped onto `rect`, as closed polylines.
pub fn wordmark_polylines(rect: Rect) -> Vec<Vec<Pos2>> {
    wordmark_paths(rect).into_iter().flatten().collect()
}

/// Star spike shapes mapped onto `rect`.
pub fn star_paths(rect: Rect) -> Vec<Vec<Vec<Pos2>>> {
    map_shapes(&STAR_SPIKES, STAR_VIEWBOX, rect)
}

/// All star spike subpaths mapped onto `rect`, as closed polylines.
pub fn star_polylines(rect: Rect) -> Vec<Vec<Pos2>> {
    star_paths(rect).into_iter().flatten().collect()
}

/// Subsamples per pixel along each axis.
const SUPERSAMPLE: usize = 4;

/// Scan-converts `shapes` into an 8-bit coverage mask of `out_w` x `out_h` pixels.
///
/// `shapes` are in a coordinate space `view` wide and high whose origin is the mask's
/// top-left corner. Each shape is filled with the nonzero rule; the result is the union of all
/// shapes (computed per subsample, so abutting shapes leave no seam).
pub fn rasterize(shapes: &[Vec<Vec<Pos2>>], view: Vec2, out_w: usize, out_h: usize) -> Vec<u8> {
    let mut hits = vec![0u16; out_w.saturating_mul(out_h)];
    if out_w == 0 || out_h == 0 || view.x <= 0.0 || view.y <= 0.0 {
        return vec![0; hits.len()];
    }
    let to_px_x = out_w as f32 / view.x;
    let to_view_y = view.y / out_h as f32;
    let ss = SUPERSAMPLE as f32;
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    let mut spans: Vec<(f32, f32)> = Vec::new();

    for (py, row) in hits.chunks_exact_mut(out_w).enumerate() {
        for sy in 0..SUPERSAMPLE {
            let y = (py as f32 + (sy as f32 + 0.5) / ss) * to_view_y;
            spans.clear();
            for shape in shapes {
                crossings.clear();
                for poly in shape {
                    let n = poly.len();
                    for (i, a) in poly.iter().enumerate() {
                        let Some(b) = poly.get((i + 1) % n) else {
                            continue;
                        };
                        let dir = if a.y <= y && b.y > y {
                            1
                        } else if b.y <= y && a.y > y {
                            -1
                        } else {
                            continue;
                        };
                        let x = a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x);
                        crossings.push((x * to_px_x, dir));
                    }
                }
                crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
                let mut winding = 0;
                for pair in crossings.windows(2) {
                    let (Some(&(x0, d)), Some(&(x1, _))) = (pair.first(), pair.get(1)) else {
                        continue;
                    };
                    winding += d;
                    if winding != 0 && x1 > x0 {
                        spans.push((x0, x1));
                    }
                }
            }
            for &(x0, x1) in &spans {
                // Subsample columns whose centers fall in [x0, x1).
                let first = (x0 * ss - 0.5).ceil().max(0.0) as usize;
                let last = ((x1 * ss - 0.5).ceil().max(0.0) as usize).min(out_w * SUPERSAMPLE);
                let mut k = first;
                while k < last {
                    if let Some(h) = row.get_mut(k / SUPERSAMPLE) {
                        *h = h.saturating_add(1);
                    }
                    k += 1;
                }
            }
        }
        // Overlapping shapes may hit the same subsample twice: clamp to full coverage.
        for h in row.iter_mut() {
            *h = (*h).min((SUPERSAMPLE * SUPERSAMPLE) as u16);
        }
    }
    let full = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    hits.into_iter()
        .map(|h| (f32::from(h) / full * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect()
}

/// Converts a coverage mask into a color image tinted with `color`.
pub fn mask_to_color_image(
    coverage: &[u8],
    width: usize,
    height: usize,
    color: Color32,
) -> egui::ColorImage {
    let pixels = coverage
        .iter()
        .map(|&c| Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), c))
        .collect();
    egui::ColorImage::new([width, height], pixels)
}

/// Which brand mark a cached texture holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Mark {
    Wordmark,
    Star,
}

#[derive(Clone)]
struct CachedMark {
    key: (usize, usize, [u8; 4]),
    texture: egui::TextureHandle,
}

fn paint_mark(ui: &egui::Ui, mark: Mark, target: Rect, color: Color32) {
    let view = match mark {
        Mark::Wordmark => WORDMARK_VIEWBOX,
        Mark::Star => STAR_VIEWBOX,
    };
    let rect = fit_aspect(target, view);
    let ppp = ui.ctx().pixels_per_point();
    let w = (rect.width() * ppp).round().clamp(1.0, 4096.0) as usize;
    let h = (rect.height() * ppp).round().clamp(1.0, 4096.0) as usize;
    let key = (w, h, color.to_array());
    // One cache slot per mark, color and size class: a resized window replaces the texture
    // instead of accumulating stale ones.
    let id = egui::Id::new(("soos_brand_mark", mark, key.2, w / 64, h / 64));
    let cached: Option<CachedMark> = ui.ctx().data(|d| d.get_temp(id));
    let texture = match cached {
        Some(c) if c.key == key => c.texture,
        _ => {
            let local = Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32));
            let shapes = match mark {
                Mark::Wordmark => wordmark_paths(local),
                Mark::Star => star_paths(local),
            };
            let coverage = rasterize(&shapes, local.size(), w, h);
            let image = mask_to_color_image(&coverage, w, h, color);
            let texture = ui.ctx().load_texture(
                format!("soos_brand_{mark:?}_{w}x{h}"),
                image,
                egui::TextureOptions::LINEAR,
            );
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    id,
                    CachedMark {
                        key,
                        texture: texture.clone(),
                    },
                );
            });
            texture
        }
    };
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// Paints the "SOOS" wordmark in `color`, fitted (aspect preserved) and centered in `rect`.
pub fn paint_wordmark(ui: &egui::Ui, rect: Rect, color: Color32) {
    paint_mark(ui, Mark::Wordmark, rect, color);
}

/// Paints the two star spikes in `color`, fitted (square) and centered in `rect`.
pub fn paint_star(ui: &egui::Ui, rect: Rect, color: Color32) {
    paint_mark(ui, Mark::Star, rect, color);
}

/// Whether the normalized point `(u, v)` (v pointing down) is blue in `Icon_logo.svg`.
fn icon_is_blue(u: f32, v: f32) -> bool {
    let left = u < 0.5;
    let top = v < 0.5;
    match (left, top) {
        (true, true) | (false, false) => true,
        (true, false) => u.hypot(1.0 - v) <= 0.5,
        (false, true) => (1.0 - u).hypot(v) <= 0.5,
    }
}

/// Builds the procedural window icon (`size` x `size`, rounded up to a multiple of 4).
pub fn window_icon(size: u32) -> egui::IconData {
    let size = size.max(4).div_ceil(4) * 4;
    let n = size as usize;
    let blue = [0x00_u8, 0x47, 0xBB];
    let pale = [0xED_u8, 0xF1, 0xFF];
    let mut rgba = Vec::with_capacity(n * n * 4);
    let ss = SUPERSAMPLE as f32;
    let total = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    for y in 0..n {
        for x in 0..n {
            let mut blue_hits = 0.0_f32;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let u = (x as f32 + (sx as f32 + 0.5) / ss) / n as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / ss) / n as f32;
                    if icon_is_blue(u, v) {
                        blue_hits += 1.0;
                    }
                }
            }
            let t = blue_hits / total;
            for (b, p) in blue.iter().zip(pale.iter()) {
                let c = f32::from(*b) * t + f32::from(*p) * (1.0 - t);
                rgba.push(c.round().clamp(0.0, 255.0) as u8);
            }
            rgba.push(255);
        }
    }
    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

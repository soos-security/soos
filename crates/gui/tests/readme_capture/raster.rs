//! Deterministic CPU rasterizer of tessellated egui output (spec §3.2, D-1).
//!
//! Pixel-centre sampling with the top-left fill rule, barycentric interpolation of the
//! premultiplied vertex colour and UV, bilinear (or nearest) texture fetch with clamp-to-edge,
//! and `out = src + dst * (1 - src.a)` blending in gamma space, which is what `egui_glow` does
//! on a non-sRGB framebuffer. Pure `f32` arithmetic without platform intrinsics: the output is
//! identical on every runner.

use std::collections::HashMap;

use eframe::egui::epaint::{ClippedPrimitive, ImageData, Primitive, TextureId, Vertex};
use eframe::egui::{Color32, TextureFilter, TexturesDelta};

/// Largest image or texture side accepted (spec §3.2).
pub const MAX_SIDE: usize = 4096;

/// An opaque RGBA8 image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA8 pixels (alpha always 255 after [`Rasterizer::render`]).
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone)]
struct Texture {
    width: usize,
    height: usize,
    /// Premultiplied RGBA, 0..=1.
    texels: Vec<[f32; 4]>,
    linear: bool,
}

impl Texture {
    fn texel(&self, x: usize, y: usize) -> [f32; 4] {
        self.texels
            .get(y * self.width + x)
            .copied()
            .unwrap_or([0.0; 4])
    }

    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [0.0; 4];
        }
        let max_x = (self.width - 1) as f32;
        let max_y = (self.height - 1) as f32;
        if !self.linear {
            let x = (u * self.width as f32).floor().clamp(0.0, max_x) as usize;
            let y = (v * self.height as f32).floor().clamp(0.0, max_y) as usize;
            return self.texel(x, y);
        }
        let x = (u * self.width as f32 - 0.5).clamp(0.0, max_x);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, max_y);
        let x0 = x.floor();
        let y0 = y.floor();
        let fx = x - x0;
        let fy = y - y0;
        let x0 = x0 as usize;
        let y0 = y0 as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let (a, b, c, d) = (
            self.texel(x0, y0),
            self.texel(x1, y0),
            self.texel(x0, y1),
            self.texel(x1, y1),
        );
        let mut out = [0.0f32; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bottom = c[i] + (d[i] - c[i]) * fx;
            out[i] = top + (bottom - top) * fy;
        }
        out
    }
}

fn unit(color: Color32) -> [f32; 4] {
    let [r, g, b, a] = color.to_array();
    [
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        f32::from(a) / 255.0,
    ]
}

/// Texture store fed by egui's `TexturesDelta`, and the triangle rasterizer.
#[derive(Debug, Default)]
pub struct Rasterizer {
    textures: HashMap<TextureId, Texture>,
}

impl Rasterizer {
    /// Applies the `set` part of one frame's texture changes (whole or partial updates); call
    /// [`Rasterizer::free`] after painting that frame.
    ///
    /// # Panics
    ///
    /// On a texture larger than [`MAX_SIDE`] or a partial update outside its texture (test
    /// failure, never silently ignored).
    pub fn apply(&mut self, delta: &TexturesDelta) {
        for (id, image_delta) in &delta.set {
            let ImageData::Color(image) = &image_delta.image;
            let [w, h] = image.size;
            assert!(w <= MAX_SIDE && h <= MAX_SIDE, "texture {w}x{h} too large");
            let texels: Vec<[f32; 4]> = image.pixels.iter().map(|c| unit(*c)).collect();
            let linear = image_delta.options.magnification == TextureFilter::Linear;
            match image_delta.pos {
                None => {
                    self.textures.insert(
                        *id,
                        Texture {
                            width: w,
                            height: h,
                            texels,
                            linear,
                        },
                    );
                }
                Some([px, py]) => {
                    let texture = self
                        .textures
                        .get_mut(id)
                        .unwrap_or_else(|| panic!("partial update of unknown texture {id:?}"));
                    assert!(
                        px + w <= texture.width && py + h <= texture.height,
                        "partial update outside texture {id:?}"
                    );
                    for row in 0..h {
                        for col in 0..w {
                            texture.texels[(py + row) * texture.width + px + col] =
                                texels[row * w + col];
                        }
                    }
                }
            }
        }
    }

    /// Applies the `free` part of one frame's texture changes (after painting that frame).
    pub fn free(&mut self, delta: &TexturesDelta) {
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    /// Renders `primitives` (points) at `pixels_per_point` onto an image of `size_px`, starting
    /// from the opaque `background`.
    ///
    /// # Panics
    ///
    /// On an image larger than [`MAX_SIDE`] or a mesh referencing an unknown texture.
    pub fn render(
        &self,
        primitives: &[ClippedPrimitive],
        pixels_per_point: f32,
        size_px: [u32; 2],
        background: Color32,
    ) -> RgbaImage {
        let [width, height] = size_px;
        let (w, h) = (width as usize, height as usize);
        assert!(w <= MAX_SIDE && h <= MAX_SIDE, "image {w}x{h} too large");
        let bg = unit(background);
        let mut buffer: Vec<[f32; 4]> = vec![[bg[0], bg[1], bg[2], 1.0]; w * h];

        for clipped in primitives {
            let Primitive::Mesh(mesh) = &clipped.primitive else {
                continue; // paint callbacks are not used by soos-gui
            };
            let texture = self
                .textures
                .get(&mesh.texture_id)
                .unwrap_or_else(|| panic!("mesh uses unknown texture {:?}", mesh.texture_id));
            let clip = clipped.clip_rect;
            let clip_min_x = (clip.min.x * pixels_per_point).round().clamp(0.0, w as f32) as usize;
            let clip_min_y = (clip.min.y * pixels_per_point).round().clamp(0.0, h as f32) as usize;
            let clip_max_x = (clip.max.x * pixels_per_point).round().clamp(0.0, w as f32) as usize;
            let clip_max_y = (clip.max.y * pixels_per_point).round().clamp(0.0, h as f32) as usize;
            if clip_min_x >= clip_max_x || clip_min_y >= clip_max_y {
                continue;
            }
            for tri in mesh.indices.as_chunks::<3>().0 {
                let (Some(a), Some(b), Some(c)) = (
                    mesh.vertices.get(tri[0] as usize),
                    mesh.vertices.get(tri[1] as usize),
                    mesh.vertices.get(tri[2] as usize),
                ) else {
                    continue;
                };
                fill_triangle(
                    &mut buffer,
                    w,
                    [clip_min_x, clip_min_y, clip_max_x, clip_max_y],
                    [a, b, c],
                    pixels_per_point,
                    texture,
                );
            }
        }

        let mut pixels = Vec::with_capacity(w * h * 4);
        for px in &buffer {
            for channel in &px[..3] {
                pixels.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            pixels.push(255);
        }
        RgbaImage {
            width,
            height,
            pixels,
        }
    }
}

/// Edge function of `p` against the directed edge `a → b`.
fn edge(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// Top-left rule (y down, positive area): left edges go up, top edges go right.
fn is_top_left(a: [f32; 2], b: [f32; 2]) -> bool {
    let dy = b[1] - a[1];
    let dx = b[0] - a[0];
    dy < 0.0 || (dy == 0.0 && dx > 0.0)
}

fn inside(w: f32, top_left: bool) -> bool {
    w > 0.0 || (w == 0.0 && top_left)
}

fn fill_triangle(
    buffer: &mut [[f32; 4]],
    width: usize,
    clip: [usize; 4],
    vertices: [&Vertex; 3],
    ppp: f32,
    texture: &Texture,
) {
    let mut v = vertices;
    let pos = |vx: &Vertex| [vx.pos.x * ppp, vx.pos.y * ppp];
    let mut p = [pos(v[0]), pos(v[1]), pos(v[2])];
    let mut area = edge(p[0], p[1], p[2]);
    if area == 0.0 || !area.is_finite() {
        return;
    }
    if area < 0.0 {
        v.swap(1, 2);
        p.swap(1, 2);
        area = -area;
    }
    let min_x = p.iter().map(|q| q[0]).fold(f32::INFINITY, f32::min);
    let max_x = p.iter().map(|q| q[0]).fold(f32::NEG_INFINITY, f32::max);
    let min_y = p.iter().map(|q| q[1]).fold(f32::INFINITY, f32::min);
    let max_y = p.iter().map(|q| q[1]).fold(f32::NEG_INFINITY, f32::max);
    let x0 = (min_x - 0.5).floor().max(clip[0] as f32) as usize;
    let y0 = (min_y - 0.5).floor().max(clip[1] as f32) as usize;
    let x1 = ((max_x + 0.5).ceil().max(0.0) as usize).min(clip[2]);
    let y1 = ((max_y + 0.5).ceil().max(0.0) as usize).min(clip[3]);
    let tl = [
        is_top_left(p[1], p[2]),
        is_top_left(p[2], p[0]),
        is_top_left(p[0], p[1]),
    ];
    let colors = [unit(v[0].color), unit(v[1].color), unit(v[2].color)];
    for y in y0..y1 {
        for x in x0..x1 {
            let centre = [x as f32 + 0.5, y as f32 + 0.5];
            let w0 = edge(p[1], p[2], centre);
            let w1 = edge(p[2], p[0], centre);
            let w2 = edge(p[0], p[1], centre);
            if !(inside(w0, tl[0]) && inside(w1, tl[1]) && inside(w2, tl[2])) {
                continue;
            }
            let (l0, l1, l2) = (w0 / area, w1 / area, w2 / area);
            let u = l0 * v[0].uv.x + l1 * v[1].uv.x + l2 * v[2].uv.x;
            let t = l0 * v[0].uv.y + l1 * v[1].uv.y + l2 * v[2].uv.y;
            let texel = texture.sample(u, t);
            let Some(dst) = buffer.get_mut(y * width + x) else {
                continue;
            };
            let mut src = [0.0f32; 4];
            for i in 0..4 {
                let vc = l0 * colors[0][i] + l1 * colors[1][i] + l2 * colors[2][i];
                src[i] = vc * texel[i];
            }
            let keep = 1.0 - src[3];
            for i in 0..3 {
                dst[i] = src[i] + dst[i] * keep;
            }
            dst[3] = 1.0;
        }
    }
}

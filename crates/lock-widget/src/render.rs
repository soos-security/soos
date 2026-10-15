//! Paints a [`FrameSpec`] with a [`Layout`] onto a [`Canvas`].

use crate::layout::{Layout, Rect};
use crate::palette::{BACKGROUND, LABEL, STAR};
use crate::raster::Canvas;
use crate::scene::FrameSpec;

/// Fills the whole canvas with the background, then paints the widget.
pub fn paint_full(canvas: &mut Canvas<'_>, layout: &Layout, frame: &FrameSpec) {
    let all = Rect {
        x: 0,
        y: 0,
        w: canvas.width(),
        h: canvas.height(),
    };
    canvas.fill_rect(all, BACKGROUND);
    paint_widget(canvas, layout, frame);
}

/// Repaints only `layout.widget`: background, ring, star and label shifted by the shake offset.
/// No pixel outside `layout.widget` is written.
pub fn paint_widget(canvas: &mut Canvas<'_>, layout: &Layout, frame: &FrameSpec) {
    let clip = layout.widget;
    canvas.fill_rect(clip, BACKGROUND);
    let cx = layout.cx + frame.offset_x;
    canvas.ring(
        cx,
        layout.cy,
        layout.radius,
        layout.stroke,
        frame.ring,
        clip,
    );
    canvas.star(
        cx,
        layout.cy,
        layout.star_half,
        frame.star_angle_deg,
        STAR,
        clip,
    );
    canvas.text(
        cx,
        layout.label_top,
        layout.label_scale,
        frame.label,
        LABEL,
        clip,
    );
}

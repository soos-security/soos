//! Face overlay of the live video (detections, PAD crop area, landmarks).
//!
//! Truthful by construction (`Docs/UI_MOTION.md` §7 G9): every box, landmark and liveness colour
//! is drawn from the frame being painted, on that same frame. Nothing here is animated or
//! interpolated, so a verdict that flips from live to spoof shows `FACE_SPOOF` at once, at the
//! latest detection.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    reason = "Video coordinate mapping on bounded f32 values"
)]

use eframe::egui::{self, Color32, Pos2, Rect, Stroke};

use crate::state::LatestFrameData;
use crate::theme;
use crate::widgets;

/// Which overlay layers are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayToggles {
    /// Face boxes with their confidence badge.
    pub bbox: bool,
    /// The five landmarks and the eye axis.
    pub landmarks: bool,
    /// The 2.7× PAD context crop area.
    pub pad_crop: bool,
}

/// Face box colour of the pipeline liveness decision (GitHub #215).
pub fn face_box_color(pad_live: bool) -> Color32 {
    if pad_live {
        theme::FACE_LIVE
    } else {
        theme::FACE_SPOOF
    }
}

/// Maps frame pixel `(x, y)` of a `frame_w` × `frame_h` frame to the screen `video` rectangle.
pub fn frame_to_screen(video: Rect, frame_w: u32, frame_h: u32, x: f32, y: f32) -> Pos2 {
    Pos2::new(
        video.min.x + (x / frame_w as f32) * video.width(),
        video.min.y + (y / frame_h as f32) * video.height(),
    )
}

/// Paints the detections of `frame` over the `video` rectangle with `painter`.
pub fn paint_detections(
    painter: &egui::Painter,
    video: Rect,
    frame: &LatestFrameData,
    toggles: OverlayToggles,
) {
    let to_screen = |x: f32, y: f32| frame_to_screen(video, frame.width, frame.height, x, y);
    for det in &frame.detections {
        // Liveness color comes only from the pipeline decision (GitHub #215).
        let box_color = face_box_color(frame.pad_live);

        // PAD 2.7x expanded bounding box context crop (drawn under the face box)
        if toggles.pad_crop {
            let pad_box =
                soos_vision::crop::expand_bbox_for_pad(&det.box_, 2.7, frame.width, frame.height);
            let pad_p1 = to_screen(pad_box.x1, pad_box.y1);
            let pad_p2 = to_screen(pad_box.x2, pad_box.y2);
            painter.rect_stroke(
                Rect::from_two_pos(pad_p1, pad_p2),
                2.0,
                Stroke::new(1.0, theme::PAD_BOX),
                egui::StrokeKind::Outside,
            );
        }

        if toggles.bbox {
            let p1 = to_screen(det.box_.x1, det.box_.y1);
            let p2 = to_screen(det.box_.x2, det.box_.y2);
            painter.rect_stroke(
                Rect::from_two_pos(p1, p2),
                3.0,
                Stroke::new(2.0, box_color),
                egui::StrokeKind::Outside,
            );

            // Confidence badge
            let label = format!("Face: {:.1}%", det.score * 100.0);
            widgets::backed_text(
                painter,
                Pos2::new(p1.x + 7.0, p1.y + 5.0),
                egui::Align2::LEFT_TOP,
                &label,
                egui::FontId::proportional(theme::F_SMALL),
                box_color,
            );
        }

        // Authentic 5-point landmarks
        if toggles.landmarks {
            if let Some(lmk) = &det.landmarks {
                // Eye axis first so the dots sit on top of it.
                painter.line_segment(
                    [
                        to_screen(lmk.left_eye.x, lmk.left_eye.y),
                        to_screen(lmk.right_eye.x, lmk.right_eye.y),
                    ],
                    Stroke::new(1.5, theme::EYE_AXIS),
                );
                let pts = [
                    (lmk.left_eye, theme::EYE),
                    (lmk.right_eye, theme::EYE),
                    (lmk.nose, theme::NOSE),
                    (lmk.mouth_left, theme::MOUTH),
                    (lmk.mouth_right, theme::MOUTH),
                ];
                for (pt, color) in pts {
                    let center = to_screen(pt.x, pt.y);
                    painter.circle_filled(center, 4.5, color);
                    painter.circle_stroke(
                        center,
                        5.0,
                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(16, 24, 32, 200)),
                    );
                }
            }
        }
    }
}

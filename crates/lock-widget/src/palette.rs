//! Brand colours mirrored by name and value from `crates/gui/src/theme.rs` (ADR 2026-10-06
//! "soos Brand Direction", item 1) and the widget's colour roles. Flat colours only.

use soos_lockstatus::view::Presented;

/// One opaque sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// `theme::BLUE`.
pub const BLUE: Rgb = Rgb(0x00, 0x47, 0xBB);
/// `theme::PALE`.
pub const PALE: Rgb = Rgb(0xED, 0xF1, 0xFF);
/// `theme::PALE_4`.
pub const PALE_4: Rgb = Rgb(0xC9, 0xD6, 0xFA);
/// `theme::PINK`.
pub const PINK: Rgb = Rgb(0xE5, 0x9B, 0xDC);
/// `theme::WARN_BORDER`.
pub const WARN_BORDER: Rgb = Rgb(0xF5, 0xB5, 0x47);

/// Surface background.
pub const BACKGROUND: Rgb = BLUE;
/// Ring while idle.
pub const RING_IDLE: Rgb = PALE_4;
/// Ring while scanning.
pub const RING_SCANNING: Rgb = PALE;
/// Ring after a failed scan.
pub const RING_NOT_RECOGNIZED: Rgb = WARN_BORDER;
/// The star mark.
pub const STAR: Rgb = PINK;
/// Label text.
pub const LABEL: Rgb = PALE;

/// Linear mix `a·(1 − t) + b·t` per channel, rounded; `t` clamped to `[0, 1]`, non-finite = 0.
#[must_use]
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let channel = |x: u8, y: u8| -> u8 {
        let value = (f32::from(x) * (1.0 - t) + f32::from(y) * t).round();
        to_u8(value)
    };
    Rgb(channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2))
}

/// Saturating conversion of a finite channel value (`NaN` → 0).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to [0, 255] first; `as` saturates and maps NaN to 0"
)]
pub(crate) fn to_u8(value: f32) -> u8 {
    value.clamp(0.0, 255.0) as u8
}

/// Ring colour of a presented state.
#[must_use]
pub fn ring_color(presented: Presented) -> Rgb {
    match presented {
        Presented::Idle => RING_IDLE,
        Presented::Scanning => RING_SCANNING,
        Presented::NotRecognized => RING_NOT_RECOGNIZED,
    }
}

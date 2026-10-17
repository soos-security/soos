//! Output renderings of a [`LockView`] for the screen lockers.
//!
//! Every string is Pango-markup safe (no `<`, `>` or `&`): hyprlock renders label text as Pango
//! markup.

use crate::motion::{MotionPrefs, T_PENDING_MS};
use crate::view::{LockView, Presented};

/// `--once` output formats (JSON exists only in `--watch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Words only (empty while idle).
    Text,
    /// One glyph.
    Glyph,
    /// Glyph and words.
    Label,
}

/// Words while scanning.
pub const SCANNING_TEXT: &str = "Scanning";
/// Words after a failed scan.
pub const NOT_RECOGNIZED_TEXT: &str = "Not recognized";
/// Words while idle (also every transport error): none.
pub const IDLE_TEXT: &str = "";
/// Four-pointed star (brand mark).
pub const IDLE_GLYPH: &str = "\u{2726}";
/// Quarter arcs stepped one per `T_PENDING_MS` while scanning.
pub const SCANNING_GLYPHS: [&str; 4] = ["\u{25DC}", "\u{25DD}", "\u{25DE}", "\u{25DF}"];
/// Static scanning glyph under reduced motion.
pub const SCANNING_STATIC_GLYPH: &str = "\u{25CC}";
/// Glyph after a failed scan.
pub const NOT_RECOGNIZED_GLYPH: &str = "\u{2715}";

/// Words of a presented state.
#[must_use]
pub fn text(presented: Presented) -> &'static str {
    match presented {
        Presented::Idle => IDLE_TEXT,
        Presented::Scanning => SCANNING_TEXT,
        Presented::NotRecognized => NOT_RECOGNIZED_TEXT,
    }
}

/// Glyph of a presented state; `now_ms` is `CLOCK_MONOTONIC` in milliseconds.
#[must_use]
pub fn glyph(presented: Presented, now_ms: u64, prefs: MotionPrefs) -> &'static str {
    match presented {
        Presented::Idle => IDLE_GLYPH,
        Presented::NotRecognized => NOT_RECOGNIZED_GLYPH,
        Presented::Scanning if prefs.reduced => SCANNING_STATIC_GLYPH,
        Presented::Scanning => {
            let quarters = now_ms / T_PENDING_MS;
            let index = usize::try_from(quarters % 4).unwrap_or(0);
            SCANNING_GLYPHS
                .get(index)
                .copied()
                .unwrap_or(SCANNING_STATIC_GLYPH)
        }
    }
}

/// The glyph alone when the words are empty, else the glyph, two spaces and the words.
#[must_use]
pub fn label(presented: Presented, now_ms: u64, prefs: MotionPrefs) -> String {
    let mark = glyph(presented, now_ms, prefs);
    let words = text(presented);
    if words.is_empty() {
        mark.to_string()
    } else {
        format!("{mark}  {words}")
    }
}

/// One `--watch` JSON object (no newline): the presented state, whether the daemon answered and
/// the transition counter.
#[must_use]
pub fn json(view: &LockView) -> String {
    let state = match view.presented() {
        Presented::Idle => "idle",
        Presented::Scanning => "scanning",
        Presented::NotRecognized => "not_recognized",
    };
    format!(
        "{{\"state\":\"{state}\",\"available\":{},\"sequence\":{}}}",
        view.available(),
        view.sequence
    )
}

/// The exact `--once` stdout bytes (no trailing newline).
#[must_use]
pub fn once_output(
    format: OutputFormat,
    view: &LockView,
    now_ms: u64,
    prefs: MotionPrefs,
) -> String {
    let presented = view.presented();
    match format {
        OutputFormat::Text => text(presented).to_string(),
        OutputFormat::Glyph => glyph(presented, now_ms, prefs).to_string(),
        OutputFormat::Label => label(presented, now_ms, prefs),
    }
}

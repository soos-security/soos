//! Lock-screen motion tokens and reduced-motion resolution.
//!
//! Mirrors the normative tables of `Docs/UI_MOTION.md` §2 and §9 (ADR 2026-10-13 "Motion System
//! for soos-gui and soos-remote") under the same names with an `_MS` suffix: the lock-screen
//! surfaces count integer milliseconds. Checked by `soos-invariants::lockscreen_adapters_contract`.

use std::ffi::OsStr;

/// `--t-short`: ring colour cross-fade.
pub const T_SHORT_MS: u64 = 180;
/// `--t-medium`: the single Not recognized shake.
pub const T_MEDIUM_MS: u64 = 260;
/// `--t-pending`: one quarter turn of the pending star.
pub const T_PENDING_MS: u64 = 1200;
/// `AMBIENT_REPAINT_MS`: ambient loop cap (at most 30 Hz).
pub const AMBIENT_REPAINT_MS: u64 = 33;
/// `--slide`: shake amplitude in pixels.
pub const SLIDE_PX: f32 = 8.0;
/// `--ease-standard`: `cubic-bezier(0.2, 0, 0, 1)`.
pub const EASE_STANDARD: [f32; 4] = [0.2, 0.0, 0.0, 1.0];
/// Longest continuous pending loop of one scan: four quarter turns (a multiple of the star's
/// 180° symmetry), below the 5 s limit of WCAG 2.2.2 (`Docs/UI_MOTION.md` §5).
pub const PENDING_LOOP_MAX_MS: u64 = 4800;
/// Environment override defined for the soos frontends by `Docs/UI_MOTION.md` §8: `1` reduces
/// motion, `0` forces full motion.
pub const REDUCE_MOTION_ENV: &str = "SOOS_REDUCE_MOTION";
/// Longer values of [`REDUCE_MOTION_ENV`] are ignored.
pub const MAX_REDUCE_MOTION_ENV_BYTES: usize = 8;

/// Resolved motion preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MotionPrefs {
    /// Reduced motion: no rotation, no shake, instant colour changes.
    pub reduced: bool,
}

/// `--reduce-motion` first, then [`REDUCE_MOTION_ENV`] (`1` reduced, `0` full, anything else
/// ignored), then full motion.
#[must_use]
pub fn resolve_motion_prefs(flag: bool, env: Option<&OsStr>) -> MotionPrefs {
    if flag {
        return MotionPrefs { reduced: true };
    }
    let from_env = env
        .filter(|value| value.len() <= MAX_REDUCE_MOTION_ENV_BYTES)
        .and_then(OsStr::to_str)
        .and_then(|value| match value {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        });
    MotionPrefs {
        reduced: from_env.unwrap_or(false),
    }
}

/// Bisection steps of [`cubic_bezier`] (the error after 32 halvings is below `f32` precision).
const BEZIER_BISECTION_STEPS: u32 = 32;

/// Cubic Bézier easing `y(x)` for control points `(x1, y1, x2, y2)`; `t` is clamped to `[0, 1]`
/// and a non-finite `t` counts as 0. Always finite; `f(0) = 0`, `f(1) = 1`.
#[must_use]
pub fn cubic_bezier(points: [f32; 4], t: f32) -> f32 {
    let [x1, y1, x2, y2] = points;
    let x = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let curve = |a: f32, b: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    // x(s) is monotonic on [0, 1] for control abscissas in [0, 1]: bisection always converges.
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for _ in 0..BEZIER_BISECTION_STEPS {
        let mid = 0.5 * (low + high);
        if curve(x1, x2, mid) < x {
            low = mid;
        } else {
            high = mid;
        }
    }
    let y = curve(y1, y2, 0.5 * (low + high));
    if y.is_finite() {
        y
    } else {
        0.0
    }
}

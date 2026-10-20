//! Motion helpers of `soos-gui` (`Docs/UI_MOTION.md` §3, §7–§9; ADR 2026-10-13 "Motion System
//! for soos-gui and soos-remote"; GitHub #438 = PR 3 of #358).
//!
//! The pure part (spring, easing, entrance and pending-star maths, reduced-motion resolution)
//! takes and returns plain numbers: no allocation, no I/O, finite output on any input. The egui
//! part keeps one small `Copy` state per animated widget in egui temp storage, mutated in place
//! and stepped at most once per egui pass. Motion code requests a repaint only while something
//! moves (`request_repaint`) or, for the pending star, at most every `AMBIENT_REPAINT_MS`
//! (`request_repaint_after`); with reduced motion every helper returns its target at once and
//! requests nothing. Animation always follows the state the caller passes; it never delays or
//! hides it.

#![forbid(unsafe_code)]
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Motion code runs every frame on the UI thread and must never panic (UI_MOTION.md §9)"
)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    reason = "Animation maths on finite, clamped f32/f64 values"
)]

pub mod portal;

use std::ffi::OsStr;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Id, Ui};

use crate::theme;
use portal::PortalReader;

// ---------------------------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------------------------

/// Sanitizes a frame time: `NaN`, negative and `-inf` become 0, `+inf` and anything longer than
/// [`theme::MAX_MOTION_DT`] become `MAX_MOTION_DT`.
pub fn clamp_dt(dt: f32) -> f32 {
    if dt.is_nan() || dt <= 0.0 {
        0.0
    } else {
        dt.min(theme::MAX_MOTION_DT)
    }
}

/// Spring constants derived from the theme: `(zeta * omega_n, omega_n^2, omega_d)`.
fn spring_constants() -> (f64, f64, f64) {
    let k = f64::from(theme::SPRING_STIFFNESS);
    let c = f64::from(theme::SPRING_DAMPING);
    let m = f64::from(theme::SPRING_MASS);
    let wn = (k / m).sqrt();
    let zeta = c / (2.0 * (k * m).sqrt());
    let wd = wn * (1.0 - zeta * zeta).max(0.0).sqrt();
    (zeta * wn, wn * wn, wd)
}

/// Damped spring advancing `(value, velocity)` toward `target` with the closed-form solution of
/// the oscillator of `Docs/UI_MOTION.md` §3 (exact for any `dt`, no integration drift).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringState {
    value: f32,
    velocity: f32,
    target: f32,
    span: f32,
    elapsed: f32,
}

impl SpringState {
    /// A spring at rest on `value` (a non-finite value becomes 0).
    pub fn new(value: f32) -> Self {
        let value = if value.is_finite() { value } else { 0.0 };
        Self {
            value,
            velocity: 0.0,
            target: value,
            span: 0.0,
            elapsed: 0.0,
        }
    }

    /// Current value.
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Current velocity (units per second).
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Current target.
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Moves the target. A non-finite target is ignored and the target the spring already has
    /// is a no-op; otherwise the velocity is kept (a reversal is smooth) and the span restarts.
    pub fn retarget(&mut self, target: f32) {
        if !target.is_finite() || target == self.target {
            return;
        }
        self.target = target;
        self.span = (target - self.value).abs();
        self.elapsed = 0.0;
        if !self.span.is_finite() {
            self.snap();
        }
    }

    /// Settled: zero span, or within `SPRING_SETTLE_EPSILON` of the span from the target and
    /// below `SPRING_SETTLE_VELOCITY` span per second.
    pub fn settled(&self) -> bool {
        if self.span <= 0.0 {
            return true;
        }
        (self.target - self.value).abs() <= theme::SPRING_SETTLE_EPSILON * self.span
            && self.velocity.abs() <= theme::SPRING_SETTLE_VELOCITY * self.span
    }

    /// Puts the spring at rest exactly on its target.
    pub fn snap(&mut self) {
        self.value = self.target;
        self.velocity = 0.0;
        self.span = 0.0;
        self.elapsed = 0.0;
    }

    /// Advances by `clamp_dt(dt)` seconds and returns the new value. A settled spring snaps
    /// exactly onto its target; a non-finite intermediate snaps too.
    pub fn step(&mut self, dt: f32) -> f32 {
        let dt = clamp_dt(dt);
        if self.settled() {
            self.snap();
            return self.value;
        }
        if dt <= 0.0 {
            return self.value;
        }
        let (a, wn2, wd) = spring_constants();
        let t = f64::from(dt);
        let x0 = f64::from(self.value) - f64::from(self.target);
        let v0 = f64::from(self.velocity);
        let decay = (-a * t).exp();
        let (sin, cos) = (wd * t).sin_cos();
        let (x, v) = if wd > 0.0 {
            (
                decay * (x0 * cos + (v0 + a * x0) / wd * sin),
                decay * (v0 * cos - (wn2 * x0 + a * v0) / wd * sin),
            )
        } else {
            (0.0, 0.0)
        };
        #[allow(
            clippy::cast_possible_truncation,
            reason = "Finite values within the span of a GUI animation"
        )]
        let (value, velocity) = ((f64::from(self.target) + x) as f32, v as f32);
        self.elapsed += dt;
        if !value.is_finite() || !velocity.is_finite() || self.elapsed >= 4.0 * theme::T_SPRING {
            self.snap();
        } else {
            self.value = value;
            self.velocity = velocity;
            if self.settled() {
                self.snap();
            }
        }
        self.value
    }
}

/// Cubic-bezier easing `points` = `[x1, y1, x2, y2]` evaluated at `t`: `NaN` is 0, `t` is
/// clamped to [0, 1] and both endpoints are exact.
pub fn cubic_bezier(points: [f32; 4], t: f32) -> f32 {
    if t.is_nan() || t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let [x1, y1, x2, y2] = points.map(f64::from);
    let curve = |p1: f64, p2: f64, s: f64| {
        let u = 1.0 - s;
        3.0 * p1 * s * u * u + 3.0 * p2 * s * s * u + s * s * s
    };
    let slope = |p1: f64, p2: f64, s: f64| {
        let u = 1.0 - s;
        3.0 * p1 * u * u + 6.0 * (p2 - p1) * s * u + 3.0 * (1.0 - p2) * s * s
    };
    let target = f64::from(t);
    let mut s = target;
    let mut solved = false;
    for _ in 0..8 {
        let err = curve(x1, x2, s) - target;
        if err.abs() < 1e-7 {
            solved = true;
            break;
        }
        let d = slope(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - err / d).clamp(0.0, 1.0);
    }
    if !solved {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if curve(x1, x2, mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        s = (lo + hi) / 2.0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Easing output of finite control points, close to [0, 1]"
    )]
    let y = curve(y1, y2, s) as f32;
    if y.is_finite() {
        y
    } else {
        t
    }
}

/// Time-based easing from the shown value toward a target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tween {
    from: f32,
    to: f32,
    elapsed: f32,
    duration: f32,
    easing: [f32; 4],
}

impl Tween {
    /// A tween at rest on `value` (a non-finite value becomes 0).
    pub fn new(value: f32) -> Self {
        let value = if value.is_finite() { value } else { 0.0 };
        Self {
            from: value,
            to: value,
            elapsed: 0.0,
            duration: 0.0,
            easing: theme::EASE_STANDARD,
        }
    }

    /// Current target.
    pub fn target(&self) -> f32 {
        self.to
    }

    /// Shown value.
    pub fn value(&self) -> f32 {
        if self.settled() {
            return self.to;
        }
        let p = cubic_bezier(self.easing, self.elapsed / self.duration);
        self.from + (self.to - self.from) * p
    }

    /// Whether the tween reached its target.
    pub fn settled(&self) -> bool {
        self.duration <= 0.0 || self.elapsed >= self.duration
    }

    /// Eases from the shown value to `target` over `duration` seconds with `easing`. A
    /// non-finite target is ignored and an unchanged target is a no-op; a zero or non-finite
    /// duration jumps to the target.
    pub fn retarget(&mut self, target: f32, duration: f32, easing: [f32; 4]) {
        if !target.is_finite() || target == self.to {
            return;
        }
        self.from = self.value();
        self.to = target;
        self.elapsed = 0.0;
        self.easing = easing;
        self.duration = if duration.is_finite() && duration > 0.0 {
            duration
        } else {
            0.0
        };
    }

    /// Advances by `clamp_dt(dt)` and returns the shown value.
    pub fn step(&mut self, dt: f32) -> f32 {
        if !self.settled() {
            self.elapsed += clamp_dt(dt);
        }
        if self.settled() {
            self.from = self.to;
        }
        self.value()
    }

    /// Jumps to the target.
    pub fn snap(&mut self) {
        self.from = self.to;
        self.elapsed = self.duration;
    }
}

/// Mixes `a` toward `b` by `t`: exactly `a` for `t ≤ 0` (and `NaN`), exactly `b` for `t ≥ 1`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Channel values are clamped to [0, 255] before the cast"
)]
pub fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    if t.is_nan() || t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let mix = |x: u8, y: u8| -> u8 {
        let (x, y) = (f32::from(x), f32::from(y));
        (x + (y - x) * t).round().clamp(0.0, 255.0) as u8
    };
    let [ar, ag, ab, aa] = a.to_array();
    let [br, bg, bb, ba] = b.to_array();
    Color32::from_rgba_premultiplied(mix(ar, br), mix(ag, bg), mix(ab, bb), mix(aa, ba))
}

/// Opacity and vertical offset of an entering element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entrance {
    /// Opacity multiplier in [0, 1].
    pub opacity: f32,
    /// Downward offset in points (`RISE_PX` → 0).
    pub offset_y: f32,
}

impl Entrance {
    /// Fully entered.
    pub const REST: Self = Self {
        opacity: 1.0,
        offset_y: 0.0,
    };
}

/// Delay of staggered item `index`: `T_STAGGER` per item, items beyond the sixth with the sixth.
pub fn stagger_delay(index: usize) -> f32 {
    let capped = index.min(theme::STAGGER_MAX_ITEMS.saturating_sub(1));
    #[allow(
        clippy::cast_precision_loss,
        reason = "At most STAGGER_MAX_ITEMS - 1 = 5"
    )]
    let n = capped as f32;
    n * theme::T_STAGGER
}

/// Total time an entrance of item `index` takes.
fn entrance_duration(index: usize) -> f32 {
    stagger_delay(index) + theme::T_MEDIUM
}

/// Entrance of staggered item `index`, `elapsed` seconds after it appeared: after its delay it
/// fades in and rises `RISE_PX` over `T_MEDIUM` with `EASE_ENTER`.
pub fn entrance_at(elapsed: f32, index: usize) -> Entrance {
    let elapsed = if elapsed.is_finite() { elapsed } else { 0.0 };
    let p = ((elapsed - stagger_delay(index)) / theme::T_MEDIUM).clamp(0.0, 1.0);
    if p >= 1.0 {
        return Entrance::REST;
    }
    let e = cubic_bezier(theme::EASE_ENTER, p);
    Entrance {
        opacity: e,
        offset_y: theme::RISE_PX * (1.0 - e),
    }
}

/// Rotation of the pending star `time_s` seconds into its loop, in radians in [0, π): a quarter
/// turn per `T_PENDING` with `EASE_STANDARD`, modulo a half turn (the mark repeats every half
/// turn). Non-finite or negative times are 0.
pub fn pending_star_angle(time_s: f64) -> f32 {
    if !time_s.is_finite() || time_s <= 0.0 {
        return 0.0;
    }
    let period = f64::from(theme::T_PENDING);
    let turns = time_s / period;
    let quarter = turns.floor();
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Fraction of a period in [0, 1)"
    )]
    let frac = (turns - quarter) as f32;
    let quarters = (quarter % 2.0) + f64::from(cubic_bezier(theme::EASE_STANDARD, frac));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "At most two quarter turns, in radians"
    )]
    let angle = (quarters * std::f64::consts::FRAC_PI_2) as f32;
    if angle.is_finite() && (0.0..std::f32::consts::PI).contains(&angle) {
        angle
    } else {
        0.0
    }
}

/// Resolved motion preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionPrefs {
    /// Every helper returns its target at once; no motion repaint.
    pub reduced: bool,
}

impl MotionPrefs {
    /// Full motion.
    pub const FULL: Self = Self { reduced: false };
    /// Reduced motion.
    pub const REDUCED: Self = Self { reduced: true };
}

/// Answer of the XDG desktop portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalPreference {
    /// The desktop asks for reduced motion.
    Reduce,
    /// The desktop states no preference.
    NoPreference,
    /// No portal, an error, an invalid value, the deadline or a dead reader.
    Unavailable,
    /// Still waiting (at most the deadline): treated as reduced.
    Pending,
}

/// Environment variable overriding the desktop preference (`1` reduced, `0` full motion).
pub const REDUCE_MOTION_ENV: &str = "SOOS_REDUCE_MOTION";

/// Longest accepted value of [`REDUCE_MOTION_ENV`]; longer values are ignored.
pub const MAX_REDUCE_MOTION_ENV_BYTES: usize = 8;

/// `Some(true)` for exactly `1`, `Some(false)` for exactly `0`, `None` otherwise (absent, other
/// values, more than [`MAX_REDUCE_MOTION_ENV_BYTES`] bytes).
pub fn parse_reduce_motion_env(value: Option<&OsStr>) -> Option<bool> {
    let bytes = value?.as_encoded_bytes();
    if bytes.len() > MAX_REDUCE_MOTION_ENV_BYTES {
        return None;
    }
    match bytes {
        b"1" => Some(true),
        b"0" => Some(false),
        _ => None,
    }
}

/// Resolves the preference: `--reduce-motion`, then [`REDUCE_MOTION_ENV`], then the portal
/// (`Pending` counts as reduced), then full motion (`Docs/UI_MOTION.md` §8).
pub fn resolve_motion_prefs(
    cli_reduce: bool,
    env: Option<&OsStr>,
    portal: PortalPreference,
) -> MotionPrefs {
    if cli_reduce {
        return MotionPrefs::REDUCED;
    }
    if let Some(reduced) = parse_reduce_motion_env(env) {
        return MotionPrefs { reduced };
    }
    match portal {
        PortalPreference::Reduce | PortalPreference::Pending => MotionPrefs::REDUCED,
        PortalPreference::NoPreference | PortalPreference::Unavailable => MotionPrefs::FULL,
    }
}

// ---------------------------------------------------------------------------------------------
// egui glue
// ---------------------------------------------------------------------------------------------

fn prefs_id() -> Id {
    Id::new("soos_motion_prefs")
}

/// Stores `prefs` in `ctx` and sets `Style::animation_time` (built-in window and popup fades)
/// to `T_SHORT`, or 0 when reduced.
pub fn install_prefs(ctx: &egui::Context, prefs: MotionPrefs) {
    ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(prefs_id(), || prefs) = prefs);
    let time = if prefs.reduced { 0.0 } else { theme::T_SHORT };
    ctx.all_styles_mut(|style| style.animation_time = time);
}

/// The installed preference; reduced when none was installed (a pending portal).
pub fn prefs(ctx: &egui::Context) -> MotionPrefs {
    ctx.data(|d| d.get_temp::<MotionPrefs>(prefs_id()))
        .unwrap_or(MotionPrefs::REDUCED)
}

/// The current pass number and the sanitized frame time.
fn frame_clock(ctx: &egui::Context) -> (u64, f32) {
    (
        ctx.cumulative_pass_nr(),
        clamp_dt(ctx.input(|i| i.stable_dt)),
    )
}

#[derive(Debug, Clone, Copy)]
struct SpringSlot {
    spring: SpringState,
    pass: u64,
}

/// A spring value for widget `id` moving toward `target` (first sight at rest on it).
pub fn spring_value(ctx: &egui::Context, id: Id, target: f32) -> f32 {
    let reduced = prefs(ctx).reduced;
    let (pass, dt) = frame_clock(ctx);
    let (value, moving) = ctx.data_mut(|d| {
        let slot = d.get_temp_mut_or_insert_with(id, || SpringSlot {
            spring: SpringState::new(target),
            pass,
        });
        slot.spring.retarget(target);
        if reduced {
            slot.spring.snap();
        } else if slot.pass != pass {
            slot.spring.step(dt);
        }
        slot.pass = pass;
        (slot.spring.value(), !slot.spring.settled())
    });
    if moving {
        ctx.request_repaint();
    }
    value
}

/// The current value of the spring of widget `id`, without stepping it (`None` before its
/// first use).
pub fn spring_peek(ctx: &egui::Context, id: Id) -> Option<f32> {
    ctx.data(|d| d.get_temp::<SpringSlot>(id))
        .map(|slot| slot.spring.value())
}

#[derive(Debug, Clone, Copy)]
struct TweenSlot {
    tween: Tween,
    pass: u64,
}

/// A tweened value for widget `id` easing toward `target` over `duration` with `easing`
/// (first sight at rest on it).
pub fn tween_value(
    ctx: &egui::Context,
    id: Id,
    target: f32,
    duration: f32,
    easing: [f32; 4],
) -> f32 {
    let reduced = prefs(ctx).reduced;
    let (pass, dt) = frame_clock(ctx);
    let (value, moving) = ctx.data_mut(|d| {
        let slot = d.get_temp_mut_or_insert_with(id, || TweenSlot {
            tween: Tween::new(target),
            pass,
        });
        let changed = slot.tween.target() != target;
        slot.tween.retarget(target, duration, easing);
        if reduced {
            slot.tween.snap();
        } else if slot.pass != pass || changed {
            slot.tween.step(dt);
        }
        slot.pass = pass;
        (slot.tween.value(), !slot.tween.settled())
    });
    if moving {
        ctx.request_repaint();
    }
    value
}

#[derive(Debug, Clone, Copy)]
struct EntranceSlot {
    elapsed: f32,
    pass: u64,
}

/// Entrance of staggered item `index` with widget id `id`. An element that was not drawn in
/// the previous pass (first sight included) enters again; one drawn every pass never does.
pub fn entrance(ctx: &egui::Context, id: Id, index: usize) -> Entrance {
    let reduced = prefs(ctx).reduced;
    let (pass, dt) = frame_clock(ctx);
    let done = entrance_duration(index);
    let elapsed = ctx.data_mut(|d| {
        let slot = d.get_temp_mut_or_insert_with(id, || EntranceSlot { elapsed: 0.0, pass });
        if reduced {
            slot.elapsed = done;
        } else if slot.pass.saturating_add(1) < pass {
            slot.elapsed = 0.0;
        } else if slot.pass != pass {
            slot.elapsed = (slot.elapsed + dt).min(done);
        }
        slot.pass = pass;
        slot.elapsed
    });
    let e = entrance_at(elapsed, index);
    if e != Entrance::REST {
        ctx.request_repaint();
    }
    e
}

#[derive(Debug, Clone, Copy)]
struct FadeSlot {
    key: u64,
    elapsed: f32,
    duration: f32,
    pass: u64,
}

/// Opacity of a value that fades in over `duration` (`EASE_ENTER`) whenever `key` changes;
/// first sight and reduced motion are fully opaque.
pub fn fade_in_on_change(ctx: &egui::Context, id: Id, key: u64, duration: f32) -> f32 {
    let reduced = prefs(ctx).reduced;
    let (pass, dt) = frame_clock(ctx);
    let duration = if duration.is_finite() && duration > 0.0 {
        duration
    } else {
        0.0
    };
    let (elapsed, duration) = ctx.data_mut(|d| {
        let slot = d.get_temp_mut_or_insert_with(id, || FadeSlot {
            key,
            elapsed: duration,
            duration,
            pass,
        });
        if slot.key != key {
            slot.key = key;
            slot.elapsed = 0.0;
            slot.duration = duration;
        } else if slot.pass != pass {
            slot.elapsed = (slot.elapsed + dt).min(slot.duration);
        }
        if reduced {
            slot.elapsed = slot.duration;
        }
        slot.pass = pass;
        (slot.elapsed, slot.duration)
    });
    if duration <= 0.0 || elapsed >= duration {
        return 1.0;
    }
    ctx.request_repaint();
    cubic_bezier(theme::EASE_ENTER, elapsed / duration)
}

#[derive(Debug, Clone, Copy)]
struct StableFadeSlot {
    key: u64,
    elapsed: f32,
    duration: f32,
    /// Time the current key has been on screen, capped at the stable hold.
    held: f32,
    pass: u64,
}

/// Like [`fade_in_on_change`], for content that may flicker at camera rate (a stat tile whose
/// caption follows face detection, `Docs/UI_MOTION.md` G4): a change fades in only when the
/// previous key had been on screen for at least twice `duration`. A change during a running fade
/// keeps that fade running (never restarts it) and a change sooner after a fade is drawn at once,
/// so a flickering value never falls back to faint: it is fully opaque at most `duration` after
/// the first change and then requests no repaint. First sight and reduced motion are opaque.
pub fn fade_in_on_stable_change(ctx: &egui::Context, id: Id, key: u64, duration: f32) -> f32 {
    let reduced = prefs(ctx).reduced;
    let (pass, dt) = frame_clock(ctx);
    let duration = if duration.is_finite() && duration > 0.0 {
        duration
    } else {
        0.0
    };
    let hold = 2.0 * duration;
    let (elapsed, duration) = ctx.data_mut(|d| {
        let slot = d.get_temp_mut_or_insert_with(id, || StableFadeSlot {
            key,
            elapsed: duration,
            duration,
            held: hold,
            pass,
        });
        if slot.pass != pass {
            slot.elapsed = (slot.elapsed + dt).min(slot.duration);
            slot.held = (slot.held + dt).min(hold);
        }
        if slot.key != key {
            if slot.held >= hold {
                slot.elapsed = 0.0;
                slot.duration = duration;
            }
            slot.key = key;
            slot.held = 0.0;
        }
        if reduced {
            slot.elapsed = slot.duration;
        }
        slot.pass = pass;
        (slot.elapsed, slot.duration)
    });
    if duration <= 0.0 || elapsed >= duration {
        return 1.0;
    }
    ctx.request_repaint();
    cubic_bezier(theme::EASE_ENTER, elapsed / duration)
}

/// Rotation of the pending star, in radians: 0 with reduced motion (a static star, no
/// repaint), otherwise [`pending_star_angle`] of the input time and one repaint request after
/// `AMBIENT_REPAINT_MS` (never an immediate repaint).
pub fn pending_star_rotation(ctx: &egui::Context) -> f32 {
    if prefs(ctx).reduced {
        return 0.0;
    }
    ctx.request_repaint_after(Duration::from_millis(theme::AMBIENT_REPAINT_MS));
    pending_star_angle(ctx.input(|i| i.time))
}

/// Multiplies the opacity of `ui` by `opacity` (clamped to [0, 1]; non-finite is 1) and returns
/// the previous opacity, to restore with `ui.set_opacity`. An opacity of 1 changes nothing.
pub fn push_opacity(ui: &mut Ui, opacity: f32) -> f32 {
    let previous = ui.opacity();
    let factor = if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if factor < 1.0 {
        ui.multiply_opacity(factor);
    }
    previous
}

/// Runs `add` with the opacity of `ui` multiplied by `opacity`, then restores it.
pub fn with_opacity<R>(ui: &mut Ui, opacity: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let previous = push_opacity(ui, opacity);
    let out = add(ui);
    ui.set_opacity(previous);
    out
}

// ---------------------------------------------------------------------------------------------
// Preference source
// ---------------------------------------------------------------------------------------------

/// Where the motion preference comes from: a fixed value, or the portal reader still running.
#[derive(Debug)]
pub struct MotionControl {
    cli_reduce: bool,
    env_override: Option<bool>,
    reader: Option<PortalReader>,
    resolved: Option<MotionPrefs>,
    installed: Option<MotionPrefs>,
}

impl MotionControl {
    /// A fixed preference (README capture scenes, and the default before `configure_motion`).
    pub fn fixed(prefs: MotionPrefs) -> Self {
        Self {
            cli_reduce: prefs.reduced,
            env_override: None,
            reader: None,
            resolved: Some(prefs),
            installed: None,
        }
    }

    /// Resolves from `--reduce-motion` and the `SOOS_REDUCE_MOTION` value; only when neither
    /// decides is `spawn_portal` called (once) to start the portal reader.
    pub fn from_sources(
        cli_reduce: bool,
        env: Option<&OsStr>,
        spawn_portal: impl FnOnce() -> PortalReader,
    ) -> Self {
        let env_override = parse_reduce_motion_env(env);
        let decided = cli_reduce || env_override.is_some();
        let resolved =
            decided.then(|| resolve_motion_prefs(cli_reduce, env, PortalPreference::Unavailable));
        Self {
            cli_reduce,
            env_override,
            reader: if decided { None } else { Some(spawn_portal()) },
            resolved,
            installed: None,
        }
    }

    /// The preference at `now` and whether the portal answer is still pending (never blocks).
    pub fn current(&mut self, now: Instant) -> (MotionPrefs, bool) {
        if let Some(prefs) = self.resolved {
            return (prefs, false);
        }
        let portal = self
            .reader
            .as_mut()
            .map_or(PortalPreference::Unavailable, |r| r.poll_at(now));
        let env = self.env_override.map(|r| if r { "1" } else { "0" });
        let prefs = resolve_motion_prefs(self.cli_reduce, env.map(OsStr::new), portal);
        if portal == PortalPreference::Pending {
            return (prefs, true);
        }
        self.resolved = Some(prefs);
        self.reader = None;
        (prefs, false)
    }

    /// Polls the source, installs a changed preference into `ctx` and, while the portal is
    /// pending, asks for one repaint after `AMBIENT_REPAINT_MS` so the deadline is observed on
    /// an idle window.
    pub fn apply(&mut self, ctx: &egui::Context) {
        let (prefs, pending) = self.current(Instant::now());
        if self.installed != Some(prefs) {
            install_prefs(ctx, prefs);
            self.installed = Some(prefs);
        }
        if pending {
            ctx.request_repaint_after(Duration::from_millis(theme::AMBIENT_REPAINT_MS));
        }
    }
}

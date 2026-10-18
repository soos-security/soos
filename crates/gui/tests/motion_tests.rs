//! Contract tests for the `soos-gui` motion system (GitHub #438 = PR 3 of #358, matrix UIM9 and
//! GMO1–GMO14; `Docs/UI_MOTION.md` §2, §3, §5, §7–§9; `AI/tester_contract_motion.md` Part B.2).
//!
//! Contract:
//! - `theme.rs` carries every motion token with the documented value;
//! - `SpringState` is the closed-form damped oscillator: frame-rate independent, finite on any
//!   input, retargetable without a jump, settled exactly on its target;
//! - `cubic_bezier` maps 0 → 0 and 1 → 1, clamps, is monotonic for the three standard easings;
//! - reduced motion resolves `--reduce-motion` > `SOOS_REDUCE_MOTION` > portal > full motion, and a
//!   pending portal counts as reduced;
//! - the portal reader never blocks the UI, answers within its deadline, never retries;
//! - motion code requests no repaint once settled, and the pending star only
//!   `request_repaint_after` ≥ 33 ms;
//! - the face overlay is drawn from the latest frame only, and the spinners are gone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::manual_range_contains,
    clippy::float_cmp,
    reason = "Contractual test suite utilizes direct assertions, unwrap, indexing and exact float checks"
)]

use std::ffi::OsStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use clap::Parser;
use eframe::egui::{self, pos2, vec2, Color32, Id, Rect};
use soos_gui::args::GuiArgs;
use soos_gui::daemon_control::DaemonState;
use soos_gui::header::{self, DaemonSwitch};
use soos_gui::motion::portal::{
    interpret, PortalReader, PortalReadings, SettingValue, SettingsBus,
};
use soos_gui::motion::{
    self, clamp_dt, cubic_bezier, entrance_at, mix_color, parse_reduce_motion_env,
    pending_star_angle, resolve_motion_prefs, Entrance, MotionControl, MotionPrefs,
    PortalPreference, SpringState, Tween,
};
use soos_gui::overlay::{self, OverlayToggles};
use soos_gui::state::LatestFrameData;
use soos_gui::theme;
use soos_inference_ort::{BoundingBox, FaceDetection};
use zeroize::Zeroizing;

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// Runs one headless egui pass with `dt` seconds of predicted frame time and returns the
/// repaint delay egui asks for afterwards (`Duration::MAX` = no repaint requested).
fn pass(ctx: &egui::Context, dt: f32, mut f: impl FnMut(&mut egui::Ui)) -> Duration {
    let input = egui::RawInput {
        predicted_dt: dt,
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| f(ui));
    output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map_or(Duration::MAX, |v| v.repaint_delay)
}

/// Whether `delay` (as reported after a 60 Hz pass) is an ambient `request_repaint_after` of at
/// least `AMBIENT_REPAINT_MS`: never immediate, never absent. egui reports the requested
/// duration minus the predicted frame time (so the frame lands on time), hence the addition.
fn is_ambient_repaint(delay: Duration) -> bool {
    delay > Duration::ZERO
        && delay < Duration::MAX
        && delay + Duration::from_secs_f32(1.0 / 60.0)
            >= Duration::from_millis(theme::AMBIENT_REPAINT_MS)
}

/// Same as [`pass`], returning the painted shapes too.
fn pass_shapes(
    ctx: &egui::Context,
    mut f: impl FnMut(&mut egui::Ui),
) -> Vec<egui::epaint::ClippedShape> {
    let input = egui::RawInput {
        predicted_dt: 1.0 / 60.0,
        ..Default::default()
    };
    ctx.run_ui(input, |ui| f(ui)).shapes
}

/// A context with the brand theme, `prefs` installed and a few warm-up passes done, so the
/// first-frame repaints egui itself requests are out of the way.
fn ctx_with(prefs: MotionPrefs) -> egui::Context {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    motion::install_prefs(&ctx, prefs);
    for _ in 0..5 {
        let _ = pass(&ctx, 1.0 / 60.0, |_| {});
    }
    ctx
}

/// Steps a spring from 0 to 1 at `hz` for `seconds`; returns `(settle_time, peak, final)`.
fn run_spring(hz: f32, seconds: f32) -> (Option<f32>, f32, SpringState) {
    let dt = 1.0 / hz;
    let mut s = SpringState::new(0.0);
    s.retarget(1.0);
    let mut peak = 0.0_f32;
    let mut settled_at = None;
    let steps = (seconds * hz).ceil() as usize;
    for i in 1..=steps {
        let v = s.step(dt);
        assert!(v.is_finite());
        peak = peak.max(v);
        if s.settled() {
            if settled_at.is_none() {
                settled_at = Some(i as f32 * dt);
            }
        } else {
            assert!(
                settled_at.is_none(),
                "{hz} Hz: the spring unsettled again at step {i}"
            );
        }
    }
    (settled_at, peak, s)
}

fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Production code of a source file: line comments removed, `#[cfg(test)]` modules cut.
fn production(src: &str) -> String {
    let code = strip_line_comments(src);
    match code.find("#[cfg(test)]") {
        Some(i) => code[..i].to_string(),
        None => code,
    }
}

// ---------------------------------------------------------------------------------------------
// GMO1 — tokens
// ---------------------------------------------------------------------------------------------

/// GMO1: every motion token of `Docs/UI_MOTION.md` §2 and §3 is in `theme.rs` with its value.
#[test]
fn test_gmo1_theme_exposes_every_motion_token() {
    let durations = [
        (theme::T_FAST, 0.12),
        (theme::T_SHORT, 0.18),
        (theme::T_MEDIUM, 0.26),
        (theme::T_LONG, 0.38),
        (theme::T_SPRING, 0.4),
        (theme::T_SHORT_EXIT, 0.126),
        (theme::T_MEDIUM_EXIT, 0.182),
        (theme::T_LONG_EXIT, 0.266),
        (theme::T_STAGGER, 0.03),
        (theme::T_PENDING, 1.2),
    ];
    for (actual, expected) in durations {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
    for (exit, entry) in [
        (theme::T_SHORT_EXIT, theme::T_SHORT),
        (theme::T_MEDIUM_EXIT, theme::T_MEDIUM),
        (theme::T_LONG_EXIT, theme::T_LONG),
    ] {
        assert_eq!(
            (exit * 1000.0).round(),
            (entry * 1000.0 * 0.7).round(),
            "exits run at 0.7 x entry"
        );
    }
    assert_eq!(theme::STAGGER_MAX_ITEMS, 6);
    assert_eq!(theme::EASE_STANDARD, [0.2, 0.0, 0.0, 1.0]);
    assert_eq!(theme::EASE_ENTER, [0.16, 1.0, 0.3, 1.0]);
    assert_eq!(theme::EASE_EXIT, [0.4, 0.0, 1.0, 1.0]);
    assert_eq!(theme::PRESS_SCALE, 0.98);
    assert_eq!(theme::RISE_PX, 6.0);
    assert_eq!(theme::SLIDE_PX, 8.0);
    assert_eq!(theme::STAR_TURN_DEG, 45.0);
    assert_eq!(theme::SPRING_STIFFNESS, 400.0);
    assert_eq!(theme::SPRING_DAMPING, 30.0);
    assert_eq!(theme::SPRING_MASS, 1.0);
    assert_eq!(theme::SPRING_SETTLE_EPSILON, 0.001);
    assert_eq!(theme::SPRING_SETTLE_VELOCITY, 0.1);
    assert_eq!(theme::MAX_MOTION_DT, 0.05);
    assert_eq!(theme::AMBIENT_REPAINT_MS, 33);
    assert_eq!(theme::PORTAL_READ_DEADLINE_MS, 200);
    // The spring's 0.1 % settle time is T_SPRING: zeta = c / (2 sqrt(k m)) = 0.75.
    let zeta =
        theme::SPRING_DAMPING / (2.0 * (theme::SPRING_STIFFNESS * theme::SPRING_MASS).sqrt());
    assert!((zeta - 0.75).abs() < 1e-6);
}

// ---------------------------------------------------------------------------------------------
// GMO2 — spring
// ---------------------------------------------------------------------------------------------

/// GMO2: from 0 to 1 at 30, 60 and 144 Hz the spring settles within 450 ms with an overshoot
/// between 1 % and 4 %, stays settled and ends exactly on the target.
#[test]
fn test_gmo2_spring_settles_at_30_60_144_hz_with_bounded_overshoot() {
    for hz in [30.0_f32, 60.0, 144.0] {
        let (settled_at, peak, s) = run_spring(hz, 1.5);
        let t = settled_at.unwrap_or_else(|| panic!("{hz} Hz: never settled"));
        assert!(t <= 0.45, "{hz} Hz: settled after {t} s");
        let overshoot = peak - 1.0;
        assert!(
            (0.01..=0.04).contains(&overshoot),
            "{hz} Hz: overshoot {overshoot}"
        );
        assert!(s.settled());
        assert_eq!(
            s.value(),
            1.0,
            "{hz} Hz: a settled spring sits exactly on its target"
        );
        assert_eq!(s.velocity(), 0.0);
    }
}

/// GMO2: the closed form is frame-rate independent: 60 Hz and 144 Hz agree at the same time.
#[test]
fn test_gmo2_spring_is_frame_rate_independent() {
    let mut a = SpringState::new(0.0);
    let mut b = SpringState::new(0.0);
    a.retarget(1.0);
    b.retarget(1.0);
    for _ in 0..12 {
        a.step(1.0 / 60.0);
    }
    for _ in 0..24 {
        b.step(1.0 / 120.0);
    }
    assert!(
        (a.value() - b.value()).abs() < 1e-4,
        "{} vs {}",
        a.value(),
        b.value()
    );
}

/// GMO2: a retarget at peak velocity keeps value and velocity (no jump) and reverses smoothly.
#[test]
fn test_gmo2_spring_retarget_mid_flight_reverses_without_discontinuity() {
    let dt = 1.0 / 60.0;
    let mut s = SpringState::new(0.0);
    s.retarget(1.0);
    let mut peak_v = 0.0_f32;
    loop {
        let before = s;
        s.step(dt);
        if s.velocity() < before.velocity() {
            s = before;
            break;
        }
        peak_v = s.velocity();
    }
    assert!(
        peak_v > 1.0,
        "the spring is moving fast at its peak velocity"
    );
    let (value, velocity) = (s.value(), s.velocity());
    s.retarget(0.0);
    assert_eq!(s.value(), value, "retarget never moves the value");
    assert_eq!(s.velocity(), velocity, "retarget keeps the velocity");
    let mut prev = s.value();
    for _ in 0..90 {
        let v = s.step(dt);
        assert!(
            (v - prev).abs() <= peak_v * dt * 1.01,
            "jump of {} in one frame",
            (v - prev).abs()
        );
        prev = v;
    }
    assert!(s.settled());
    assert_eq!(s.value(), 0.0);
}

/// GMO2: retargeting every frame to the unchanged target is a no-op (same trajectory, settles).
#[test]
fn test_gmo2_spring_retarget_to_same_target_is_a_noop() {
    let dt = 1.0 / 60.0;
    let mut plain = SpringState::new(0.0);
    let mut nagged = SpringState::new(0.0);
    plain.retarget(1.0);
    nagged.retarget(1.0);
    for _ in 0..60 {
        nagged.retarget(1.0);
        assert_eq!(plain.step(dt), nagged.step(dt));
    }
    assert!(nagged.settled());
    assert_eq!(nagged.value(), 1.0);
}

/// GMO2: `dt` = 0, negative, NaN, ±inf and 10 s never produce a non-finite value or panic, and
/// any stall advances at most `MAX_MOTION_DT`.
#[test]
fn test_gmo2_spring_dt_edge_values_are_harmless() {
    assert_eq!(clamp_dt(0.0), 0.0);
    assert_eq!(clamp_dt(-1.0), 0.0);
    assert_eq!(clamp_dt(f32::NAN), 0.0);
    assert_eq!(clamp_dt(f32::NEG_INFINITY), 0.0);
    assert_eq!(clamp_dt(f32::INFINITY), theme::MAX_MOTION_DT);
    assert_eq!(clamp_dt(10.0), theme::MAX_MOTION_DT);
    assert_eq!(clamp_dt(0.01), 0.01);

    let mut moving = SpringState::new(0.0);
    moving.retarget(1.0);
    moving.step(1.0 / 60.0);
    for dt in [0.0, -1.0, f32::NAN, f32::NEG_INFINITY] {
        let mut s = moving;
        let v = s.step(dt);
        assert!(v.is_finite());
        assert_eq!(v, moving.value(), "dt {dt} must not move the spring");
    }
    let mut one_max_step = moving;
    one_max_step.step(theme::MAX_MOTION_DT);
    for dt in [10.0, f32::INFINITY] {
        let mut s = moving;
        let v = s.step(dt);
        assert!(v.is_finite());
        assert_eq!(
            v,
            one_max_step.value(),
            "dt {dt} advances at most MAX_MOTION_DT"
        );
    }
}

/// GMO2: a non-finite target is ignored, a non-finite start is made finite, a zero span is
/// settled at once.
#[test]
fn test_gmo2_spring_non_finite_and_zero_span_inputs() {
    let mut s = SpringState::new(0.25);
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        s.retarget(bad);
        assert_eq!(s.target(), 0.25, "non-finite target {bad} ignored");
        assert!(s.settled());
    }
    let n = SpringState::new(f32::NAN);
    assert!(n.value().is_finite() && n.target().is_finite());
    let mut z = SpringState::new(0.5);
    z.retarget(0.5);
    assert!(z.settled(), "a zero span is settled at once");
    assert_eq!(z.step(1.0 / 60.0), 0.5);
    // Large and tiny spans settle too (the epsilon is relative to the span).
    for span in [1e-4_f32, 500.0] {
        let mut s = SpringState::new(0.0);
        s.retarget(span);
        for _ in 0..60 {
            s.step(1.0 / 60.0);
        }
        assert!(s.settled(), "span {span}");
        assert_eq!(s.value(), span);
    }
}

// ---------------------------------------------------------------------------------------------
// GMO3 — easing
// ---------------------------------------------------------------------------------------------

/// Reference cubic-bezier evaluation by plain bisection on x (independent of the product).
fn reference_bezier(p: [f32; 4], t: f64) -> f64 {
    let [x1, y1, x2, y2] = p.map(f64::from);
    let bez = |a: f64, b: f64, s: f64| {
        3.0 * a * s * (1.0 - s).powi(2) + 3.0 * b * s * s * (1.0 - s) + s.powi(3)
    };
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..80 {
        let mid = (lo + hi) / 2.0;
        if bez(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(y1, y2, (lo + hi) / 2.0)
}

/// GMO3: `cubic_bezier` maps 0 → 0 and 1 → 1, clamps, is monotonic, matches the reference, and
/// turns NaN into a finite value in [0, 1].
#[test]
fn test_gmo3_cubic_bezier_endpoints_clamp_monotonic_and_nan() {
    for p in [theme::EASE_STANDARD, theme::EASE_ENTER, theme::EASE_EXIT] {
        assert_eq!(cubic_bezier(p, 0.0), 0.0);
        assert_eq!(cubic_bezier(p, 1.0), 1.0);
        assert_eq!(cubic_bezier(p, -3.0), 0.0);
        assert_eq!(cubic_bezier(p, 7.0), 1.0);
        assert_eq!(cubic_bezier(p, f32::INFINITY), 1.0);
        assert_eq!(cubic_bezier(p, f32::NEG_INFINITY), 0.0);
        let nan = cubic_bezier(p, f32::NAN);
        assert!(nan.is_finite() && (0.0..=1.0).contains(&nan));
        let mut prev = 0.0;
        for i in 0..=1000 {
            let t = i as f32 / 1000.0;
            let y = cubic_bezier(p, t);
            assert!(y >= prev - 1e-6, "{p:?} not monotonic at {t}");
            assert!(
                (y - reference_bezier(p, f64::from(t)) as f32).abs() < 1e-3,
                "{p:?} at {t}"
            );
            prev = y;
        }
    }
}

/// GMO3: `Tween` eases from the shown value, settles exactly after its duration, and a zero
/// duration settles at once.
#[test]
fn test_gmo3_tween_eases_and_settles_exactly() {
    let mut tw = Tween::new(0.0);
    tw.retarget(10.0, theme::T_MEDIUM, theme::EASE_STANDARD);
    let mut prev = 0.0;
    let mut steps = 0;
    while !tw.settled() {
        let v = tw.step(1.0 / 60.0);
        assert!(v >= prev && v <= 10.0);
        prev = v;
        steps += 1;
        assert!(steps < 100);
    }
    assert_eq!(tw.value(), 10.0);
    assert!(steps as f32 / 60.0 <= theme::T_MEDIUM + 1.0 / 60.0);
    // A retarget mid-flight starts from the shown value (no jump).
    let mut tw = Tween::new(0.0);
    tw.retarget(1.0, theme::T_MEDIUM, theme::EASE_STANDARD);
    tw.step(0.1);
    let shown = tw.value();
    tw.retarget(0.0, theme::T_MEDIUM, theme::EASE_STANDARD);
    assert_eq!(tw.value(), shown);
    let mut z = Tween::new(3.0);
    z.retarget(4.0, 0.0, theme::EASE_STANDARD);
    assert!(z.settled());
    assert_eq!(z.value(), 4.0);
    let mut bad = Tween::new(3.0);
    bad.retarget(f32::NAN, theme::T_SHORT, theme::EASE_STANDARD);
    assert_eq!(bad.value(), 3.0, "a non-finite target is ignored");
}

/// GMO3: colour mixing is exact at both ends (a settled widget paints its old colours).
#[test]
fn test_gmo3_mix_color_is_exact_at_the_ends() {
    let (a, b) = (theme::LINE, theme::SUCCESS_TOGGLE);
    assert_eq!(mix_color(a, b, 0.0), a);
    assert_eq!(mix_color(a, b, -1.0), a);
    assert_eq!(mix_color(a, b, 1.0), b);
    assert_eq!(mix_color(a, b, 2.0), b);
    assert_eq!(mix_color(a, b, f32::NAN), a);
    let mid = mix_color(
        Color32::from_rgb(0, 0, 0),
        Color32::from_rgb(200, 100, 50),
        0.5,
    );
    assert_eq!(mid, Color32::from_rgb(100, 50, 25));
}

// ---------------------------------------------------------------------------------------------
// GMO4 — reduced-motion resolution
// ---------------------------------------------------------------------------------------------

/// GMO4: `--reduce-motion` > `SOOS_REDUCE_MOTION` (`1`/`0`) > portal > full motion; a pending
/// portal counts as reduced; other or over-long values are ignored.
#[test]
fn test_gmo4_resolution_order_and_values() {
    use PortalPreference::{NoPreference, Pending, Reduce, Unavailable};
    let one = Some(OsStr::new("1"));
    let zero = Some(OsStr::new("0"));
    for portal in [Reduce, NoPreference, Unavailable, Pending] {
        for env in [None, one, zero, Some(OsStr::new("yes"))] {
            assert!(
                resolve_motion_prefs(true, env, portal).reduced,
                "the CLI switch wins ({env:?}, {portal:?})"
            );
        }
        assert!(resolve_motion_prefs(false, one, portal).reduced);
        assert!(
            !resolve_motion_prefs(false, zero, portal).reduced,
            "SOOS_REDUCE_MOTION=0 forces full motion even when the portal says {portal:?}"
        );
    }
    assert!(resolve_motion_prefs(false, None, Reduce).reduced);
    assert!(resolve_motion_prefs(false, None, Pending).reduced);
    assert!(!resolve_motion_prefs(false, None, NoPreference).reduced);
    assert!(!resolve_motion_prefs(false, None, Unavailable).reduced);
    for ignored in ["", "2", "true", " 1", "1 ", "01", "123456789"] {
        assert_eq!(
            parse_reduce_motion_env(Some(OsStr::new(ignored))),
            None,
            "{ignored:?}"
        );
        assert!(resolve_motion_prefs(false, Some(OsStr::new(ignored)), Reduce).reduced);
        assert!(!resolve_motion_prefs(false, Some(OsStr::new(ignored)), NoPreference).reduced);
    }
    assert_eq!(parse_reduce_motion_env(one), Some(true));
    assert_eq!(parse_reduce_motion_env(zero), Some(false));
    assert_eq!(parse_reduce_motion_env(None), None);
    assert_eq!(motion::REDUCE_MOTION_ENV, "SOOS_REDUCE_MOTION");
    assert_eq!(motion::MAX_REDUCE_MOTION_ENV_BYTES, 8);
    assert_ne!(MotionPrefs::FULL, MotionPrefs::REDUCED);
    assert_eq!(
        resolve_motion_prefs(true, None, NoPreference),
        MotionPrefs::REDUCED
    );
}

/// GMO4: `GuiArgs` parses `--reduce-motion` (off by default).
#[test]
fn test_gmo4_gui_args_parse_reduce_motion() {
    assert!(!GuiArgs::try_parse_from(["soos-gui"]).unwrap().reduce_motion);
    assert!(
        GuiArgs::try_parse_from(["soos-gui", "--reduce-motion"])
            .unwrap()
            .reduce_motion
    );
}

// ---------------------------------------------------------------------------------------------
// GMO5 — portal reader
// ---------------------------------------------------------------------------------------------

/// A bus that never answers (blocks until the test process ends).
struct SilentBus;
impl SettingsBus for SilentBus {
    fn read_settings(&mut self, _deadline: Duration) -> PortalReadings {
        loop {
            std::thread::park();
        }
    }
}

/// A bus that panics (the reader thread dies without answering).
struct PanickingBus;
impl SettingsBus for PanickingBus {
    fn read_settings(&mut self, _deadline: Duration) -> PortalReadings {
        panic!("fake bus failure");
    }
}

/// A bus that answers `readings` after `delay`, counting its calls.
struct ScriptedBus {
    readings: PortalReadings,
    delay: Duration,
    calls: Arc<AtomicUsize>,
}
impl SettingsBus for ScriptedBus {
    fn read_settings(&mut self, _deadline: Duration) -> PortalReadings {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        self.readings
    }
}

/// A bus that blocks until the test releases it, recording the thread it ran on.
struct GatedBus {
    gate: Mutex<mpsc::Receiver<()>>,
    thread: Arc<Mutex<Option<std::thread::ThreadId>>>,
}
impl SettingsBus for GatedBus {
    fn read_settings(&mut self, _deadline: Duration) -> PortalReadings {
        *self.thread.lock().unwrap() = Some(std::thread::current().id());
        let _ = self.gate.lock().unwrap().recv();
        PortalReadings {
            reduced_motion: Some(SettingValue::U32(1)),
            enable_animations: None,
        }
    }
}

/// Polls until the answer is not `Pending` (or `bound` passes); returns the answer, the time
/// waited and the instant passed to the poll that produced it.
fn wait_answer(
    reader: &mut PortalReader,
    bound: Duration,
) -> (PortalPreference, Duration, Instant) {
    let start = Instant::now();
    loop {
        let now = Instant::now();
        let p = reader.poll_at(now);
        if p != PortalPreference::Pending || start.elapsed() > bound {
            return (p, start.elapsed(), now);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// GMO5: the portal values are validated: `reduced-motion` 1/0 decides, anything else falls back
/// to `enable-animations`, otherwise the portal is unavailable.
#[test]
fn test_gmo5_portal_values_are_validated() {
    use PortalPreference::{NoPreference, Reduce, Unavailable};
    use SettingValue::{Bool, Other, U32};
    let r = |rm, ea| PortalReadings {
        reduced_motion: rm,
        enable_animations: ea,
    };
    assert_eq!(interpret(r(Some(U32(1)), None)), Reduce);
    assert_eq!(interpret(r(Some(U32(0)), None)), NoPreference);
    assert_eq!(interpret(r(Some(U32(1)), Some(Bool(true)))), Reduce);
    assert_eq!(interpret(r(Some(U32(0)), Some(Bool(false)))), NoPreference);
    for odd in [
        Some(U32(2)),
        Some(U32(u32::MAX)),
        Some(Bool(true)),
        Some(Other),
        None,
    ] {
        assert_eq!(interpret(r(odd, Some(Bool(false)))), Reduce, "{odd:?}");
        assert_eq!(interpret(r(odd, Some(Bool(true)))), NoPreference, "{odd:?}");
        assert_eq!(interpret(r(odd, Some(U32(0)))), Unavailable, "{odd:?}");
        assert_eq!(interpret(r(odd, Some(Other))), Unavailable, "{odd:?}");
        assert_eq!(interpret(r(odd, None)), Unavailable, "{odd:?}");
    }
}

/// GMO5: a bus that never answers yields `Unavailable` once the 200 ms deadline has passed
/// (never before it; the upper bound is generous for CI).
#[test]
fn test_gmo5_silent_bus_is_unavailable_after_the_deadline() {
    let deadline = Duration::from_millis(theme::PORTAL_READ_DEADLINE_MS);
    let mut reader = PortalReader::spawn(SilentBus, deadline);
    let started = Instant::now();
    assert_eq!(
        reader.poll_at(reader.deadline_at() - Duration::from_millis(1)),
        PortalPreference::Pending
    );
    let (p, _, observed) = wait_answer(&mut reader, Duration::from_secs(5));
    let waited = started.elapsed();
    assert_eq!(p, PortalPreference::Unavailable);
    assert!(waited <= deadline + Duration::from_secs(2), "{waited:?}");
    assert!(
        observed >= reader.deadline_at(),
        "never Unavailable before the deadline"
    );
}

/// GMO5: a reader thread that dies without answering (disconnected channel) is `Unavailable`
/// without waiting for the deadline.
#[test]
fn test_gmo5_dead_reader_thread_is_unavailable() {
    let mut reader = PortalReader::spawn(PanickingBus, Duration::from_secs(30));
    let (p, waited, _) = wait_answer(&mut reader, Duration::from_secs(10));
    assert_eq!(p, PortalPreference::Unavailable);
    assert!(waited < Duration::from_secs(10), "{waited:?}");
}

/// GMO5: the reader runs off the calling thread and the poll never blocks while the bus is
/// stuck; the answer arrives once the bus answers.
#[test]
fn test_gmo5_poll_never_blocks_and_runs_off_the_ui_thread() {
    let (release, gate) = mpsc::channel();
    let thread = Arc::new(Mutex::new(None));
    let mut reader = PortalReader::spawn(
        GatedBus {
            gate: Mutex::new(gate),
            thread: Arc::clone(&thread),
        },
        Duration::from_secs(30),
    );
    for _ in 0..200 {
        assert_eq!(reader.poll(), PortalPreference::Pending);
    }
    release.send(()).unwrap();
    let (p, _, _) = wait_answer(&mut reader, Duration::from_secs(10));
    assert_eq!(p, PortalPreference::Reduce);
    let ran_on = thread.lock().unwrap().expect("the bus ran");
    assert_ne!(
        ran_on,
        std::thread::current().id(),
        "never on the UI thread"
    );
}

/// GMO5: one read, no retry; a late answer after the deadline never overrides `Unavailable`.
#[test]
fn test_gmo5_no_retry_and_no_late_override() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut reader = PortalReader::spawn(
        ScriptedBus {
            readings: PortalReadings {
                reduced_motion: None,
                enable_animations: None,
            },
            delay: Duration::ZERO,
            calls: Arc::clone(&calls),
        },
        Duration::from_secs(30),
    );
    let (p, _, _) = wait_answer(&mut reader, Duration::from_secs(10));
    assert_eq!(p, PortalPreference::Unavailable);
    for _ in 0..50 {
        assert_eq!(reader.poll(), PortalPreference::Unavailable);
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no retry after a failure");

    let late_calls = Arc::new(AtomicUsize::new(0));
    let mut late = PortalReader::spawn(
        ScriptedBus {
            readings: PortalReadings {
                reduced_motion: Some(SettingValue::U32(1)),
                enable_animations: None,
            },
            delay: Duration::from_millis(400),
            calls: Arc::clone(&late_calls),
        },
        Duration::from_millis(50),
    );
    let (p, _, _) = wait_answer(&mut late, Duration::from_secs(5));
    assert_eq!(p, PortalPreference::Unavailable);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        late.poll(),
        PortalPreference::Unavailable,
        "cached, never overridden"
    );
    assert_eq!(late_calls.load(Ordering::SeqCst), 1);
}

/// GMO5: the motion code spawns no process and `soos-gui` takes zbus and Tokio from the
/// workspace as direct dependencies.
#[test]
fn test_gmo5_no_process_spawn_and_direct_workspace_dependencies() {
    for (name, src) in [
        ("motion.rs", include_str!("../src/motion.rs")),
        ("motion/portal.rs", include_str!("../src/motion/portal.rs")),
    ] {
        let code = production(src);
        for forbidden in ["Command::new", "std::process", "process::Command"] {
            assert!(!code.contains(forbidden), "{name} must not use {forbidden}");
        }
    }
    let manifest = include_str!("../Cargo.toml");
    let deps: Vec<&str> = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap()
        .lines()
        .map(str::trim)
        .collect();
    assert!(deps.contains(&"zbus = { workspace = true }"), "{deps:?}");
    assert!(deps.contains(&"tokio = { workspace = true }"), "{deps:?}");
}

// ---------------------------------------------------------------------------------------------
// GMO6 — reduced behaviour
// ---------------------------------------------------------------------------------------------

/// GMO6: with reduced motion every helper returns its target on the first frame, the style's
/// animation time is 0 and motion code requests no repaint.
#[test]
fn test_gmo6_reduced_motion_returns_targets_at_once_without_repaint() {
    let ctx = ctx_with(MotionPrefs::REDUCED);
    assert_eq!(ctx.global_style().animation_time, 0.0);
    assert_eq!(motion::prefs(&ctx), MotionPrefs::REDUCED);
    let id = Id::new("gmo6");
    for frame in 0..20 {
        let target = if frame % 2 == 0 { 1.0 } else { 0.0 };
        let delay = pass(&ctx, 1.0 / 60.0, |ui| {
            let c = ui.ctx().clone();
            assert_eq!(motion::spring_value(&c, id.with("s"), target), target);
            assert_eq!(
                motion::tween_value(
                    &c,
                    id.with("t"),
                    target * 5.0,
                    theme::T_MEDIUM,
                    theme::EASE_STANDARD
                ),
                target * 5.0
            );
            assert_eq!(
                motion::fade_in_on_change(&c, id.with("f"), frame, theme::T_SHORT),
                1.0
            );
            assert_eq!(
                motion::entrance(&c, id.with(("e", frame)), 0),
                Entrance::REST
            );
            assert_eq!(motion::pending_star_rotation(&c), 0.0);
        });
        assert_eq!(
            delay,
            Duration::MAX,
            "frame {frame}: reduced motion requested a repaint"
        );
    }
}

/// GMO6/G10: full motion sets `Style::animation_time` to `T_SHORT`; nothing installed counts as
/// reduced (a pending portal).
#[test]
fn test_gmo6_animation_time_follows_prefs_and_default_is_reduced() {
    let ctx = egui::Context::default();
    assert_eq!(motion::prefs(&ctx), MotionPrefs::REDUCED);
    motion::install_prefs(&ctx, MotionPrefs::FULL);
    assert_eq!(ctx.global_style().animation_time, theme::T_SHORT);
    assert_eq!(motion::prefs(&ctx), MotionPrefs::FULL);
    motion::install_prefs(&ctx, MotionPrefs::REDUCED);
    assert_eq!(ctx.global_style().animation_time, 0.0);
}

// ---------------------------------------------------------------------------------------------
// GMO7 — repaint budget
// ---------------------------------------------------------------------------------------------

/// GMO7: an interactive transition repaints every frame while unsettled, stops on the frame it
/// settles, and 120 idle frames afterwards request no repaint from motion code.
#[test]
fn test_gmo7_idle_budget_no_repaint_once_settled() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let id = Id::new("gmo7");
    let draw = |target: f32| {
        move |ui: &mut egui::Ui| {
            let c = ui.ctx().clone();
            let _ = motion::spring_value(&c, id.with("s"), target);
            let _ = motion::tween_value(
                &c,
                id.with("t"),
                target,
                theme::T_MEDIUM,
                theme::EASE_STANDARD,
            );
            let _ = motion::entrance(&c, id.with("e"), 3);
            let _ = motion::fade_in_on_change(
                &c,
                id.with("f"),
                u64::from(target.to_bits()),
                theme::T_SHORT,
            );
        }
    };
    assert_eq!(
        pass(&ctx, 1.0 / 60.0, draw(0.0)),
        Duration::ZERO,
        "the entrance runs"
    );
    let mut frames = 1;
    while pass(&ctx, 1.0 / 60.0, draw(0.0)) != Duration::MAX {
        frames += 1;
        assert!(frames < 120, "the entrance never settled");
    }
    assert_eq!(
        pass(&ctx, 1.0 / 60.0, draw(1.0)),
        Duration::ZERO,
        "a retarget animates"
    );
    let mut moving = 1;
    while pass(&ctx, 1.0 / 60.0, draw(1.0)) == Duration::ZERO {
        moving += 1;
        assert!(moving < 60, "never settled");
    }
    assert!(moving as f32 / 60.0 <= 0.45 + 1.0 / 60.0, "{moving} frames");
    for frame in 0..120 {
        assert_eq!(
            pass(&ctx, 1.0 / 60.0, draw(1.0)),
            Duration::MAX,
            "idle frame {frame} requested a repaint"
        );
    }
}

/// GMO7: the pending star repaints only through `request_repaint_after` with at least 33 ms.
#[test]
fn test_gmo7_pending_star_uses_ambient_repaint_after() {
    let ctx = ctx_with(MotionPrefs::FULL);
    for _ in 0..30 {
        let delay = pass(&ctx, 1.0 / 60.0, |ui| {
            let a = motion::pending_star_rotation(ui.ctx());
            assert!(a.is_finite());
        });
        assert!(is_ambient_repaint(delay), "{delay:?}");
    }
}

/// GMO7: the pending-star angle turns a quarter turn per `T_PENDING` with the standard easing,
/// modulo a half turn.
#[test]
fn test_gmo7_pending_star_angle_quarter_turn_per_period() {
    let quarter = std::f32::consts::FRAC_PI_2;
    assert_eq!(pending_star_angle(0.0), 0.0);
    let p = f64::from(theme::T_PENDING);
    assert!((pending_star_angle(p) - quarter).abs() < 1e-4);
    assert!(
        pending_star_angle(2.0 * p).abs() < 1e-4,
        "modulo a half turn"
    );
    let half = pending_star_angle(p / 2.0);
    let expected = quarter * cubic_bezier(theme::EASE_STANDARD, 0.5);
    assert!((half - expected).abs() < 1e-4);
    for t in [f64::NAN, f64::INFINITY, -1.0, 1e12] {
        let a = pending_star_angle(t);
        assert!(
            a.is_finite() && (0.0..std::f32::consts::PI).contains(&a),
            "{t}: {a}"
        );
    }
}

/// GMO7: widgets at rest request nothing; the busy daemon panel only the ambient repaint.
#[test]
fn test_gmo7_daemon_panel_repaint_budget() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let panel = Rect::from_min_size(pos2(600.0, 0.0), vec2(260.0, 41.0));
    let idle = DaemonSwitch::new(DaemonState::Active, false);
    for _ in 0..10 {
        let _ = pass(&ctx, 1.0 / 60.0, |ui| {
            let _ = header::daemon_panel(ui, panel, &idle);
        });
    }
    for frame in 0..120 {
        let delay = pass(&ctx, 1.0 / 60.0, |ui| {
            let _ = header::daemon_panel(ui, panel, &idle);
        });
        assert_eq!(delay, Duration::MAX, "idle frame {frame}");
    }
    let busy = DaemonSwitch::new(DaemonState::Active, true);
    let delay = pass(&ctx, 1.0 / 60.0, |ui| {
        let _ = header::daemon_panel(ui, panel, &busy);
    });
    assert!(is_ambient_repaint(delay), "{delay:?}");
}

// ---------------------------------------------------------------------------------------------
// GMO8 — truthful overlay
// ---------------------------------------------------------------------------------------------

fn frame_with(pad_live: bool, x1: f32, sequence: u64) -> LatestFrameData {
    LatestFrameData {
        rgb: Zeroizing::new(vec![0; 640 * 480 * 3]),
        width: 640,
        height: 480,
        detections: vec![FaceDetection::new(
            BoundingBox::new(x1, 100.0, x1 + 160.0, 300.0),
            0.97,
        )],
        pad_result: None,
        pad_live,
        pose: None,
        aligned_crop: None,
        pipeline_latency_ms: 10.0,
        det_latency_ms: 4.0,
        pad_latency_ms: 2.0,
        fps: 30.0,
        sequence,
    }
}

fn stroked_rects(shapes: &[egui::epaint::ClippedShape], color: Color32) -> Vec<Rect> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.stroke.color == color && r.stroke.width > 1.5 => Some(r.rect),
            _ => None,
        })
        .collect()
}

/// GMO8: a frame whose PAD verdict flips from live to spoof is drawn with `FACE_SPOOF` on that
/// same frame, at the latest detection (no interpolation), even with full motion.
#[test]
fn test_gmo8_face_overlay_is_never_interpolated() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let video = Rect::from_min_size(pos2(40.0, 60.0), vec2(640.0, 480.0));
    let toggles = OverlayToggles {
        bbox: true,
        landmarks: true,
        pad_crop: true,
    };
    let live = frame_with(true, 100.0, 1);
    let shapes = pass_shapes(&ctx, |ui| {
        overlay::paint_detections(ui.painter(), video, &live, toggles)
    });
    let live_boxes = stroked_rects(&shapes, theme::FACE_LIVE);
    assert_eq!(
        live_boxes,
        vec![Rect::from_min_max(pos2(140.0, 160.0), pos2(300.0, 360.0))]
    );
    assert!(stroked_rects(&shapes, theme::FACE_SPOOF).is_empty());

    let spoof = frame_with(false, 300.0, 2);
    let shapes = pass_shapes(&ctx, |ui| {
        overlay::paint_detections(ui.painter(), video, &spoof, toggles)
    });
    assert!(
        stroked_rects(&shapes, theme::FACE_LIVE).is_empty(),
        "no live colour left over"
    );
    assert_eq!(
        stroked_rects(&shapes, theme::FACE_SPOOF),
        vec![Rect::from_min_max(pos2(340.0, 160.0), pos2(500.0, 360.0))],
        "drawn at the latest detection"
    );
    assert_eq!(overlay::face_box_color(true), theme::FACE_LIVE);
    assert_eq!(overlay::face_box_color(false), theme::FACE_SPOOF);
    let src = production(include_str!("../src/overlay.rs"));
    assert!(
        !src.contains("motion::") && !src.contains("animate"),
        "no motion in the overlay"
    );
}

// ---------------------------------------------------------------------------------------------
// GMO9 — spinners replaced
// ---------------------------------------------------------------------------------------------

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

/// GMO9: no `egui::Spinner` remains in the header and the camera status card; the pending
/// daemon panel paints the star and keeps "Waiting for authorization..." visible.
#[test]
fn test_gmo9_spinners_replaced_and_authorization_text_visible() {
    for (name, src) in [
        ("header.rs", include_str!("../src/header.rs")),
        ("camera_status.rs", include_str!("../src/camera_status.rs")),
    ] {
        assert!(!src.contains("Spinner"), "{name} still uses egui::Spinner");
    }
    for prefs in [MotionPrefs::FULL, MotionPrefs::REDUCED] {
        let ctx = ctx_with(prefs);
        let panel = Rect::from_min_size(pos2(600.0, 0.0), vec2(260.0, 41.0));
        let busy = DaemonSwitch::new(DaemonState::Inactive, true);
        let shapes = pass_shapes(&ctx, |ui| {
            assert!(header::daemon_panel(ui, panel, &busy).is_none());
        });
        assert!(
            texts(&shapes)
                .iter()
                .any(|t| t == "Waiting for authorization..."),
            "{prefs:?}: {:?}",
            texts(&shapes)
        );
        let star_meshes = shapes
            .iter()
            .filter(|c| match &c.shape {
                egui::Shape::Rect(r) => r.fill_texture_id() != egui::TextureId::default(),
                egui::Shape::Mesh(m) => m.texture_id != egui::TextureId::default(),
                _ => false,
            })
            .count();
        assert!(star_meshes >= 1, "{prefs:?}: the pending star is painted");
    }
}

// ---------------------------------------------------------------------------------------------
// GMO10 — hygiene
// ---------------------------------------------------------------------------------------------

/// GMO10: `motion.rs` denies panic paths and allocates nothing in production code.
#[test]
fn test_gmo10_motion_module_has_no_panic_path_or_allocation() {
    let src = include_str!("../src/motion.rs");
    for lint in [
        "clippy::indexing_slicing",
        "clippy::unwrap_used",
        "clippy::expect_used",
        "clippy::panic",
    ] {
        let deny = src
            .lines()
            .take_while(|l| !l.starts_with("use "))
            .any(|l| l.contains(lint))
            && src.contains("#![deny(");
        assert!(deny, "motion.rs must #![deny({lint})]");
    }
    assert!(src.contains("#![forbid(unsafe_code)]"));
    let code = production(src);
    for forbidden in [
        ".unwrap()",
        ".expect(",
        "Vec<",
        "vec!",
        "String",
        "format!",
        "Box<",
        ".to_owned()",
        ".to_string()",
        ".collect",
        "insert_temp(",
    ] {
        assert!(
            !code.contains(forbidden),
            "motion.rs production code contains `{forbidden}`"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// GMO11 — tab pill
// ---------------------------------------------------------------------------------------------

/// GMO11 (G3): the tab pill appears at its target, slides to a newly active tab over `T_MEDIUM`
/// with the standard easing, follows its own tab at once when only the rectangle moves (window
/// resize), and is at the new target at once under reduced motion.
#[test]
fn test_gmo11_tab_pill_slides_between_tabs() {
    let a = Rect::from_min_max(pos2(200.0, 0.0), pos2(420.0, 44.0));
    let b = Rect::from_min_max(pos2(420.0, 0.0), pos2(600.0, 44.0));
    let ctx = ctx_with(MotionPrefs::FULL);
    let mut shown = Rect::NOTHING;
    let _ = pass(&ctx, 1.0 / 60.0, |ui| shown = header::tab_pill(ui, a, 0));
    assert_eq!(shown, a, "first sight is at rest");
    let _ = pass(&ctx, 1.0 / 60.0, |ui| shown = header::tab_pill(ui, b, 1));
    assert!(shown.min.x > a.min.x && shown.min.x < b.min.x, "{shown:?}");
    assert!(shown.max.x > a.max.x && shown.max.x < b.max.x, "{shown:?}");
    for _ in 0..30 {
        let _ = pass(&ctx, 1.0 / 60.0, |ui| shown = header::tab_pill(ui, b, 1));
    }
    assert_eq!(shown, b);
    let resized = b.translate(vec2(-37.0, 0.0));
    let _ = pass(&ctx, 1.0 / 60.0, |ui| {
        shown = header::tab_pill(ui, resized, 1)
    });
    assert_eq!(shown, resized, "the same tab is followed at once");

    let reduced = ctx_with(MotionPrefs::REDUCED);
    let _ = pass(&reduced, 1.0 / 60.0, |ui| {
        shown = header::tab_pill(ui, a, 0)
    });
    let _ = pass(&reduced, 1.0 / 60.0, |ui| {
        shown = header::tab_pill(ui, b, 1)
    });
    assert_eq!(shown, b);
}

// ---------------------------------------------------------------------------------------------
// GMO12 — entrances and stagger
// ---------------------------------------------------------------------------------------------

/// GMO12 (G4): entrances fade in and rise `RISE_PX`, staggered `T_STAGGER`, at most 6 items.
#[test]
fn test_gmo12_entrance_stagger_is_bounded() {
    let start = entrance_at(0.0, 0);
    assert_eq!(start.opacity, 0.0);
    assert_eq!(start.offset_y, theme::RISE_PX);
    let fifth = theme::T_STAGGER * 5.0;
    for i in [5, 6, 7, 50, usize::MAX] {
        assert_eq!(
            entrance_at(fifth, i),
            entrance_at(fifth, 5),
            "items beyond the sixth"
        );
        assert_eq!(entrance_at(fifth, i).opacity, 0.0);
    }
    assert!(
        entrance_at(fifth, 0).opacity > 0.0,
        "the first item started earlier"
    );
    assert_eq!(
        entrance_at(fifth + theme::T_MEDIUM + 1e-3, 5),
        Entrance::REST
    );
    assert_eq!(entrance_at(theme::T_MEDIUM, 0), Entrance::REST);
    assert!(entrance_at(f32::NAN, 0).opacity.is_finite());
    let mut prev = 0.0;
    for i in 0..=26 {
        let e = entrance_at(i as f32 / 100.0, 0);
        assert!(e.opacity >= prev && (0.0..=1.0).contains(&e.opacity));
        assert!((0.0..=theme::RISE_PX).contains(&e.offset_y));
        prev = e.opacity;
    }
}

/// GMO12 (G4): an element drawn every frame (refresh polls) never re-enters; one that was not
/// drawn in the previous pass (tab change) enters again.
#[test]
fn test_gmo12_entrance_only_on_first_render_after_appearing() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let id = Id::new("gmo12");
    let mut e = Entrance::REST;
    let _ = pass(&ctx, 1.0 / 60.0, |ui| e = motion::entrance(ui.ctx(), id, 0));
    assert!(e.opacity < 1.0, "first appearance enters");
    for _ in 0..40 {
        let _ = pass(&ctx, 1.0 / 60.0, |ui| e = motion::entrance(ui.ctx(), id, 0));
    }
    assert_eq!(e, Entrance::REST);
    for _ in 0..40 {
        let _ = pass(&ctx, 1.0 / 60.0, |ui| e = motion::entrance(ui.ctx(), id, 0));
        assert_eq!(e, Entrance::REST, "a refresh never replays the entrance");
    }
    let _ = pass(&ctx, 1.0 / 60.0, |_| {});
    let _ = pass(&ctx, 1.0 / 60.0, |ui| e = motion::entrance(ui.ctx(), id, 0));
    assert!(
        e.opacity < 1.0,
        "re-appearing after a frame away enters again"
    );
}

/// GMO12 (G4): a value fades in only when its key changes; first sight is at rest.
#[test]
fn test_gmo12_fade_in_on_change() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let id = Id::new("gmo12f");
    let mut o = 0.0;
    let _ = pass(&ctx, 1.0 / 60.0, |ui| {
        o = motion::fade_in_on_change(ui.ctx(), id, 1, theme::T_SHORT)
    });
    assert_eq!(o, 1.0);
    let _ = pass(&ctx, 1.0 / 60.0, |ui| {
        o = motion::fade_in_on_change(ui.ctx(), id, 2, theme::T_SHORT)
    });
    assert!(o < 1.0);
    for _ in 0..20 {
        let _ = pass(&ctx, 1.0 / 60.0, |ui| {
            o = motion::fade_in_on_change(ui.ctx(), id, 2, theme::T_SHORT)
        });
    }
    assert_eq!(o, 1.0);
}

// ---------------------------------------------------------------------------------------------
// GMO13 — star turn
// ---------------------------------------------------------------------------------------------

/// GMO13 (G8): the star at angle 0 paints exactly the unrotated mark; a turned star paints a
/// rotated mesh; the star tile springs to 45° on a confirmed paused daemon.
#[test]
fn test_gmo13_star_turn_and_rotation_at_rest() {
    let ctx = ctx_with(MotionPrefs::FULL);
    let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(200.0, 200.0));
    let plain = pass_shapes(&ctx, |ui| {
        soos_gui::brand::paint_star(ui, rect, theme::PINK)
    });
    let zero = pass_shapes(&ctx, |ui| {
        soos_gui::brand::paint_star_rotated(ui, rect, theme::PINK, 0.0);
    });
    assert_eq!(plain, zero, "angle 0 is exactly paint_star");
    let turned = pass_shapes(&ctx, |ui| {
        soos_gui::brand::paint_star_rotated(ui, rect, theme::PINK, 0.5);
    });
    assert!(turned
        .iter()
        .any(|c| matches!(c.shape, egui::Shape::Mesh(_))));
    assert_ne!(plain, turned);

    let _ = pass(&ctx, 1.0 / 60.0, |ui| {
        soos_gui::widgets::star_tile_turned(ui, rect, false)
    });
    assert_eq!(soos_gui::widgets::star_turn_degrees(&ctx), 0.0);
    for _ in 0..40 {
        let _ = pass(&ctx, 1.0 / 60.0, |ui| {
            soos_gui::widgets::star_tile_turned(ui, rect, true)
        });
    }
    assert_eq!(
        soos_gui::widgets::star_turn_degrees(&ctx),
        theme::STAR_TURN_DEG
    );
}

// ---------------------------------------------------------------------------------------------
// GMO14 — wiring
// ---------------------------------------------------------------------------------------------

/// GMO14: the portal is only read when neither the switch nor the variable decides; a pending
/// portal is reduced; the answer is installed into the context.
#[test]
fn test_gmo14_motion_control_reads_the_portal_only_when_needed() {
    let spawned = AtomicUsize::new(0);
    let spawn = || {
        spawned.fetch_add(1, Ordering::SeqCst);
        PortalReader::spawn(SilentBus, Duration::from_secs(30))
    };
    let mut cli = MotionControl::from_sources(true, None, spawn);
    assert_eq!(cli.current(Instant::now()), (MotionPrefs::REDUCED, false));
    let mut env_full = MotionControl::from_sources(false, Some(OsStr::new("0")), spawn);
    assert_eq!(env_full.current(Instant::now()), (MotionPrefs::FULL, false));
    let mut env_reduced = MotionControl::from_sources(false, Some(OsStr::new("1")), spawn);
    assert_eq!(
        env_reduced.current(Instant::now()),
        (MotionPrefs::REDUCED, false)
    );
    assert_eq!(
        spawned.load(Ordering::SeqCst),
        0,
        "no D-Bus read when overridden"
    );

    let mut portal = MotionControl::from_sources(false, Some(OsStr::new("maybe")), spawn);
    assert_eq!(spawned.load(Ordering::SeqCst), 1);
    assert_eq!(
        portal.current(Instant::now()),
        (MotionPrefs::REDUCED, true),
        "pending = reduced"
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let mut answered = MotionControl::from_sources(false, None, || {
        PortalReader::spawn(
            ScriptedBus {
                readings: PortalReadings {
                    reduced_motion: Some(SettingValue::U32(0)),
                    enable_animations: None,
                },
                delay: Duration::ZERO,
                calls: Arc::clone(&calls),
            },
            Duration::from_secs(30),
        )
    });
    let ctx = egui::Context::default();
    let start = Instant::now();
    loop {
        answered.apply(&ctx);
        if motion::prefs(&ctx) == MotionPrefs::FULL || start.elapsed() > Duration::from_secs(10) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(motion::prefs(&ctx), MotionPrefs::FULL);
    assert_eq!(ctx.global_style().animation_time, theme::T_SHORT);
    let mut fixed = MotionControl::fixed(MotionPrefs::REDUCED);
    assert_eq!(fixed.current(Instant::now()), (MotionPrefs::REDUCED, false));
}

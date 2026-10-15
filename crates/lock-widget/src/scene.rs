//! Pure animation state of the widget (no clock read, no I/O).
//!
//! Motion (ADR 2026-10-13 "Motion System", `Docs/UI_MOTION.md`; spec
//! `AI/architect_spec_lockscreen_adapters.md` §2.2, §9 and §10):
//!
//! - the label of a new state is in the first frame after [`Scene::update`] (no text fade);
//! - the ring colour cross-fades over `T_SHORT_MS` with the standard easing;
//! - while scanning, the pink star turns one quarter turn per `T_PENDING_MS` (standard easing per
//!   quarter), never later than `PENDING_LOOP_MAX_MS` after the scan started; the star repeats
//!   every 180°, so a turn only ever stops on a half-turn boundary and a new scan arriving
//!   mid-turn keeps the phase;
//! - a new Not recognized (new sequence) shakes the widget once over `T_MEDIUM_MS`;
//! - reduced motion: instant colours, no rotation, no shake.

use soos_lockstatus::format::text;
use soos_lockstatus::motion::{
    cubic_bezier, MotionPrefs, EASE_STANDARD, PENDING_LOOP_MAX_MS, SLIDE_PX, T_MEDIUM_MS,
    T_PENDING_MS, T_SHORT_MS,
};
use soos_lockstatus::view::{LockView, Presented};

use crate::palette::{self, Rgb};

/// A half turn of the star (its symmetry period), in milliseconds.
const HALF_TURN_MS: u64 = 2 * T_PENDING_MS;

/// What one frame draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSpec {
    /// Ring colour.
    pub ring: Rgb,
    /// Star angle in degrees, in `[0, 180)` (the star repeats every 180°).
    pub star_angle_deg: f32,
    /// Horizontal shake offset in pixels, in `[-SLIDE_PX, SLIDE_PX]`.
    pub offset_x: f32,
    /// Label words.
    pub label: &'static str,
    /// Whether another frame is needed.
    pub animating: bool,
}

/// Animation state machine fed with polled views and explicit monotonic milliseconds.
#[derive(Debug, Clone)]
pub struct Scene {
    prefs: MotionPrefs,
    presented: Presented,
    sequence: u64,
    entered_ms: u64,
    from_ring: Rgb,
    /// Start of the current star turn and the instant it stops (a half-turn boundary).
    turn: Option<(u64, u64)>,
    shake_from_ms: Option<u64>,
    last_shaken_sequence: Option<u64>,
    /// Last sequence the daemon answered (a lower one starts a new epoch: daemon restart).
    epoch_sequence: u64,
    /// Sequence and first-seen instant of the last Scanning view: a scan seen again after a
    /// transient error keeps its original start for the loop limit.
    scan_start: Option<(u64, u64)>,
}

/// Elapsed milliseconds as a fraction of `span` (saturating; out-of-order clocks read 0).
#[allow(
    clippy::cast_precision_loss,
    reason = "both operands are clamped below `span`, a few thousand milliseconds"
)]
fn fraction(elapsed: u64, span: u64) -> f32 {
    if span == 0 {
        return 1.0;
    }
    (elapsed.min(span) as f32) / (span as f32)
}

/// `anchor + ceil((at − anchor) / HALF_TURN_MS) · HALF_TURN_MS` (saturating).
fn up_to_half_turn(anchor: u64, at: u64) -> u64 {
    let elapsed = at.saturating_sub(anchor);
    let turns = elapsed.div_ceil(HALF_TURN_MS);
    anchor.saturating_add(turns.saturating_mul(HALF_TURN_MS))
}

/// `anchor + floor((at − anchor) / HALF_TURN_MS) · HALF_TURN_MS` (saturating).
fn down_to_half_turn(anchor: u64, at: u64) -> u64 {
    let turns = at.saturating_sub(anchor) / HALF_TURN_MS;
    anchor.saturating_add(turns.saturating_mul(HALF_TURN_MS))
}

impl Scene {
    /// An idle, settled scene.
    #[must_use]
    pub fn new(prefs: MotionPrefs, now_ms: u64) -> Self {
        Self {
            prefs,
            presented: Presented::Idle,
            sequence: 0,
            entered_ms: now_ms,
            from_ring: palette::ring_color(Presented::Idle),
            turn: None,
            shake_from_ms: None,
            last_shaken_sequence: None,
            epoch_sequence: 0,
            scan_start: None,
        }
    }

    /// Applies a polled view. Every idle view (unavailable included) is the same scene state, so
    /// a transport error never draws differently from Idle; an unchanged view is a no-op.
    pub fn update(&mut self, view: LockView, now_ms: u64) {
        if view.available() && view.sequence != 0 {
            // The daemon restarts its per-UID counter when it restarts: a failure numbered like
            // the last shaken one in the old epoch is a new failure. Sequence 0 is also the
            // daemon's neutral answer (clock error, no local seat session, session check
            // timeout), so it never starts an epoch; the restarted daemon's first scan (1) does.
            if view.sequence < self.epoch_sequence {
                self.last_shaken_sequence = None;
                self.scan_start = None;
            }
            self.epoch_sequence = view.sequence;
        }
        let presented = view.presented();
        let sequence = if presented == Presented::Idle {
            0
        } else {
            view.sequence
        };
        if presented == self.presented && sequence == self.sequence {
            return;
        }
        let shown_ring = self.ring_at(now_ms);
        let leaving_scan = self.presented == Presented::Scanning;
        self.from_ring = shown_ring;
        self.presented = presented;
        self.sequence = sequence;
        self.entered_ms = now_ms;
        if self.prefs.reduced {
            return;
        }
        match presented {
            Presented::Scanning => {
                let scan_start = match self.scan_start {
                    Some((scan, start)) if scan == sequence => start,
                    _ => now_ms,
                };
                self.scan_start = Some((sequence, scan_start));
                let anchor = match self.turn {
                    Some((anchor, stop)) if anchor <= now_ms && now_ms < stop => anchor,
                    _ => now_ms,
                };
                // Rounded down: a scan never turns the star later than its own start plus
                // PENDING_LOOP_MAX_MS, even when it joins a running turn or is seen again after a
                // transient error (the loop limit is a multiple of a half turn, so a fresh scan
                // turns exactly that long).
                let wanted =
                    down_to_half_turn(anchor, scan_start.saturating_add(PENDING_LOOP_MAX_MS));
                let stop = match self.turn {
                    Some((old_anchor, old_stop)) if old_anchor == anchor => old_stop.max(wanted),
                    _ => wanted,
                };
                self.turn = Some((anchor, stop));
            }
            Presented::Idle | Presented::NotRecognized => {
                if leaving_scan {
                    if let Some((anchor, stop)) = self.turn {
                        self.turn = Some((anchor, stop.min(up_to_half_turn(anchor, now_ms))));
                    }
                }
                if presented == Presented::NotRecognized
                    && self.last_shaken_sequence != Some(sequence)
                {
                    self.shake_from_ms = Some(now_ms);
                    self.last_shaken_sequence = Some(sequence);
                }
            }
        }
    }

    fn target_ring(&self) -> Rgb {
        palette::ring_color(self.presented)
    }

    fn fading(&self, now_ms: u64) -> bool {
        !self.prefs.reduced
            && self.from_ring != self.target_ring()
            && now_ms.saturating_sub(self.entered_ms) < T_SHORT_MS
    }

    fn ring_at(&self, now_ms: u64) -> Rgb {
        if !self.fading(now_ms) {
            return self.target_ring();
        }
        let t = fraction(now_ms.saturating_sub(self.entered_ms), T_SHORT_MS);
        palette::mix(
            self.from_ring,
            self.target_ring(),
            cubic_bezier(EASE_STANDARD, t),
        )
    }

    fn turning(&self, now_ms: u64) -> Option<u64> {
        if self.prefs.reduced {
            return None;
        }
        let (anchor, stop) = self.turn?;
        (anchor <= now_ms && now_ms < stop).then(|| now_ms.saturating_sub(anchor))
    }

    fn star_angle(&self, now_ms: u64) -> f32 {
        let Some(elapsed) = self.turning(now_ms) else {
            return 0.0;
        };
        let quarters = elapsed / T_PENDING_MS;
        let within = fraction(elapsed % T_PENDING_MS, T_PENDING_MS);
        let base = if quarters.is_multiple_of(2) {
            0.0
        } else {
            90.0
        };
        let angle = base + 90.0 * cubic_bezier(EASE_STANDARD, within);
        if (0.0..180.0).contains(&angle) {
            angle
        } else {
            0.0
        }
    }

    fn shaking(&self, now_ms: u64) -> Option<f32> {
        if self.prefs.reduced || self.presented != Presented::NotRecognized {
            return None;
        }
        let from = self.shake_from_ms?;
        let elapsed = now_ms.checked_sub(from)?;
        (elapsed < T_MEDIUM_MS).then(|| fraction(elapsed, T_MEDIUM_MS))
    }

    /// The frame at `now_ms`.
    #[must_use]
    pub fn frame(&self, now_ms: u64) -> FrameSpec {
        let shake = self.shaking(now_ms);
        let offset_x = shake.map_or(0.0, |t| {
            (SLIDE_PX * (4.0 * std::f32::consts::PI * t).sin() * (1.0 - t))
                .clamp(-SLIDE_PX, SLIDE_PX)
        });
        FrameSpec {
            ring: self.ring_at(now_ms),
            star_angle_deg: self.star_angle(now_ms),
            offset_x,
            label: text(self.presented),
            animating: self.fading(now_ms) || self.turning(now_ms).is_some() || shake.is_some(),
        }
    }
}

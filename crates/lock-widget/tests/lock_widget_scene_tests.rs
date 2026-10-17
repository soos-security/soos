//! Contract tests of the widget animation state machine (GitHub #401; matrix LKA3, LKA8;
//! spec §2.2, §9 R2/R9/R10, §10 R2-1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_lock_widget::palette::{self, ring_color, RING_IDLE, RING_NOT_RECOGNIZED, RING_SCANNING};
use soos_lock_widget::scene::Scene;
use soos_lockstatus::format::{NOT_RECOGNIZED_TEXT, SCANNING_TEXT};
use soos_lockstatus::motion::{
    cubic_bezier, MotionPrefs, AMBIENT_REPAINT_MS, EASE_STANDARD, PENDING_LOOP_MAX_MS, SLIDE_PX,
    T_MEDIUM_MS, T_PENDING_MS, T_SHORT_MS,
};
use soos_lockstatus::view::{LockView, Presented, ViewState};

const FULL: MotionPrefs = MotionPrefs { reduced: false };
const REDUCED: MotionPrefs = MotionPrefs { reduced: true };

fn v(state: ViewState, sequence: u64) -> LockView {
    LockView { state, sequence }
}

/// Circular distance on the star's [0, 180) angle domain.
fn circ(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(180.0);
    d.min(180.0 - d)
}

#[test]
fn test_lka8_new_scene_is_idle_and_settled() {
    let s = Scene::new(FULL, 1000);
    let f = s.frame(1000);
    assert_eq!(f.ring, RING_IDLE);
    assert_eq!(f.star_angle_deg, 0.0);
    assert_eq!(f.offset_x, 0.0);
    assert_eq!(f.label, "");
    assert!(!f.animating, "idle draws no further frame");
}

#[test]
fn test_lka8_ring_roles() {
    assert_eq!(ring_color(Presented::Idle), RING_IDLE);
    assert_eq!(ring_color(Presented::Scanning), RING_SCANNING);
    assert_eq!(ring_color(Presented::NotRecognized), RING_NOT_RECOGNIZED);
}

#[test]
fn test_lka8_label_is_in_the_first_frame() {
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::Scanning, 1), 100);
    assert_eq!(s.frame(100).label, SCANNING_TEXT);
    s.update(v(ViewState::NotRecognized, 2), 3340);
    assert_eq!(s.frame(3340).label, NOT_RECOGNIZED_TEXT);
    s.update(v(ViewState::Idle, 2), 6340);
    assert_eq!(s.frame(6340).label, "");
}

#[test]
fn test_lka8_ring_cross_fades_over_t_short_with_standard_easing() {
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::Scanning, 1), 1000);
    assert_eq!(s.frame(1000).ring, RING_IDLE);
    assert!(s.frame(1000).animating);
    let half = 1000 + T_SHORT_MS / 2;
    let t = cubic_bezier(EASE_STANDARD, 0.5);
    assert_eq!(
        s.frame(half).ring,
        palette::mix(RING_IDLE, RING_SCANNING, t)
    );
    assert_eq!(s.frame(1000 + T_SHORT_MS).ring, RING_SCANNING);
    assert_eq!(s.frame(1000 + T_SHORT_MS + 500).ring, RING_SCANNING);
}

#[test]
fn test_lka8_pending_star_turns_quarter_per_token_with_easing() {
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::Scanning, 1), 0);
    assert_eq!(s.frame(0).star_angle_deg, 0.0);
    let q = T_PENDING_MS;
    let mid = 90.0 * cubic_bezier(EASE_STANDARD, 0.5);
    assert!((s.frame(q / 2).star_angle_deg - mid).abs() < 0.01);
    assert!((s.frame(q).star_angle_deg - 90.0).abs() < 0.01);
    assert!((s.frame(q + q / 2).star_angle_deg - (90.0 + mid)).abs() < 0.01);
    // A half turn is a full symmetry: 180° reads 0°.
    assert!(circ(s.frame(2 * q).star_angle_deg, 0.0) < 0.01);
    for t in (0..PENDING_LOOP_MAX_MS).step_by(37) {
        let a = s.frame(t).star_angle_deg;
        assert!((0.0..180.0).contains(&a), "{t}: {a}");
        assert!(s.frame(t).animating);
    }
}

#[test]
fn test_lka8_pending_loop_is_bounded_per_scan() {
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::Scanning, 1), 0);
    assert_eq!(PENDING_LOOP_MAX_MS, 4800);
    assert_eq!(PENDING_LOOP_MAX_MS % (2 * T_PENDING_MS), 0);
    let f = s.frame(PENDING_LOOP_MAX_MS);
    assert_eq!(f.star_angle_deg, 0.0);
    assert!(
        !f.animating,
        "the star rests after 4.8 s even while scanning"
    );
    assert_eq!(f.label, SCANNING_TEXT, "the words still say Scanning");
    assert_eq!(s.frame(9000).star_angle_deg, 0.0);
}

#[test]
fn test_lka8_leaving_scanning_finishes_the_half_turn() {
    let q = T_PENDING_MS;
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::Scanning, 1), 0);
    // Leave at 1.5 quarters: the turn continues to 2 quarters (180° = 0°), then rests.
    s.update(v(ViewState::NotRecognized, 2), q + q / 2);
    let a = s.frame(q + q / 2 + 100).star_angle_deg;
    assert!(a > 90.0 && a < 180.0, "{a}");
    assert!(circ(s.frame(2 * q - 1).star_angle_deg, 0.0) < 1.0);
    assert_eq!(s.frame(2 * q).star_angle_deg, 0.0);
    assert_eq!(s.frame(2 * q + 500).star_angle_deg, 0.0);
    // Leaving exactly on a half-turn boundary stops at once.
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::Scanning, 1), 0);
    s.update(v(ViewState::Idle, 2), 2 * q);
    assert_eq!(s.frame(2 * q).star_angle_deg, 0.0);
}

#[test]
fn test_lka8_presence_cadence_keeps_the_star_continuous() {
    // Presence failing repeatedly: Scanning at k·4240 ms, NotRecognized revealed 3240 ms later,
    // the next scan noticed at the next poll (sometimes 500 ms late).
    let mut s = Scene::new(FULL, 0);
    let mut events = Vec::new();
    let mut start = 0u64;
    for k in 0..7u64 {
        let seen = start + if k % 2 == 0 { 0 } else { 500 };
        events.push((seen, v(ViewState::Scanning, 2 * k + 1)));
        events.push((start + 3240, v(ViewState::NotRecognized, 2 * k + 2)));
        start += 4240;
    }
    let end = start + 6000;
    let mut next = 0;
    let mut previous = s.frame(0).star_angle_deg;
    // Largest legitimate per-frame increment: the steepest 33 ms step of the eased quarter.
    let mut max_step = 0.0_f32;
    for i in 0..T_PENDING_MS {
        let a = 90.0 * cubic_bezier(EASE_STANDARD, i as f32 / T_PENDING_MS as f32);
        let b = 90.0
            * cubic_bezier(
                EASE_STANDARD,
                ((i + AMBIENT_REPAINT_MS) as f32 / T_PENDING_MS as f32).min(1.0),
            );
        max_step = max_step.max(b - a);
    }
    let mut t = 0;
    while t <= end {
        while next < events.len() && events[next].0 <= t {
            s.update(events[next].1, events[next].0);
            next += 1;
        }
        let a = s.frame(t).star_angle_deg;
        assert!(
            circ(a, previous) <= max_step + 0.01,
            "jump at {t}: {previous} -> {a} (max {max_step})"
        );
        previous = a;
        t += AMBIENT_REPAINT_MS;
    }
    let rest = s.frame(end);
    assert_eq!(rest.star_angle_deg, 0.0);
    assert!(!rest.animating);
}

#[test]
fn test_lka8_not_recognized_shakes_once_per_sequence() {
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::NotRecognized, 2), 1000);
    let mut peak = 0.0_f32;
    for t in 1000..1000 + T_MEDIUM_MS {
        let x = s.frame(t).offset_x;
        assert!(x.abs() <= SLIDE_PX, "{t}: {x}");
        peak = peak.max(x.abs());
    }
    assert!(peak > SLIDE_PX / 2.0, "a visible shake: {peak}");
    assert_eq!(s.frame(1000).offset_x, 0.0);
    assert_eq!(s.frame(1000 + T_MEDIUM_MS).offset_x, 0.0);
    assert_eq!(s.frame(5000).offset_x, 0.0);
    // A transient poll error (unavailable reads idle) and the same NotRecognized again: no replay.
    s.update(LockView::UNAVAILABLE, 2000);
    s.update(v(ViewState::NotRecognized, 2), 2500);
    for t in 2500..2500 + T_MEDIUM_MS {
        assert_eq!(s.frame(t).offset_x, 0.0, "{t}");
    }
    // A new failure (new sequence) shakes again.
    s.update(v(ViewState::NotRecognized, 4), 9000);
    let replay = (9000..9000 + T_MEDIUM_MS).any(|t| s.frame(t).offset_x.abs() > 1.0);
    assert!(replay);
}

#[test]
fn test_lka8_reduced_motion_settles_at_once() {
    let mut s = Scene::new(REDUCED, 0);
    s.update(v(ViewState::Scanning, 1), 100);
    let f = s.frame(100);
    assert_eq!(f.ring, RING_SCANNING, "colours change instantly");
    assert_eq!(f.star_angle_deg, 0.0);
    assert!(!f.animating);
    assert_eq!(f.label, SCANNING_TEXT);
    assert_eq!(s.frame(1300).star_angle_deg, 0.0);
    s.update(v(ViewState::NotRecognized, 2), 3340);
    for t in 3340..3340 + T_MEDIUM_MS {
        let f = s.frame(t);
        assert_eq!(f.offset_x, 0.0);
        assert_eq!(f.ring, RING_NOT_RECOGNIZED);
        assert!(!f.animating);
    }
}

#[test]
fn test_lka3_unavailable_draws_exactly_like_idle() {
    let mut a = Scene::new(FULL, 0);
    let mut b = Scene::new(FULL, 0);
    for (t, view) in [
        (100, v(ViewState::Scanning, 1)),
        (3340, v(ViewState::NotRecognized, 2)),
    ] {
        a.update(view, t);
        b.update(view, t);
    }
    a.update(LockView::UNAVAILABLE, 7000);
    b.update(v(ViewState::Idle, 2), 7000);
    for t in (7000..9000).step_by(11) {
        assert_eq!(a.frame(t), b.frame(t), "{t}");
    }
    // A different idle sequence is no new state either.
    b.update(v(ViewState::Idle, 9), 9100);
    a.update(v(ViewState::Idle, 3), 9100);
    assert_eq!(a.frame(9100), b.frame(9100));
    assert!(!a.frame(9100).animating);
}

#[test]
fn test_lka8_unchanged_view_is_a_no_op_and_clock_skew_is_safe() {
    let mut s = Scene::new(FULL, 5000);
    s.update(v(ViewState::Scanning, 1), 5000);
    let before = s.frame(5600);
    s.update(v(ViewState::Scanning, 1), 5500);
    assert_eq!(s.frame(5600), before);
    // Frames asked for before the last update never panic and stay in range.
    let f = s.frame(0);
    assert!((0.0..180.0).contains(&f.star_angle_deg));
    assert!(f.offset_x.abs() <= SLIDE_PX);
    let f = s.frame(u64::MAX);
    assert!((0.0..180.0).contains(&f.star_angle_deg));
}

#[test]
fn test_lka8_idle_after_failure_settles() {
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::NotRecognized, 2), 0);
    s.update(v(ViewState::Idle, 2), 3000);
    assert!(s.frame(3000).animating, "the ring fades back");
    let f = s.frame(3000 + T_SHORT_MS);
    assert_eq!(f.ring, RING_IDLE);
    assert!(!f.animating);
}

#[test]
fn test_lka8_scan_joining_a_running_turn_stops_within_the_loop_limit() {
    // Presence cadence: the second scan starts at 4740 ms while the first turn still runs.
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::Scanning, 1), 0);
    s.update(v(ViewState::NotRecognized, 2), 3240);
    s.update(v(ViewState::Scanning, 3), 4740);
    assert!(
        s.frame(4740).animating,
        "the turn continues into the joining scan"
    );
    let limit = 4740 + PENDING_LOOP_MAX_MS;
    for t in (limit..limit + 5000).step_by(17) {
        let f = s.frame(t);
        assert_eq!(f.star_angle_deg, 0.0, "{t}");
        assert!(!f.animating, "{t}");
        assert_eq!(f.label, SCANNING_TEXT);
    }
}

#[test]
fn test_lka8_daemon_restart_epoch_shakes_a_reused_sequence() {
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::NotRecognized, 2), 0);
    assert!((0..T_MEDIUM_MS).any(|t| s.frame(t).offset_x.abs() > 1.0));
    // The daemon restarts: its counter starts again at 0, then the same numbers recur.
    s.update(v(ViewState::Idle, 0), 5000);
    s.update(v(ViewState::Scanning, 1), 6000);
    s.update(v(ViewState::NotRecognized, 2), 9240);
    let shaken = (9240..9240 + T_MEDIUM_MS).any(|t| s.frame(t).offset_x.abs() > 1.0);
    assert!(shaken, "a failure of the new epoch shakes");
}

#[test]
fn test_lka8_scan_seen_again_after_a_transient_error_keeps_its_start() {
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::Scanning, 1), 0);
    // A transient poll error reads Idle, then the same scan (same sequence) is seen again.
    s.update(LockView::UNAVAILABLE, 4000);
    s.update(v(ViewState::Scanning, 1), 4500);
    for t in (PENDING_LOOP_MAX_MS..PENDING_LOOP_MAX_MS + 6000).step_by(13) {
        let f = s.frame(t);
        assert_eq!(f.star_angle_deg, 0.0, "{t}");
        assert!(!f.animating, "{t}");
    }
    // A new scan (new sequence) turns again.
    s.update(v(ViewState::NotRecognized, 2), 12_000);
    s.update(v(ViewState::Scanning, 3), 13_000);
    assert!(s.frame(13_600).star_angle_deg > 0.0);
}

#[test]
fn test_lka8_neutral_idle_answer_is_not_a_new_epoch() {
    // The daemon's neutral answer (Idle, sequence 0) between two polls of the same failure: one
    // shake only.
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::NotRecognized, 4), 0);
    s.update(v(ViewState::Idle, 0), 500);
    s.update(v(ViewState::NotRecognized, 4), 1000);
    for t in 1000..1000 + T_MEDIUM_MS {
        assert_eq!(s.frame(t).offset_x, 0.0, "{t}");
    }
    // Between two polls of the same scan: the star still stops by the scan start plus 4.8 s.
    let mut s = Scene::new(FULL, 0);
    s.update(v(ViewState::Scanning, 5), 0);
    s.update(v(ViewState::Idle, 0), 4000);
    s.update(v(ViewState::Scanning, 5), 4500);
    for t in (PENDING_LOOP_MAX_MS..PENDING_LOOP_MAX_MS + 6000).step_by(13) {
        assert_eq!(s.frame(t).star_angle_deg, 0.0, "{t}");
        assert!(!s.frame(t).animating, "{t}");
    }
}

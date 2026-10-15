//! Contract tests of the `soos-lockstatus` presentation policy, formats and motion resolution
//! (GitHub #401; matrix LKA3, LKA4, LKA5, LKA7; spec `AI/architect_spec_lockscreen_adapters.md`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::ffi::OsStr;

use soos_lockstatus::format::{
    glyph, json, label, once_output, text, OutputFormat, IDLE_GLYPH, IDLE_TEXT,
    NOT_RECOGNIZED_GLYPH, NOT_RECOGNIZED_TEXT, SCANNING_GLYPHS, SCANNING_STATIC_GLYPH,
    SCANNING_TEXT,
};
use soos_lockstatus::motion::{
    cubic_bezier, resolve_motion_prefs, MotionPrefs, EASE_STANDARD, T_PENDING_MS,
};
use soos_lockstatus::view::{LockView, Presented, ViewState};
use soos_lockstatus::MAX_JSON_LINE_BYTES;
use soos_protocol::{LockStateResponse, LockWidgetState, CURRENT_VERSION};

const FULL: MotionPrefs = MotionPrefs { reduced: false };
const REDUCED: MotionPrefs = MotionPrefs { reduced: true };

fn view(state: ViewState, sequence: u64) -> LockView {
    LockView { state, sequence }
}

#[test]
fn test_lka3_from_response_maps_each_wire_state_one_to_one() {
    for (wire, expected) in [
        (LockWidgetState::Idle, ViewState::Idle),
        (LockWidgetState::Scanning, ViewState::Scanning),
        (LockWidgetState::NotRecognized, ViewState::NotRecognized),
    ] {
        let v = LockView::from_response(&LockStateResponse {
            version: CURRENT_VERSION,
            state: wire,
            sequence: 7,
        });
        assert_eq!(v, view(expected, 7));
        assert!(v.available());
    }
}

#[test]
fn test_lka3_other_version_is_unavailable() {
    for version in [0, CURRENT_VERSION + 1, u8::MAX] {
        let v = LockView::from_response(&LockStateResponse {
            version,
            state: LockWidgetState::Scanning,
            sequence: 9,
        });
        assert_eq!(v, LockView::UNAVAILABLE);
        assert!(!v.available());
    }
}

#[test]
fn test_lka3_only_three_presented_states_and_unavailable_is_idle() {
    assert_eq!(LockView::UNAVAILABLE.presented(), Presented::Idle);
    assert_eq!(view(ViewState::Idle, 4).presented(), Presented::Idle);
    assert_eq!(
        view(ViewState::Scanning, 5).presented(),
        Presented::Scanning
    );
    assert_eq!(
        view(ViewState::NotRecognized, 6).presented(),
        Presented::NotRecognized
    );
}

#[test]
fn test_lka3_unavailable_renders_byte_identical_to_idle_in_once_formats() {
    for format in [OutputFormat::Text, OutputFormat::Glyph, OutputFormat::Label] {
        for prefs in [FULL, REDUCED] {
            for now in [0, 1, 1199, 1200, 5000, u64::MAX] {
                assert_eq!(
                    once_output(format, &LockView::UNAVAILABLE, now, prefs),
                    once_output(format, &view(ViewState::Idle, 12), now, prefs),
                    "{format:?} {prefs:?} {now}"
                );
            }
        }
    }
}

#[test]
fn test_lka4_text_words() {
    assert_eq!(text(Presented::Idle), "");
    assert_eq!(IDLE_TEXT, "");
    assert_eq!(text(Presented::Scanning), "Scanning");
    assert_eq!(SCANNING_TEXT, "Scanning");
    assert_eq!(text(Presented::NotRecognized), "Not recognized");
    assert_eq!(NOT_RECOGNIZED_TEXT, "Not recognized");
}

#[test]
fn test_lka4_glyphs_and_scanning_quarter_step_per_pending_token() {
    assert_eq!(IDLE_GLYPH, "\u{2726}");
    assert_eq!(
        SCANNING_GLYPHS,
        ["\u{25DC}", "\u{25DD}", "\u{25DE}", "\u{25DF}"]
    );
    assert_eq!(SCANNING_STATIC_GLYPH, "\u{25CC}");
    assert_eq!(NOT_RECOGNIZED_GLYPH, "\u{2715}");
    assert_eq!(T_PENDING_MS, 1200);

    assert_eq!(glyph(Presented::Idle, 0, FULL), IDLE_GLYPH);
    assert_eq!(
        glyph(Presented::NotRecognized, 0, FULL),
        NOT_RECOGNIZED_GLYPH
    );
    for (now, index) in [
        (0, 0),
        (1199, 0),
        (1200, 1),
        (2400, 2),
        (3600, 3),
        (4800, 0),
        (4800 * 1000 + 1300, 1),
    ] {
        assert_eq!(
            glyph(Presented::Scanning, now, FULL),
            SCANNING_GLYPHS[index],
            "now {now}"
        );
    }
    let last = u64::MAX;
    assert!(SCANNING_GLYPHS.contains(&glyph(Presented::Scanning, last, FULL)));
}

#[test]
fn test_lka4_reduced_motion_glyph_is_static() {
    for now in [0, 1200, 2400, 3600, 99_999] {
        assert_eq!(
            glyph(Presented::Scanning, now, REDUCED),
            SCANNING_STATIC_GLYPH
        );
        assert_eq!(glyph(Presented::Idle, now, REDUCED), IDLE_GLYPH);
        assert_eq!(
            glyph(Presented::NotRecognized, now, REDUCED),
            NOT_RECOGNIZED_GLYPH
        );
    }
}

#[test]
fn test_lka4_label_joins_glyph_and_words() {
    assert_eq!(label(Presented::Idle, 0, FULL), IDLE_GLYPH);
    assert_eq!(
        label(Presented::Scanning, 1200, FULL),
        format!("{}  Scanning", SCANNING_GLYPHS[1])
    );
    assert_eq!(
        label(Presented::NotRecognized, 0, REDUCED),
        format!("{NOT_RECOGNIZED_GLYPH}  Not recognized")
    );
}

#[test]
fn test_lka4_once_output_has_no_newline_and_is_pango_safe() {
    for state in [
        ViewState::Unavailable,
        ViewState::Idle,
        ViewState::Scanning,
        ViewState::NotRecognized,
    ] {
        for format in [OutputFormat::Text, OutputFormat::Glyph, OutputFormat::Label] {
            let out = once_output(format, &view(state, 3), 2400, FULL);
            assert!(!out.contains('\n'), "{out:?}");
            assert!(!out.contains(['<', '>', '&']), "{out:?}");
        }
    }
    assert_eq!(
        once_output(OutputFormat::Text, &view(ViewState::Scanning, 3), 0, FULL),
        "Scanning"
    );
    assert_eq!(
        once_output(
            OutputFormat::Glyph,
            &view(ViewState::NotRecognized, 4),
            0,
            FULL
        ),
        NOT_RECOGNIZED_GLYPH
    );
    assert_eq!(
        once_output(
            OutputFormat::Label,
            &view(ViewState::Scanning, 3),
            3600,
            FULL
        ),
        format!("{}  Scanning", SCANNING_GLYPHS[3])
    );
}

#[test]
fn test_lka5_json_exact_lines() {
    assert_eq!(
        json(&LockView::UNAVAILABLE),
        r#"{"state":"idle","available":false,"sequence":0}"#
    );
    assert_eq!(
        json(&view(ViewState::Idle, 2)),
        r#"{"state":"idle","available":true,"sequence":2}"#
    );
    assert_eq!(
        json(&view(ViewState::Scanning, 3)),
        r#"{"state":"scanning","available":true,"sequence":3}"#
    );
    assert_eq!(
        json(&view(ViewState::NotRecognized, 4)),
        r#"{"state":"not_recognized","available":true,"sequence":4}"#
    );
}

#[test]
fn test_lka5_json_key_set_and_line_bound() {
    for state in [
        ViewState::Unavailable,
        ViewState::Idle,
        ViewState::Scanning,
        ViewState::NotRecognized,
    ] {
        for sequence in [0, 1, u64::MAX] {
            let line = json(&view(state, sequence));
            assert!(
                line.len() < MAX_JSON_LINE_BYTES,
                "{line} (newline included)"
            );
            assert!(line.starts_with('{') && line.ends_with('}'));
            let keys: Vec<&str> = line
                .trim_matches(['{', '}'])
                .split(',')
                .map(|kv| kv.split(':').next().unwrap().trim_matches('"'))
                .collect();
            assert_eq!(keys, ["state", "available", "sequence"]);
            for forbidden in [
                "score",
                "reason",
                "frame",
                "embedding",
                "template",
                "session",
                "time",
            ] {
                assert!(!line.contains(forbidden), "{line}");
            }
        }
    }
    let longest = json(&view(ViewState::NotRecognized, u64::MAX));
    assert_eq!(longest.len() + 1, 76);
}

#[test]
fn test_lka7_reduced_motion_resolution_order() {
    assert_eq!(resolve_motion_prefs(false, None), FULL);
    assert_eq!(resolve_motion_prefs(true, None), REDUCED);
    assert_eq!(resolve_motion_prefs(true, Some(OsStr::new("0"))), REDUCED);
    assert_eq!(resolve_motion_prefs(false, Some(OsStr::new("1"))), REDUCED);
    assert_eq!(resolve_motion_prefs(false, Some(OsStr::new("0"))), FULL);
    for junk in ["", "yes", "true", " 1", "1 ", "2", "123456789", "111111111"] {
        assert_eq!(
            resolve_motion_prefs(false, Some(OsStr::new(junk))),
            FULL,
            "{junk:?}"
        );
    }
}

#[test]
fn test_lka7_non_utf8_env_is_ignored() {
    use std::os::unix::ffi::OsStrExt;
    let raw = OsStr::from_bytes(&[0xFF, 0x31]);
    assert_eq!(resolve_motion_prefs(false, Some(raw)), FULL);
}

#[test]
fn test_lka8_cubic_bezier_endpoints_monotonic_and_finite() {
    assert!(cubic_bezier(EASE_STANDARD, 0.0).abs() < 1e-4);
    assert!((cubic_bezier(EASE_STANDARD, 1.0) - 1.0).abs() < 1e-4);
    assert!(cubic_bezier(EASE_STANDARD, -3.0).abs() < 1e-4);
    assert!((cubic_bezier(EASE_STANDARD, 7.0) - 1.0).abs() < 1e-4);
    assert!(cubic_bezier(EASE_STANDARD, f32::NAN).abs() < 1e-4);
    assert!(cubic_bezier(EASE_STANDARD, f32::INFINITY).abs() < 1e-4);
    // The standard curve is ahead of linear at mid-course: cubic-bezier(0.2, 0, 0, 1) at x = 0.5
    // is about 0.878.
    let mid = cubic_bezier(EASE_STANDARD, 0.5);
    assert!((0.87..=0.885).contains(&mid), "{mid}");
    let mut previous = 0.0_f32;
    for i in 0..=100 {
        let y = cubic_bezier(EASE_STANDARD, i as f32 / 100.0);
        assert!(y.is_finite());
        assert!(y + 1e-4 >= previous, "not monotonic at {i}");
        previous = y;
    }
}

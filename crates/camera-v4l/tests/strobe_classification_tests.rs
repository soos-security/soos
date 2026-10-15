//! IR stream strobe classification on synthetic fixtures (GitHub #378, PR 1 of #353; matrix
//! IRA12–IRA15). The four classes are recognised, everything else fails closed, and a stream
//! that changes class vetoes `Allow` for good.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use proptest::prelude::*;
use soos_camera_v4l::ir_emitter::EmitterState;
use soos_camera_v4l::strobe::{
    classify_strobe, frame_mean_luma, LumaSample, StrobeClass, StrobeError, StrobeMonitor,
    StrobeStatus, STROBE_DARK_LUMA, STROBE_MAX_SAMPLES, STROBE_MIN_LUMA_DELTA, STROBE_MIN_SAMPLES,
};
use soos_camera_v4l::{Frame, PixelFormat, SensorType};
use soos_test_fixtures::ir_strobe::{self, IrSample};

fn samples(seq: &[IrSample]) -> Vec<LumaSample> {
    seq.iter()
        .map(|s| {
            let cmd = s.commanded_on.map(|on| {
                if on {
                    EmitterState::On
                } else {
                    EmitterState::Off
                }
            });
            LumaSample::from_ir_frame(&s.frame, cmd).expect("IR fixture frame")
        })
        .collect()
}

fn raw(lumas: &[f32]) -> Vec<LumaSample> {
    lumas
        .iter()
        .enumerate()
        .map(|(i, &l)| LumaSample {
            sequence: i as u64,
            mean_luma: l,
            commanded: None,
        })
        .collect()
}

/// The canonical non-periodic pattern (not a function of `sequence % p` for p = 2, 3, 4).
const PATTERN: [bool; 8] = [true, true, false, false, true, false, false, true];

/// IRA13: the canonical pattern really is non-periodic for p = 2..=4 (guards later edits).
#[test]
fn test_ira_canonical_pattern_is_not_periodic() {
    for p in 2..=4usize {
        let periodic = (0..PATTERN.len()).all(|i| {
            (0..PATTERN.len())
                .filter(|j| j % p == i % p)
                .all(|j| PATTERN[j] == PATTERN[i])
        });
        assert!(!periodic, "PATTERN is periodic with p = {p}");
    }
    assert_eq!(PATTERN.iter().filter(|on| **on).count(), 4);
}

/// IRA12: each synthetic sequence gets its class.
#[test]
fn test_ira_synthetic_sequences_are_classified() {
    assert_eq!(
        classify_strobe(&samples(&ir_strobe::steady_sequence(12))).unwrap(),
        StrobeClass::Steady
    );
    assert_eq!(
        classify_strobe(&samples(&ir_strobe::native_alternating_sequence(12, true))).unwrap(),
        StrobeClass::NativeAlternating
    );
    assert_eq!(
        classify_strobe(&samples(&ir_strobe::native_alternating_sequence(12, false))).unwrap(),
        StrobeClass::NativeAlternating
    );
    assert_eq!(
        classify_strobe(&samples(&ir_strobe::controllable_sequence(&PATTERN))).unwrap(),
        StrobeClass::Controllable
    );
    assert_eq!(
        classify_strobe(&samples(&ir_strobe::absent_sequence(12))).unwrap(),
        StrobeClass::Absent
    );
}

/// IRA12: commands that the scene ignores do not make a stream controllable.
#[test]
fn test_ira_unresponsive_control_is_not_controllable() {
    let class = classify_strobe(&samples(&ir_strobe::unresponsive_sequence(&PATTERN))).unwrap();
    assert_eq!(class, StrobeClass::Steady);
}

/// IRA12: a natively alternating stream with a non-parity command pattern stays native.
#[test]
fn test_ira_native_alternation_is_not_mistaken_for_control() {
    let mut seq = samples(&ir_strobe::native_alternating_sequence(8, true));
    for (s, on) in seq.iter_mut().zip(PATTERN) {
        s.commanded = Some(if on {
            EmitterState::On
        } else {
            EmitterState::Off
        });
    }
    assert_eq!(
        classify_strobe(&seq).unwrap(),
        StrobeClass::NativeAlternating
    );
}

/// IRA13: a command pattern aligned with sequence parity proves nothing and is refused.
#[test]
fn test_ira_parity_aligned_pattern_is_ambiguous() {
    let pattern: Vec<bool> = (0..10).map(|i| i % 2 == 0).collect();
    let err = classify_strobe(&samples(&ir_strobe::controllable_sequence(&pattern))).unwrap_err();
    assert_eq!(err, StrobeError::AmbiguousPattern);
}

/// IRA13: a pattern that is a function of `sequence % p` for p in 2..=4 is ambiguous: a native
/// period-4 strobe with the in-phase on,on,off,off pattern is never `Controllable`.
#[test]
fn test_ira_short_period_patterns_are_ambiguous() {
    let lit = |i: usize| i % 4 < 2;
    let seq: Vec<LumaSample> = (0..12)
        .map(|i| LumaSample {
            sequence: i as u64,
            mean_luma: if lit(i) { 120.0 } else { 15.0 },
            commanded: Some(if lit(i) {
                EmitterState::On
            } else {
                EmitterState::Off
            }),
        })
        .collect();
    assert_eq!(
        classify_strobe(&seq).unwrap_err(),
        StrobeError::AmbiguousPattern
    );
    // Inverted period-3 pattern: still a function of sequence % 3.
    let seq: Vec<LumaSample> = (0..12u64)
        .map(|i| LumaSample {
            sequence: i,
            mean_luma: if i % 3 == 0 { 15.0 } else { 120.0 },
            commanded: Some(if i % 3 == 0 {
                EmitterState::Off
            } else {
                EmitterState::On
            }),
        })
        .collect();
    assert_eq!(
        classify_strobe(&seq).unwrap_err(),
        StrobeError::AmbiguousPattern
    );
}

/// IRA13: sample-count bounds, invalid luma and non-monotonic sequences fail closed.
#[test]
fn test_ira_invalid_inputs_fail_closed() {
    assert_eq!(STROBE_MIN_SAMPLES, 8);
    assert_eq!(STROBE_MAX_SAMPLES, 64);
    assert_eq!(
        classify_strobe(&raw(&[100.0; 7])).unwrap_err(),
        StrobeError::TooFewSamples { got: 7, min: 8 }
    );
    assert_eq!(
        classify_strobe(&raw(&[100.0; 65])).unwrap_err(),
        StrobeError::TooManySamples { got: 65, max: 64 }
    );
    assert!(classify_strobe(&raw(&[100.0; 8])).is_ok());
    assert!(classify_strobe(&raw(&[100.0; 64])).is_ok());
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 255.5] {
        let mut lumas = [100.0f32; 8];
        lumas[3] = bad;
        assert_eq!(
            classify_strobe(&raw(&lumas)).unwrap_err(),
            StrobeError::InvalidLuma,
            "{bad}"
        );
    }
    let mut seq = raw(&[100.0; 8]);
    seq[5].sequence = seq[4].sequence;
    assert_eq!(
        classify_strobe(&seq).unwrap_err(),
        StrobeError::NonMonotonicSequence
    );
}

/// IRA13: irregular flicker matches no class.
#[test]
fn test_ira_irregular_flicker_is_refused() {
    let lumas = [100.0, 140.0, 101.0, 99.0, 150.0, 98.0, 160.0, 161.0];
    assert_eq!(
        classify_strobe(&raw(&lumas)).unwrap_err(),
        StrobeError::Irregular
    );
}

/// IRA13: the thresholds are boundaries: a gap of exactly DELTA separates, just below does not;
/// a maximum of exactly DARK is not absent.
#[test]
fn test_ira_threshold_boundaries() {
    let low = 60.0;
    let high = low + STROBE_MIN_LUMA_DELTA;
    let alt: Vec<f32> = (0..8)
        .map(|i| if i % 2 == 0 { high } else { low })
        .collect();
    assert_eq!(
        classify_strobe(&raw(&alt)).unwrap(),
        StrobeClass::NativeAlternating
    );
    let near = high - 0.01;
    let alt: Vec<f32> = (0..8)
        .map(|i| if i % 2 == 0 { near } else { low })
        .collect();
    assert_eq!(classify_strobe(&raw(&alt)).unwrap(), StrobeClass::Steady);
    assert_eq!(
        classify_strobe(&raw(&[STROBE_DARK_LUMA - 0.01; 8])).unwrap(),
        StrobeClass::Absent
    );
    assert_eq!(
        classify_strobe(&raw(&[STROBE_DARK_LUMA; 8])).unwrap(),
        StrobeClass::Steady
    );
}

/// IRA14: mean luma by format; mismatched geometry, MJPEG and empty frames give None; only
/// Infrared frames become samples.
#[test]
fn test_ira_frame_mean_luma_by_format() {
    let grey = Frame::new(vec![10, 20, 30, 40], 2, 2, 1, PixelFormat::Grey, 0);
    assert_eq!(frame_mean_luma(&grey), Some(25.0));
    let yuyv = Frame::new(vec![100, 0, 50, 255], 2, 1, 1, PixelFormat::Yuyv, 0);
    assert_eq!(frame_mean_luma(&yuyv), Some(75.0));
    let nv12 = Frame::new(vec![8, 8, 8, 8, 200, 200], 2, 2, 1, PixelFormat::Nv12, 0);
    assert_eq!(frame_mean_luma(&nv12), Some(8.0));
    let rgb = Frame::new(vec![30, 60, 90], 1, 1, 1, PixelFormat::Rgb24, 0);
    assert_eq!(frame_mean_luma(&rgb), Some(60.0));
    let short = Frame::new(vec![1, 2, 3], 2, 2, 1, PixelFormat::Grey, 0);
    assert_eq!(frame_mean_luma(&short), None);
    let mjpeg = Frame::new(vec![0xff, 0xd8], 2, 2, 1, PixelFormat::Mjpeg, 0);
    assert_eq!(frame_mean_luma(&mjpeg), None);
    let empty = Frame::new(Vec::new(), 0, 0, 1, PixelFormat::Grey, 0);
    assert_eq!(frame_mean_luma(&empty), None);

    let rgb_sensor = ir_strobe::ir_frame(16, 16, 3, true).with_sensor_type(SensorType::Rgb);
    assert!(LumaSample::from_ir_frame(&rgb_sensor, None).is_none());
    let unknown = ir_strobe::ir_frame(16, 16, 3, true).with_sensor_type(SensorType::Unknown);
    assert!(LumaSample::from_ir_frame(&unknown, None).is_none());
    let ir = ir_strobe::ir_frame(16, 16, 3, true);
    let s = LumaSample::from_ir_frame(&ir, Some(EmitterState::On)).unwrap();
    assert_eq!(s.sequence, 3);
    assert_eq!(s.commanded, Some(EmitterState::On));
}

/// IRA15: the monitor establishes a class, keeps it, and a class change is sticky and vetoes.
#[test]
fn test_ira_class_change_mid_stream_vetoes_allow() {
    let mut monitor = StrobeMonitor::new(8);
    let seq = samples(&ir_strobe::class_change_sequence(8, 16));
    let mut statuses = Vec::new();
    for s in seq {
        statuses.push(monitor.push(s));
    }
    assert_eq!(statuses[6], StrobeStatus::Learning);
    assert_eq!(statuses[7], StrobeStatus::Stable(StrobeClass::Steady));
    assert!(!statuses[7].vetoes_allow());
    assert_eq!(
        statuses[15],
        StrobeStatus::Changed {
            from: StrobeClass::Steady,
            to: Some(StrobeClass::NativeAlternating)
        }
    );
    assert!(monitor.status().vetoes_allow());
    // Sticky: a later block of the original class does not clear it.
    for s in samples(&ir_strobe::steady_sequence(8))
        .into_iter()
        .map(|mut s| {
            s.sequence += 100;
            s
        })
    {
        monitor.push(s);
    }
    assert!(matches!(monitor.status(), StrobeStatus::Changed { .. }));
}

/// IRA15: a stable stream stays stable over many blocks; learning and unclassified veto.
#[test]
fn test_ira_monitor_stable_stream_and_learning_states() {
    let mut monitor = StrobeMonitor::new(8);
    assert_eq!(monitor.status(), StrobeStatus::Learning);
    assert!(StrobeStatus::Learning.vetoes_allow());
    for s in samples(&ir_strobe::native_alternating_sequence(32, true)) {
        monitor.push(s);
    }
    assert_eq!(
        monitor.status(),
        StrobeStatus::Stable(StrobeClass::NativeAlternating)
    );

    let mut monitor = StrobeMonitor::new(8);
    for s in raw(&[100.0, 140.0, 101.0, 99.0, 150.0, 98.0, 160.0, 161.0]) {
        monitor.push(s);
    }
    assert_eq!(
        monitor.status(),
        StrobeStatus::Unclassified(StrobeError::Irregular)
    );
    assert!(monitor.status().vetoes_allow());
}

/// IRA15: `Unclassified` is sticky: a later clean block never makes the stream stable.
#[test]
fn test_ira_unclassified_is_sticky() {
    let mut monitor = StrobeMonitor::new(8);
    for s in raw(&[100.0, 140.0, 101.0, 99.0, 150.0, 98.0, 160.0, 161.0]) {
        monitor.push(s);
    }
    for s in samples(&ir_strobe::steady_sequence(16))
        .into_iter()
        .map(|mut s| {
            s.sequence += 8;
            s
        })
    {
        monitor.push(s);
    }
    assert_eq!(
        monitor.status(),
        StrobeStatus::Unclassified(StrobeError::Irregular)
    );
    assert!(monitor.status().vetoes_allow());
}

/// IRA15: a block that fails classification after a class was established is a change.
#[test]
fn test_ira_failure_after_establishment_is_a_change() {
    let mut monitor = StrobeMonitor::new(8);
    for s in samples(&ir_strobe::steady_sequence(8)) {
        monitor.push(s);
    }
    let mut flicker = raw(&[100.0, 140.0, 101.0, 99.0, 150.0, 98.0, 160.0, 161.0]);
    for s in &mut flicker {
        s.sequence += 8;
    }
    for s in flicker {
        monitor.push(s);
    }
    assert_eq!(
        monitor.status(),
        StrobeStatus::Changed {
            from: StrobeClass::Steady,
            to: None
        }
    );
}

/// IRA15: the block length is clamped to the classifier bounds.
#[test]
fn test_ira_monitor_block_length_is_clamped() {
    let mut monitor = StrobeMonitor::new(0);
    let seq = raw(&[100.0; STROBE_MIN_SAMPLES]);
    let last = seq.into_iter().map(|s| monitor.push(s)).last().unwrap();
    assert_eq!(last, StrobeStatus::Stable(StrobeClass::Steady));
    let mut monitor = StrobeMonitor::new(usize::MAX);
    let seq: Vec<LumaSample> = (0..STROBE_MAX_SAMPLES as u64)
        .map(|i| LumaSample {
            sequence: i,
            mean_luma: 100.0,
            commanded: None,
        })
        .collect();
    let last = seq.into_iter().map(|s| monitor.push(s)).last().unwrap();
    assert_eq!(last, StrobeStatus::Stable(StrobeClass::Steady));
}

proptest! {
    /// IRA13: arbitrary samples never panic, and a class is only returned for valid input.
    #[test]
    fn prop_ira_classifier_never_panics(
        lumas in proptest::collection::vec(any::<f32>(), 0..80),
        cmds in proptest::collection::vec(0u8..3, 0..80),
        gaps in proptest::collection::vec(0u64..3, 0..80),
    ) {
        let mut seq = 0u64;
        let samples: Vec<LumaSample> = lumas.iter().enumerate().map(|(i, &l)| {
            seq = seq.saturating_add(*gaps.get(i).unwrap_or(&1));
            LumaSample {
                sequence: seq,
                mean_luma: l,
                commanded: match cmds.get(i) { Some(1) => Some(EmitterState::On), Some(2) => Some(EmitterState::Off), _ => None },
            }
        }).collect();
        if classify_strobe(&samples).is_ok() {
            prop_assert!((STROBE_MIN_SAMPLES..=STROBE_MAX_SAMPLES).contains(&samples.len()));
            prop_assert!(samples.iter().all(|s| s.mean_luma.is_finite() && (0.0..=255.0).contains(&s.mean_luma)));
        }
        let mut monitor = StrobeMonitor::new(lumas.len());
        for s in samples {
            let _ = monitor.push(s);
        }
    }

    /// IRA14: mean luma never panics on arbitrary geometry and is always within 0..=255.
    #[test]
    fn prop_ira_mean_luma_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..512),
        width in 0u32..40,
        height in 0u32..40,
        fmt in 0usize..5,
    ) {
        let format = [PixelFormat::Grey, PixelFormat::Yuyv, PixelFormat::Nv12, PixelFormat::Rgb24, PixelFormat::Mjpeg][fmt];
        let frame = Frame::new(data, width, height, 1, format, 0);
        if let Some(l) = frame_mean_luma(&frame) {
            prop_assert!(l.is_finite() && (0.0..=255.0).contains(&l));
        }
    }
}

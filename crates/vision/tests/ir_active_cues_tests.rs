//! Contract tests of the active-illumination on/off differential liveness cues
//! (`soos_vision::ir_active`, GitHub #431 = PR 2 of #353; spec
//! `AI/architect_spec_ir_active_cues.md`; matrix IRA29–IRA34).
//!
//! Every fixture is synthetic and non-biometric (`soos_test_fixtures::ir_strobe::active_pair`).
//! A passing evaluation only means that no cue vetoed: it is never a liveness decision.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual test suite utilizes direct assertions, unwrap, indexing and fixture arithmetic"
)]

use proptest::prelude::*;
use soos_frame::{Frame, PixelFormat, SensorType};
use soos_inference_ort::{BoundingBox, FaceLandmarks, Point2f};
use soos_test_fixtures::ir_strobe::{self, ActivePair, ActivePresentation};
use soos_vision::ir_active::{
    evaluate_ir_active_cues, ir_active_statistics, IrActiveConfig, IrActiveRejection,
    IR_ACTIVE_GLINT_MIN_DELTA, IR_ACTIVE_GLINT_MIN_LUMA, IR_ACTIVE_MAX_CROP_PIXELS,
    IR_ACTIVE_MAX_GLINT_FRACTION, IR_ACTIVE_MAX_REFLECTANCE, IR_ACTIVE_MAX_SATURATED_FRACTION,
    IR_ACTIVE_MIN_FACE_RESPONSE, IR_ACTIVE_MIN_GLINT_PIXELS, IR_ACTIVE_MIN_REFLECTANCE,
    IR_ACTIVE_MIN_RELIEF_RATIO, IR_ACTIVE_MIN_RESPONSE_RATIO, IR_ACTIVE_SATURATION_LUMA,
};

fn face_box(pair: &ActivePair) -> BoundingBox {
    let [x1, y1, x2, y2] = pair.face_box;
    BoundingBox::new(x1, y1, x2, y2)
}

fn landmarks_of(points: [[f32; 2]; 5]) -> FaceLandmarks {
    FaceLandmarks::from_array(points.map(|[x, y]| Point2f::new(x, y)))
}

fn evaluate_pair(pair: &ActivePair, config: &IrActiveConfig) -> Result<(), IrActiveRejection> {
    evaluate_ir_active_cues(
        &pair.lit,
        &pair.unlit,
        &face_box(pair),
        &landmarks_of(pair.landmarks),
        config,
    )
    .map(|_| ())
}

fn evaluate_default(presentation: ActivePresentation) -> Result<(), IrActiveRejection> {
    evaluate_pair(
        &ir_strobe::active_pair(presentation),
        &IrActiveConfig::default(),
    )
}

fn grey_ir(data: Vec<u8>, width: u32, height: u32, sequence: u64) -> Frame {
    Frame::new(data, width, height, 1_000, PixelFormat::Grey, sequence)
        .with_sensor_type(SensorType::Infrared)
}

// ---------------------------------------------------------------------------------------------
// IRA29 — the bona fide shaded head passes every cue; statistics are consistent
// ---------------------------------------------------------------------------------------------

#[test]
fn test_ira_active_shaded_head_passes_every_cue() {
    let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
    let config = IrActiveConfig::default();
    let clear = evaluate_ir_active_cues(
        &pair.lit,
        &pair.unlit,
        &face_box(&pair),
        &landmarks_of(pair.landmarks),
        &config,
    )
    .expect("the 3D-shaded head with glints must not be vetoed");
    let stats = clear.statistics();
    assert!(
        stats.face_response >= IR_ACTIVE_MIN_FACE_RESPONSE,
        "{stats:?}"
    );
    assert!(
        stats.response_ratio >= IR_ACTIVE_MIN_RESPONSE_RATIO,
        "{stats:?}"
    );
    assert!(stats.background_response > 0.0, "{stats:?}");
    assert!(
        stats.saturated_fraction <= IR_ACTIVE_MAX_SATURATED_FRACTION,
        "{stats:?}"
    );
    assert!(
        (IR_ACTIVE_MIN_REFLECTANCE..=IR_ACTIVE_MAX_REFLECTANCE).contains(&stats.lit_face_mean),
        "{stats:?}"
    );
    assert!(
        stats.relief_ratio >= IR_ACTIVE_MIN_RELIEF_RATIO,
        "{stats:?}"
    );
    for eye in 0..2 {
        assert!(
            stats.glint_pixels[eye] >= IR_ACTIVE_MIN_GLINT_PIXELS,
            "{stats:?}"
        );
        assert!(
            stats.eye_region_pixels[eye] > stats.glint_pixels[eye],
            "{stats:?}"
        );
    }
}

#[test]
fn test_ira_active_statistics_match_the_evaluation() {
    let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
    let config = IrActiveConfig::default();
    let landmarks = landmarks_of(pair.landmarks);
    let stats = ir_active_statistics(
        &pair.lit,
        &pair.unlit,
        &face_box(&pair),
        &landmarks,
        &config,
    )
    .expect("statistics of a well-formed pair");
    let clear = evaluate_ir_active_cues(
        &pair.lit,
        &pair.unlit,
        &face_box(&pair),
        &landmarks,
        &config,
    )
    .expect("clear");
    assert_eq!(stats, clear.statistics());
    // Deterministic: the same input gives the same statistics.
    let again = ir_active_statistics(
        &pair.lit,
        &pair.unlit,
        &face_box(&pair),
        &landmarks,
        &config,
    )
    .expect("statistics");
    assert_eq!(stats, again);
}

#[test]
fn test_ira_active_statistics_are_computed_for_attack_fixtures_too() {
    // The statistics never decide: they are available for every well-formed pair, attack or not.
    for presentation in [
        ActivePresentation::FlatPrint,
        ActivePresentation::Screen,
        ActivePresentation::NoGlint,
        ActivePresentation::Saturated,
        ActivePresentation::Unlit,
    ] {
        let pair = ir_strobe::active_pair(presentation);
        let stats = ir_active_statistics(
            &pair.lit,
            &pair.unlit,
            &face_box(&pair),
            &landmarks_of(pair.landmarks),
            &IrActiveConfig::default(),
        );
        assert!(stats.is_ok(), "{presentation:?}: {stats:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// IRA30 — every attack fixture is rejected, each by its intended cue
// ---------------------------------------------------------------------------------------------

#[test]
fn test_ira_active_flat_print_is_rejected_for_flat_relief() {
    assert_eq!(
        evaluate_default(ActivePresentation::FlatPrint),
        Err(IrActiveRejection::FlatRelief)
    );
}

#[test]
fn test_ira_active_screen_is_rejected_for_no_response() {
    assert_eq!(
        evaluate_default(ActivePresentation::Screen),
        Err(IrActiveRejection::NoResponse)
    );
}

#[test]
fn test_ira_active_missing_glint_is_rejected() {
    assert_eq!(
        evaluate_default(ActivePresentation::NoGlint),
        Err(IrActiveRejection::NoGlint)
    );
}

#[test]
fn test_ira_active_saturated_pair_is_rejected() {
    assert_eq!(
        evaluate_default(ActivePresentation::Saturated),
        Err(IrActiveRejection::Saturated)
    );
}

#[test]
fn test_ira_active_unlit_pair_is_rejected_for_no_response() {
    let pair = ir_strobe::active_pair(ActivePresentation::Unlit);
    assert_ne!(
        pair.lit.data, pair.unlit.data,
        "fixture must not be byte-identical"
    );
    assert_eq!(
        evaluate_default(ActivePresentation::Unlit),
        Err(IrActiveRejection::NoResponse)
    );
}

#[test]
fn test_ira_active_identical_pair_is_rejected_as_static_injection() {
    assert_eq!(
        evaluate_default(ActivePresentation::StaticInjection),
        Err(IrActiveRejection::StaticPair)
    );
    // The same frame passed twice (same sequence number) is static as well, even when the
    // bytes of one copy were edited.
    let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
    let mut unlit = pair.unlit.clone();
    unlit.sequence = pair.lit.sequence;
    assert_eq!(
        evaluate_ir_active_cues(
            &pair.lit,
            &unlit,
            &face_box(&pair),
            &landmarks_of(pair.landmarks),
            &IrActiveConfig::default()
        )
        .map(|_| ()),
        Err(IrActiveRejection::StaticPair)
    );
}

#[test]
fn test_ira_active_only_the_shaded_head_passes() {
    for presentation in ActivePresentation::ALL {
        let result = evaluate_default(presentation);
        if presentation == ActivePresentation::ShadedHead {
            assert_eq!(result, Ok(()), "{presentation:?}");
        } else {
            assert!(result.is_err(), "{presentation:?} must be rejected");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// IRA31 — a swapped pair fails; every cue alone vetoes (no cue compensates another)
// ---------------------------------------------------------------------------------------------

#[test]
fn test_ira_active_swapped_pair_is_rejected() {
    let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
    let result = evaluate_ir_active_cues(
        &pair.unlit,
        &pair.lit,
        &face_box(&pair),
        &landmarks_of(pair.landmarks),
        &IrActiveConfig::default(),
    )
    .map(|_| ());
    assert_eq!(result, Err(IrActiveRejection::NoResponse));
}

#[test]
fn test_ira_active_each_cue_alone_vetoes_the_shaded_head() {
    let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
    let base = IrActiveConfig::default();
    let cases: [(&str, IrActiveConfig, IrActiveRejection); 7] = [
        (
            "min_face_response",
            IrActiveConfig {
                min_face_response: 255.0,
                ..base
            },
            IrActiveRejection::NoResponse,
        ),
        (
            "min_response_ratio",
            IrActiveConfig {
                min_response_ratio: 255.0,
                ..base
            },
            IrActiveRejection::NoResponse,
        ),
        (
            "max_saturated_fraction",
            IrActiveConfig {
                max_saturated_fraction: 0.0,
                ..base
            },
            IrActiveRejection::Saturated,
        ),
        (
            "max_reflectance",
            IrActiveConfig {
                max_reflectance: 60.0,
                ..base
            },
            IrActiveRejection::ReflectanceOutOfBand,
        ),
        (
            "min_relief_ratio",
            IrActiveConfig {
                min_relief_ratio: 255.0,
                ..base
            },
            IrActiveRejection::FlatRelief,
        ),
        (
            "min_glint_pixels",
            IrActiveConfig {
                min_glint_pixels: 4096,
                ..base
            },
            IrActiveRejection::NoGlint,
        ),
        (
            "max_glint_fraction",
            IrActiveConfig {
                max_glint_fraction: 0.01,
                ..base
            },
            IrActiveRejection::NoGlint,
        ),
    ];
    for (field, config, expected) in cases {
        assert!(
            config.validate().is_ok(),
            "{field}: test config must be valid"
        );
        assert_eq!(evaluate_pair(&pair, &config), Err(expected), "{field}");
    }
    // The reflectance band rejects a too-dark lit face as well.
    let dark = IrActiveConfig {
        min_reflectance: 199.0,
        max_reflectance: 200.0,
        ..base
    };
    assert_eq!(
        evaluate_pair(&pair, &dark),
        Err(IrActiveRejection::ReflectanceOutOfBand)
    );
}

// ---------------------------------------------------------------------------------------------
// IRA32 — malformed frames and geometry fail closed
// ---------------------------------------------------------------------------------------------

fn evaluate_with(
    lit: &Frame,
    unlit: &Frame,
    bbox: BoundingBox,
    landmarks: [[f32; 2]; 5],
) -> Result<(), IrActiveRejection> {
    evaluate_ir_active_cues(
        lit,
        unlit,
        &bbox,
        &landmarks_of(landmarks),
        &IrActiveConfig::default(),
    )
    .map(|_| ())
}

#[test]
fn test_ira_active_rejects_malformed_frames() {
    let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
    let bbox = face_box(&pair);
    let lm = pair.landmarks;

    // Wrong pixel format on either frame.
    for format in [
        PixelFormat::Yuyv,
        PixelFormat::Rgb24,
        PixelFormat::Nv12,
        PixelFormat::Mjpeg,
    ] {
        let mut lit = pair.lit.clone();
        lit.format = format;
        assert_eq!(
            evaluate_with(&lit, &pair.unlit, bbox, lm),
            Err(IrActiveRejection::InvalidFrames),
            "lit {format:?}"
        );
        let mut unlit = pair.unlit.clone();
        unlit.format = format;
        assert_eq!(
            evaluate_with(&pair.lit, &unlit, bbox, lm),
            Err(IrActiveRejection::InvalidFrames),
            "unlit {format:?}"
        );
    }

    // A frame not stamped Infrared (RGB or unclassified sensor).
    for sensor in [SensorType::Rgb, SensorType::Unknown] {
        let lit = pair.lit.clone().with_sensor_type(sensor);
        assert_eq!(
            evaluate_with(&lit, &pair.unlit, bbox, lm),
            Err(IrActiveRejection::InvalidFrames),
            "lit {sensor:?}"
        );
        let unlit = pair.unlit.clone().with_sensor_type(sensor);
        assert_eq!(
            evaluate_with(&pair.lit, &unlit, bbox, lm),
            Err(IrActiveRejection::InvalidFrames),
            "unlit {sensor:?}"
        );
    }

    // Mismatched geometry.
    let w = ir_strobe::ACTIVE_WIDTH;
    let h = ir_strobe::ACTIVE_HEIGHT;
    let narrower = grey_ir(vec![10; ((w - 1) * h) as usize], w - 1, h, 99);
    assert_eq!(
        evaluate_with(&pair.lit, &narrower, bbox, lm),
        Err(IrActiveRejection::InvalidFrames)
    );
    let shorter = grey_ir(vec![10; (w * (h - 1)) as usize], w, h - 1, 99);
    assert_eq!(
        evaluate_with(&shorter, &pair.unlit, bbox, lm),
        Err(IrActiveRejection::InvalidFrames)
    );

    // Buffer length that does not match the declared dimensions.
    let mut truncated = pair.unlit.clone();
    truncated.data.pop();
    assert_eq!(
        evaluate_with(&pair.lit, &truncated, bbox, lm),
        Err(IrActiveRejection::InvalidFrames)
    );
    let mut padded = pair.lit.clone();
    padded.data.push(0);
    assert_eq!(
        evaluate_with(&padded, &pair.unlit, bbox, lm),
        Err(IrActiveRejection::InvalidFrames)
    );

    // Zero-size and empty frames.
    let empty_a = grey_ir(Vec::new(), 0, 0, 1);
    let empty_b = grey_ir(Vec::new(), 0, 0, 2);
    assert_eq!(
        evaluate_with(&empty_a, &empty_b, bbox, lm),
        Err(IrActiveRejection::InvalidFrames)
    );
    let zero_w_a = grey_ir(Vec::new(), 0, h, 1);
    let zero_w_b = grey_ir(Vec::new(), 0, h, 2);
    assert_eq!(
        evaluate_with(&zero_w_a, &zero_w_b, bbox, lm),
        Err(IrActiveRejection::InvalidFrames)
    );

    // Dimensions whose product overflows or exceeds the frame bound never panic.
    let huge_a = grey_ir(Vec::new(), u32::MAX, u32::MAX, 1);
    let huge_b = grey_ir(Vec::new(), u32::MAX, u32::MAX, 2);
    assert_eq!(
        evaluate_with(&huge_a, &huge_b, bbox, lm),
        Err(IrActiveRejection::InvalidFrames)
    );
    let over_a = grey_ir(Vec::new(), 4097, 4096, 1);
    let over_b = grey_ir(Vec::new(), 4097, 4096, 2);
    assert_eq!(
        evaluate_with(&over_a, &over_b, bbox, lm),
        Err(IrActiveRejection::InvalidFrames)
    );
}

#[test]
fn test_ira_active_rejects_malformed_geometry() {
    let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
    let lit = &pair.lit;
    let unlit = &pair.unlit;
    let lm = pair.landmarks;
    let [x1, y1, x2, y2] = pair.face_box;

    // Non-finite box coordinates.
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let boxes = [
            BoundingBox {
                x1: bad,
                y1,
                x2,
                y2,
            },
            BoundingBox {
                x1,
                y1: bad,
                x2,
                y2,
            },
            BoundingBox {
                x1,
                y1,
                x2: bad,
                y2,
            },
            BoundingBox {
                x1,
                y1,
                x2,
                y2: bad,
            },
        ];
        for b in boxes {
            assert_eq!(
                evaluate_with(lit, unlit, b, lm),
                Err(IrActiveRejection::InvalidGeometry),
                "{b:?}"
            );
        }
        // Non-finite landmark coordinates, each point and each axis.
        for point in 0..5 {
            for axis in 0..2 {
                let mut bad_lm = lm;
                bad_lm[point][axis] = bad;
                assert_eq!(
                    evaluate_with(lit, unlit, BoundingBox::new(x1, y1, x2, y2), bad_lm),
                    Err(IrActiveRejection::InvalidGeometry),
                    "landmark {point} axis {axis} = {bad}"
                );
            }
        }
    }

    // Box entirely outside the frame, and an inverted (zero-area after clamp) box.
    assert_eq!(
        evaluate_with(lit, unlit, BoundingBox::new(200.0, 200.0, 260.0, 280.0), lm),
        Err(IrActiveRejection::InvalidGeometry)
    );
    assert_eq!(
        evaluate_with(
            lit,
            unlit,
            BoundingBox {
                x1: 110.0,
                y1: 105.0,
                x2: 50.0,
                y2: 25.0
            },
            lm
        ),
        Err(IrActiveRejection::InvalidGeometry)
    );

    // Box smaller than the minimum face side (eyes and nose still inside it).
    let tiny_lm = [
        [72.0, 60.0],
        [81.0, 60.0],
        [76.0, 64.0],
        [73.0, 67.0],
        [79.0, 67.0],
    ];
    assert_eq!(
        evaluate_with(
            lit,
            unlit,
            BoundingBox::new(70.0, 58.0, 84.0, 70.0),
            tiny_lm
        ),
        Err(IrActiveRejection::InvalidGeometry)
    );

    // Nose, left eye or right eye outside the box.
    for (point, coord) in [
        (2usize, [130.0, 68.0]),
        (0, [40.0, 50.0]),
        (1, [92.0, 10.0]),
    ] {
        let mut out = lm;
        out[point] = coord;
        assert_eq!(
            evaluate_with(lit, unlit, BoundingBox::new(x1, y1, x2, y2), out),
            Err(IrActiveRejection::InvalidGeometry),
            "landmark {point} outside the box"
        );
    }

    // Eyes too close together (and coincident eyes).
    let mut close = lm;
    close[0] = [78.0, 50.0];
    close[1] = [82.0, 50.0];
    assert_eq!(
        evaluate_with(lit, unlit, BoundingBox::new(x1, y1, x2, y2), close),
        Err(IrActiveRejection::InvalidGeometry)
    );
    let mut same = lm;
    same[1] = same[0];
    assert_eq!(
        evaluate_with(lit, unlit, BoundingBox::new(x1, y1, x2, y2), same),
        Err(IrActiveRejection::InvalidGeometry)
    );
}

#[test]
fn test_ira_active_rejects_a_face_filling_the_frame() {
    let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
    let w = ir_strobe::ACTIVE_WIDTH as f32;
    let h = ir_strobe::ACTIVE_HEIGHT as f32;
    assert_eq!(
        evaluate_with(
            &pair.lit,
            &pair.unlit,
            BoundingBox::new(0.0, 0.0, w, h),
            pair.landmarks
        ),
        Err(IrActiveRejection::InsufficientBackground)
    );
}

#[test]
fn test_ira_active_rejects_a_face_crop_above_the_bound() {
    // 1030 x 1030 = 1_060_900 pixels > IR_ACTIVE_MAX_CROP_PIXELS (1024 x 1024).
    let (w, h) = (2048u32, 1100u32);
    const { assert!(1030 * 1030 > IR_ACTIVE_MAX_CROP_PIXELS) };
    let lit = grey_ir(vec![120; (w * h) as usize], w, h, 1);
    let unlit = grey_ir(vec![20; (w * h) as usize], w, h, 2);
    let lm = [
        [400.0, 400.0],
        [600.0, 400.0],
        [500.0, 550.0],
        [420.0, 700.0],
        [580.0, 700.0],
    ];
    assert_eq!(
        evaluate_with(&lit, &unlit, BoundingBox::new(5.0, 5.0, 1035.0, 1035.0), lm),
        Err(IrActiveRejection::InvalidGeometry)
    );
}

// ---------------------------------------------------------------------------------------------
// IRA33 — the configuration is validated; an invalid configuration always rejects
// ---------------------------------------------------------------------------------------------

#[test]
fn test_ira_active_default_config_is_valid_and_provisional() {
    let config = IrActiveConfig::default();
    assert_eq!(config.validate(), Ok(()));
    assert_eq!(config.min_face_response, IR_ACTIVE_MIN_FACE_RESPONSE);
    assert_eq!(config.min_response_ratio, IR_ACTIVE_MIN_RESPONSE_RATIO);
    assert_eq!(config.saturation_luma, IR_ACTIVE_SATURATION_LUMA);
    assert_eq!(
        config.max_saturated_fraction,
        IR_ACTIVE_MAX_SATURATED_FRACTION
    );
    assert_eq!(config.min_reflectance, IR_ACTIVE_MIN_REFLECTANCE);
    assert_eq!(config.max_reflectance, IR_ACTIVE_MAX_REFLECTANCE);
    assert_eq!(config.min_relief_ratio, IR_ACTIVE_MIN_RELIEF_RATIO);
    assert_eq!(config.glint_min_luma, IR_ACTIVE_GLINT_MIN_LUMA);
    assert_eq!(config.glint_min_delta, IR_ACTIVE_GLINT_MIN_DELTA);
    assert_eq!(config.min_glint_pixels, IR_ACTIVE_MIN_GLINT_PIXELS);
    assert_eq!(config.max_glint_fraction, IR_ACTIVE_MAX_GLINT_FRACTION);
}

fn invalid_configs() -> Vec<(&'static str, IrActiveConfig)> {
    let base = IrActiveConfig::default();
    let mut out = Vec::new();
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        out.push((
            "min_face_response",
            IrActiveConfig {
                min_face_response: bad,
                ..base
            },
        ));
        out.push((
            "min_response_ratio",
            IrActiveConfig {
                min_response_ratio: bad,
                ..base
            },
        ));
        out.push((
            "max_saturated_fraction",
            IrActiveConfig {
                max_saturated_fraction: bad,
                ..base
            },
        ));
        out.push((
            "min_reflectance",
            IrActiveConfig {
                min_reflectance: bad,
                ..base
            },
        ));
        out.push((
            "max_reflectance",
            IrActiveConfig {
                max_reflectance: bad,
                ..base
            },
        ));
        out.push((
            "min_relief_ratio",
            IrActiveConfig {
                min_relief_ratio: bad,
                ..base
            },
        ));
        out.push((
            "max_glint_fraction",
            IrActiveConfig {
                max_glint_fraction: bad,
                ..base
            },
        ));
    }
    // Out-of-range values that would switch a cue off or make it inconsistent.
    out.extend([
        (
            "min_face_response",
            IrActiveConfig {
                min_face_response: 0.0,
                ..base
            },
        ),
        (
            "min_face_response",
            IrActiveConfig {
                min_face_response: -1.0,
                ..base
            },
        ),
        (
            "min_face_response",
            IrActiveConfig {
                min_face_response: 255.5,
                ..base
            },
        ),
        (
            "min_response_ratio",
            IrActiveConfig {
                min_response_ratio: 0.99,
                ..base
            },
        ),
        (
            "min_response_ratio",
            IrActiveConfig {
                min_response_ratio: 256.0,
                ..base
            },
        ),
        (
            "saturation_luma",
            IrActiveConfig {
                saturation_luma: 0,
                ..base
            },
        ),
        (
            "max_saturated_fraction",
            IrActiveConfig {
                max_saturated_fraction: -0.01,
                ..base
            },
        ),
        (
            "max_saturated_fraction",
            IrActiveConfig {
                max_saturated_fraction: 1.01,
                ..base
            },
        ),
        (
            "min_reflectance",
            IrActiveConfig {
                min_reflectance: -1.0,
                ..base
            },
        ),
        (
            "max_reflectance",
            IrActiveConfig {
                max_reflectance: 256.0,
                ..base
            },
        ),
        (
            "min_reflectance",
            IrActiveConfig {
                min_reflectance: 120.0,
                max_reflectance: 120.0,
                ..base
            },
        ),
        (
            "min_reflectance",
            IrActiveConfig {
                min_reflectance: 150.0,
                max_reflectance: 100.0,
                ..base
            },
        ),
        (
            "min_relief_ratio",
            IrActiveConfig {
                min_relief_ratio: 0.5,
                ..base
            },
        ),
        (
            "min_relief_ratio",
            IrActiveConfig {
                min_relief_ratio: 300.0,
                ..base
            },
        ),
        (
            "glint_min_luma",
            IrActiveConfig {
                glint_min_luma: 0,
                ..base
            },
        ),
        (
            "glint_min_delta",
            IrActiveConfig {
                glint_min_delta: 0,
                ..base
            },
        ),
        (
            "min_glint_pixels",
            IrActiveConfig {
                min_glint_pixels: 0,
                ..base
            },
        ),
        (
            "min_glint_pixels",
            IrActiveConfig {
                min_glint_pixels: 4097,
                ..base
            },
        ),
        (
            "max_glint_fraction",
            IrActiveConfig {
                max_glint_fraction: 0.0,
                ..base
            },
        ),
        (
            "max_glint_fraction",
            IrActiveConfig {
                max_glint_fraction: 1.5,
                ..base
            },
        ),
    ]);
    out
}

#[test]
fn test_ira_active_config_validation_names_the_field() {
    for (field, config) in invalid_configs() {
        let err = config
            .validate()
            .expect_err(&format!("{field}: {config:?} must be invalid"));
        assert_eq!(err.field, field, "{config:?}");
        let text = err.to_string();
        assert!(text.contains(field), "{text}");
        assert!(
            !text.chars().any(|c| c.is_ascii_digit()),
            "the message must not echo a value: {text}"
        );
    }
}

#[test]
fn test_ira_active_invalid_config_always_rejects() {
    let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
    for (field, config) in invalid_configs() {
        assert_eq!(
            evaluate_pair(&pair, &config),
            Err(IrActiveRejection::InvalidConfig),
            "{field}"
        );
        assert_eq!(
            ir_active_statistics(
                &pair.lit,
                &pair.unlit,
                &face_box(&pair),
                &landmarks_of(pair.landmarks),
                &config
            ),
            Err(IrActiveRejection::InvalidConfig),
            "{field}"
        );
    }
}

#[test]
fn test_ira_active_rejection_messages_carry_no_value() {
    let all = [
        IrActiveRejection::InvalidConfig,
        IrActiveRejection::InvalidFrames,
        IrActiveRejection::InvalidGeometry,
        IrActiveRejection::InsufficientBackground,
        IrActiveRejection::StaticPair,
        IrActiveRejection::NoResponse,
        IrActiveRejection::Saturated,
        IrActiveRejection::ReflectanceOutOfBand,
        IrActiveRejection::FlatRelief,
        IrActiveRejection::NoGlint,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for reason in all {
        let text = reason.to_string();
        assert!(!text.is_empty());
        assert!(
            !text.chars().any(|c| c.is_ascii_digit()),
            "{reason:?}: {text}"
        );
        assert!(seen.insert(text), "{reason:?}: messages must be distinct");
    }
}

// ---------------------------------------------------------------------------------------------
// IRA34 — no input panics or passes without every cue holding
// ---------------------------------------------------------------------------------------------

fn any_f32() -> impl Strategy<Value = f32> {
    prop_oneof![
        Just(f32::NAN),
        Just(f32::INFINITY),
        Just(f32::NEG_INFINITY),
        Just(0.0f32),
        -50.0f32..250.0f32,
        prop::num::f32::ANY,
    ]
}

fn any_config() -> impl Strategy<Value = IrActiveConfig> {
    (
        (any_f32(), any_f32(), any::<u8>(), any_f32()),
        (any_f32(), any_f32(), any_f32()),
        (any::<u8>(), any::<u8>(), 0u32..5000, any_f32()),
    )
        .prop_map(
            |(
                (min_face_response, min_response_ratio, saturation_luma, max_saturated_fraction),
                (min_reflectance, max_reflectance, min_relief_ratio),
                (glint_min_luma, glint_min_delta, min_glint_pixels, max_glint_fraction),
            )| IrActiveConfig {
                min_face_response,
                min_response_ratio,
                saturation_luma,
                max_saturated_fraction,
                min_reflectance,
                max_reflectance,
                min_relief_ratio,
                glint_min_luma,
                glint_min_delta,
                min_glint_pixels,
                max_glint_fraction,
            },
        )
}

/// A valid configuration drawn inside the documented ranges.
fn valid_config() -> impl Strategy<Value = IrActiveConfig> {
    (
        (1.0f32..=255.0, 1.0f32..=255.0, 1u8..=255, 0.0f32..=1.0),
        (0.0f32..=254.0, 1.0f32..=255.0),
        (1u8..=255, 1u8..=255, 1u32..=4096, 0.001f32..=1.0),
    )
        .prop_map(
            |(
                (min_face_response, min_response_ratio, saturation_luma, max_saturated_fraction),
                (min_reflectance, relief),
                (glint_min_luma, glint_min_delta, min_glint_pixels, max_glint_fraction),
            )| IrActiveConfig {
                min_face_response,
                min_response_ratio,
                saturation_luma,
                max_saturated_fraction,
                min_reflectance,
                max_reflectance: 255.0,
                min_relief_ratio: relief,
                glint_min_luma,
                glint_min_delta,
                min_glint_pixels,
                max_glint_fraction,
            },
        )
}

fn presentation() -> impl Strategy<Value = ActivePresentation> {
    (0usize..ActivePresentation::ALL.len()).prop_map(|i| ActivePresentation::ALL[i])
}

/// `Ok` implies that every cue held on the reported statistics (no hidden grant path).
fn assert_clear_is_consistent(
    result: &Result<soos_vision::ir_active::IrActiveCuesClear, IrActiveRejection>,
    config: &IrActiveConfig,
) {
    if let Ok(clear) = result {
        let s = clear.statistics();
        assert!(config.validate().is_ok(), "Ok with an invalid config");
        assert!(s.face_response.is_finite() && s.face_response >= config.min_face_response);
        assert!(s.response_ratio.is_finite() && s.response_ratio >= config.min_response_ratio);
        assert!(s.saturated_fraction <= config.max_saturated_fraction);
        assert!(s.lit_face_mean >= config.min_reflectance);
        assert!(s.lit_face_mean <= config.max_reflectance);
        assert!(s.relief_ratio.is_finite() && s.relief_ratio >= config.min_relief_ratio);
        for eye in 0..2 {
            assert!(s.glint_pixels[eye] >= config.min_glint_pixels);
            assert!(s.eye_region_pixels[eye] > 0);
            let fraction = s.glint_pixels[eye] as f32 / s.eye_region_pixels[eye] as f32;
            assert!(fraction <= config.max_glint_fraction);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Arbitrary bytes, dimensions, formats, sensors, geometry and configs never panic, and an
    /// `Ok` is only ever returned when every cue held.
    #[test]
    fn prop_ira_active_arbitrary_input_never_panics(
        w in 0u32..48,
        h in 0u32..48,
        lit_bytes in prop::collection::vec(any::<u8>(), 0..2400),
        unlit_bytes in prop::collection::vec(any::<u8>(), 0..2400),
        exact_length in any::<bool>(),
        lit_seq in 0u64..4,
        unlit_seq in 0u64..4,
        grey in any::<bool>(),
        infrared in any::<bool>(),
        coords in prop::array::uniform4(any_f32()),
        points in prop::array::uniform10(any_f32()),
        config in any_config(),
    ) {
        let n = (w * h) as usize;
        let shape = |mut bytes: Vec<u8>| {
            if exact_length {
                bytes.resize(n, 0);
            }
            bytes
        };
        let format = if grey { PixelFormat::Grey } else { PixelFormat::Yuyv };
        let sensor = if infrared { SensorType::Infrared } else { SensorType::Unknown };
        let lit = Frame::new(shape(lit_bytes), w, h, 1, format, lit_seq).with_sensor_type(sensor);
        let unlit = Frame::new(shape(unlit_bytes), w, h, 2, format, unlit_seq)
            .with_sensor_type(sensor);
        let bbox = BoundingBox { x1: coords[0], y1: coords[1], x2: coords[2], y2: coords[3] };
        let lm = [
            [points[0], points[1]],
            [points[2], points[3]],
            [points[4], points[5]],
            [points[6], points[7]],
            [points[8], points[9]],
        ];
        let landmarks = landmarks_of(lm);
        let result = evaluate_ir_active_cues(&lit, &unlit, &bbox, &landmarks, &config);
        assert_clear_is_consistent(&result, &config);
        let _ = ir_active_statistics(&lit, &unlit, &bbox, &landmarks, &config);
    }

    /// One malformation of a fixture pair (zero size, mismatched geometry, wrong length,
    /// non-finite geometry, NaN threshold, wrong format or sensor) always rejects.
    #[test]
    fn prop_ira_active_malformed_input_always_rejects(
        presentation in presentation(),
        kind in 0u8..8,
        index in 0usize..10,
        extra in 1usize..64,
        bad in prop_oneof![Just(f32::NAN), Just(f32::INFINITY), Just(f32::NEG_INFINITY)],
    ) {
        let pair = ir_strobe::active_pair(presentation);
        let mut lit = pair.lit.clone();
        let mut unlit = pair.unlit.clone();
        let mut bbox = face_box(&pair);
        let mut lm = pair.landmarks;
        let mut config = IrActiveConfig::default();
        match kind {
            0 => {
                lit = grey_ir(Vec::new(), 0, 0, 1);
                unlit = grey_ir(Vec::new(), 0, 0, 2);
            }
            1 => {
                let w = ir_strobe::ACTIVE_WIDTH + extra as u32;
                let h = ir_strobe::ACTIVE_HEIGHT;
                unlit = grey_ir(vec![10; (w * h) as usize], w, h, 99);
            }
            2 => {
                let keep = lit.data.len().saturating_sub(extra);
                lit.data.truncate(keep);
            }
            3 => unlit.data.extend(std::iter::repeat_n(7u8, extra)),
            4 => match index % 4 {
                0 => bbox.x1 = bad,
                1 => bbox.y1 = bad,
                2 => bbox.x2 = bad,
                _ => bbox.y2 = bad,
            },
            5 => lm[index / 2][index % 2] = bad,
            6 => match index % 6 {
                0 => config.min_face_response = bad,
                1 => config.min_response_ratio = bad,
                2 => config.max_saturated_fraction = bad,
                3 => config.min_reflectance = bad,
                4 => config.min_relief_ratio = bad,
                _ => config.max_glint_fraction = bad,
            },
            _ => {
                if index % 2 == 0 {
                    lit.format = PixelFormat::Rgb24;
                } else {
                    unlit = unlit.with_sensor_type(SensorType::Rgb);
                }
            }
        }
        let result = evaluate_ir_active_cues(&lit, &unlit, &bbox, &landmarks_of(lm), &config);
        prop_assert!(result.is_err(), "kind {} on {:?} was not rejected", kind, presentation);
    }

    /// Cues only remove: tightening one threshold never turns a rejection into a pass, and an
    /// attack fixture is never accepted by a configuration at least as strict as the default
    /// on every threshold.
    #[test]
    fn prop_ira_active_tightening_never_turns_a_rejection_into_a_pass(
        presentation in presentation(),
        config in valid_config(),
        which in 0u8..7,
    ) {
        prop_assume!(config.validate().is_ok());
        let pair = ir_strobe::active_pair(presentation);
        let before = evaluate_pair(&pair, &config);
        let mut stricter = config;
        match which {
            0 => stricter.min_face_response = (config.min_face_response * 1.5).min(255.0),
            1 => stricter.min_response_ratio = (config.min_response_ratio * 1.5).min(255.0),
            2 => stricter.max_saturated_fraction = config.max_saturated_fraction * 0.5,
            3 => stricter.min_reflectance =
                (config.min_reflectance + 10.0).min(config.max_reflectance - 1.0),
            4 => stricter.min_relief_ratio = (config.min_relief_ratio * 1.5).min(255.0),
            5 => stricter.min_glint_pixels = config.min_glint_pixels.saturating_mul(2).min(4096),
            _ => stricter.max_glint_fraction = (config.max_glint_fraction * 0.5).max(0.0005),
        }
        prop_assume!(stricter.validate().is_ok());
        let after = evaluate_pair(&pair, &stricter);
        if before.is_err() {
            prop_assert!(after.is_err(), "tightening {} accepted {:?}", which, presentation);
        }
        // At least as strict as the default on every threshold (pixel-classification levels
        // kept at their defaults): every attack fixture stays rejected.
        let d = IrActiveConfig::default();
        let combined = IrActiveConfig {
            min_face_response: config.min_face_response.max(d.min_face_response),
            min_response_ratio: config.min_response_ratio.max(d.min_response_ratio),
            saturation_luma: config.saturation_luma.min(d.saturation_luma),
            max_saturated_fraction: config.max_saturated_fraction.min(d.max_saturated_fraction),
            min_reflectance: config.min_reflectance.max(d.min_reflectance),
            max_reflectance: config.max_reflectance.min(d.max_reflectance),
            min_relief_ratio: config.min_relief_ratio.max(d.min_relief_ratio),
            glint_min_luma: d.glint_min_luma,
            glint_min_delta: d.glint_min_delta,
            min_glint_pixels: config.min_glint_pixels.max(d.min_glint_pixels),
            max_glint_fraction: config.max_glint_fraction.min(d.max_glint_fraction),
        };
        if presentation != ActivePresentation::ShadedHead {
            prop_assert!(
                evaluate_pair(&pair, &combined).is_err(),
                "{:?} accepted with {:?}",
                presentation,
                combined
            );
        }
        // Static injection is rejected whatever the thresholds.
        if presentation == ActivePresentation::StaticInjection {
            prop_assert_eq!(before, Err(IrActiveRejection::StaticPair));
        }
    }

    /// A pair whose two frames carry the same bytes is always a static pair, whatever the bytes.
    #[test]
    fn prop_ira_active_identical_random_frames_are_static(
        bytes in prop::collection::vec(any::<u8>(), 160 * 120),
        config in valid_config(),
    ) {
        let lit = grey_ir(bytes.clone(), 160, 120, 1);
        let unlit = grey_ir(bytes, 160, 120, 2);
        let pair = ir_strobe::active_pair(ActivePresentation::ShadedHead);
        let result = evaluate_ir_active_cues(
            &lit,
            &unlit,
            &face_box(&pair),
            &landmarks_of(pair.landmarks),
            &config,
        );
        prop_assert_eq!(result.map(|_| ()), Err(IrActiveRejection::StaticPair));
    }
}

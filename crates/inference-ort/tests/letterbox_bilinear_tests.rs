//! Contract tests for the single production letterbox implementation (GitHub #248, VIS-06).
//!
//! Enforces (verification matrix rows VTD1, VTD2):
//! - The SCRFD input tensor is resampled with bilinear interpolation, never nearest neighbour.
//! - The letterbox padding is an integer offset, and the offsets returned for un-projection
//!   are exactly the offsets at which the image was placed (odd padding included).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "Contract test suite uses direct assertions and pixel indexing"
)]

use soos_inference_ort::detector::{letterbox_pad, unproject};

fn norm(v: f32) -> f32 {
    (v - 127.5) / 128.0
}

/// Red channel plane offset in the NCHW BGR tensor.
fn r_index(target: usize, x: usize, y: usize) -> usize {
    2 * target * target + y * target + x
}

#[test]
fn test_letterbox_odd_padding_unproject_exact() {
    // 640x479 -> 640x640: scale 1.0, 161 rows of padding (odd).
    let (w, h, target) = (640u32, 479u32, 640usize);
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    // Mark source row 0 in red (R = 255), everything else black.
    for x in 0..w as usize {
        rgb[x * 3] = 255;
    }

    let (tensor, scale, pad_x, pad_y) = letterbox_pad(&rgb, w, h, target);
    assert!((scale - 1.0).abs() < 1e-6);
    assert_eq!(pad_x, 0.0);
    assert_eq!(
        pad_y.fract(),
        0.0,
        "the letterbox offset must be an integer, got {pad_y}"
    );

    // The marked source row must land exactly at row `pad_y`.
    let placed_row = (0..target)
        .find(|&y| (tensor[r_index(target, 320, y)] - norm(255.0)).abs() < 1e-4)
        .expect("the marked row must be present in the tensor");
    assert_eq!(placed_row as f32, pad_y);

    // Un-projecting the placed row must give back source row 0 exactly (within 1e-4).
    let (ox, oy) = unproject(320.0, placed_row as f32, scale, pad_x, pad_y);
    assert!((ox - 320.0).abs() < 1e-4);
    assert!(
        oy.abs() < 1e-4,
        "odd padding must round-trip exactly, got source row {oy}"
    );
}

#[test]
fn test_letterbox_pad_is_bilinear_on_downscale() {
    // 960x960 -> 640x640: scale 2/3, so destination column 1 samples source x = 1.5.
    let (w, h, target) = (960u32, 960u32, 640usize);
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            // Column 1 is black, column 2 is R = 200; alternate the rest.
            let v = if x % 3 == 2 { 200u8 } else { 0u8 };
            rgb[(y * w as usize + x) * 3] = v;
        }
    }

    let (tensor, scale, pad_x, pad_y) = letterbox_pad(&rgb, w, h, target);
    assert!((scale - 2.0 / 3.0).abs() < 1e-6);
    assert_eq!((pad_x, pad_y), (0.0, 0.0));

    // Bilinear: 0.5 * 0 + 0.5 * 200 = 100. Nearest neighbour would give 0.
    let got = tensor[r_index(target, 1, 0)];
    assert!(
        (got - norm(100.0)).abs() < 1e-4,
        "destination column 1 must blend source columns 1 and 2 (expected {}, got {got})",
        norm(100.0)
    );
}

#[test]
fn test_letterbox_integer_offsets_for_all_odd_heights() {
    for h in [479u32, 481, 359, 361, 1] {
        let w = 640u32;
        let rgb = vec![10u8; (w * h * 3) as usize];
        let (_, _, pad_x, pad_y) = letterbox_pad(&rgb, w, h, 640);
        assert_eq!(pad_x.fract(), 0.0, "h={h}");
        assert_eq!(pad_y.fract(), 0.0, "h={h}");
    }
}

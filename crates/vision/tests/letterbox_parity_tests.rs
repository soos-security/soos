//! Parity contract between the production SCRFD letterbox (`soos_inference_ort::letterbox_pad`)
//! and `soos_vision::letterbox::letterbox_resize` (GitHub #248, VIS-06; matrix row VTD3).
//!
//! Both entry points must share one implementation: identical geometry (scale and integer
//! offsets) and identical bilinear pixels, so the NGM14 evidence covers the code the daemon runs.

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

use soos_inference_ort::detector::letterbox_pad;
use soos_vision::letterbox::{letterbox_params, letterbox_resize};

fn gradient(w: u32, h: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            rgb.push((x % 256) as u8);
            rgb.push((y % 256) as u8);
            rgb.push(((x + y) % 256) as u8);
        }
    }
    rgb
}

fn assert_parity(w: u32, h: u32) {
    let target = 640usize;
    let rgb = gradient(w, h);

    let (tensor, scale, pad_x, pad_y) = letterbox_pad(&rgb, w, h, target);
    let (resized, params) = letterbox_resize(&rgb, w, h, 640, 640).expect("resize");
    assert_eq!(params, letterbox_params(w, h, 640, 640).expect("params"));

    assert!((params.scale - scale).abs() < 1e-6, "{w}x{h}: scale");
    assert_eq!(params.pad_x, pad_x, "{w}x{h}: pad_x");
    assert_eq!(params.pad_y, pad_y, "{w}x{h}: pad_y");

    let plane = target * target;
    for y in 0..target {
        for x in 0..target {
            let src = (y * target + x) * 3;
            let r = (f32::from(resized[src]) - 127.5) / 128.0;
            let g = (f32::from(resized[src + 1]) - 127.5) / 128.0;
            let b = (f32::from(resized[src + 2]) - 127.5) / 128.0;
            let (tb, tg, tr) = (
                tensor[y * target + x],
                tensor[plane + y * target + x],
                tensor[2 * plane + y * target + x],
            );
            let background = resized[src] == 0 && resized[src + 1] == 0 && resized[src + 2] == 0;
            if background && tb == 0.0 && tg == 0.0 && tr == 0.0 {
                // Padding: the detector tensor uses 0.0 as its border value.
                continue;
            }
            assert!(
                (tb - b).abs() < 1e-4 && (tg - g).abs() < 1e-4 && (tr - r).abs() < 1e-4,
                "{w}x{h}: pixel ({x},{y}) differs: tensor=({tr},{tg},{tb}) resize=({r},{g},{b})"
            );
        }
    }
}

#[test]
fn test_letterbox_pad_matches_vision_resize_on_gradient() {
    assert_parity(1280, 720);
    assert_parity(640, 479);
    assert_parity(1920, 1080);
    assert_parity(479, 640);
}

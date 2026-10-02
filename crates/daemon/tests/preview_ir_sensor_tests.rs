//! GitHub #305 (CAM-NEW-1, matrix row GCV2): the preview never drops the IR sensor stamp.
//!
//! `PreviewResponse` has no sensor field, so a frame stamped `SensorType::Infrared` is sent as
//! greyscale (wire format 1) whatever the pixel format its node streams (YUYV, RGB24, NV12,
//! MJPEG). The GUI then builds a `Grey` frame and takes the Monochrome PAD path with the IR
//! gate and the stricter IR threshold (GitHub #169). Colour sensors keep their wire format.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_camera_v4l::{Frame, PixelFormat, SensorType};
use soos_daemon::preview::{preview_image_for_frame, MAX_PREVIEW_WIDTH, PREVIEW_FORMAT_EMPTY};

const GREY: u8 = 1;

fn frame(data: Vec<u8>, w: u32, h: u32, format: PixelFormat, sensor: SensorType) -> Frame {
    Frame::new(data, w, h, 1_000, format, 7).with_sensor_type(sensor)
}

/// YUYV 4x2 frame with luma `10 * (i + 1)` and constant chroma.
fn yuyv_4x2() -> (Vec<u8>, Vec<u8>) {
    let luma: Vec<u8> = (0..8u8).map(|i| 10 * (i + 1)).collect();
    let mut data = Vec::new();
    for pair in luma.chunks(2) {
        data.extend_from_slice(&[pair[0], 90, pair[1], 160]);
    }
    (data, luma)
}

#[test]
fn test_gcv_ir_yuyv_preview_is_sent_as_grey_luma() {
    let (data, luma) = yuyv_4x2();
    let image =
        preview_image_for_frame(&frame(data, 4, 2, PixelFormat::Yuyv, SensorType::Infrared));
    assert_eq!(image.format, GREY, "IR frames are previewed as Grey");
    assert_eq!((image.width, image.height), (4, 2));
    assert_eq!(image.data, luma, "the luma plane is forwarded verbatim");
}

#[test]
fn test_gcv_ir_nv12_and_rgb_previews_are_grey() {
    // NV12 4x2: luma plane then one interleaved UV row.
    let mut nv12: Vec<u8> = (1..=8u8).collect();
    nv12.extend_from_slice(&[100, 150, 100, 150]);
    let image =
        preview_image_for_frame(&frame(nv12, 4, 2, PixelFormat::Nv12, SensorType::Infrared));
    assert_eq!(image.format, GREY);
    assert_eq!(image.data, (1..=8u8).collect::<Vec<_>>());

    // RGB24 2x1: a grey pixel keeps its value, any pixel becomes one luma byte.
    let rgb = vec![50, 50, 50, 200, 200, 200];
    let image =
        preview_image_for_frame(&frame(rgb, 2, 1, PixelFormat::Rgb24, SensorType::Infrared));
    assert_eq!(image.format, GREY);
    assert_eq!(image.data, vec![50, 200]);
}

#[test]
fn test_gcv_wide_ir_frame_is_downscaled_as_grey() {
    let w = MAX_PREVIEW_WIDTH * 2;
    let h = 4;
    let data = vec![120u8; (w * h * 2) as usize];
    let image =
        preview_image_for_frame(&frame(data, w, h, PixelFormat::Yuyv, SensorType::Infrared));
    assert_eq!(image.format, GREY);
    assert!(image.width <= MAX_PREVIEW_WIDTH);
    assert_eq!(image.data.len(), (image.width * image.height) as usize);
    assert!(image.data.iter().all(|&v| v == 120));
}

#[test]
fn test_gcv_truncated_ir_frame_yields_empty_preview() {
    let image = preview_image_for_frame(&frame(
        vec![1, 2, 3],
        4,
        2,
        PixelFormat::Yuyv,
        SensorType::Infrared,
    ));
    assert_eq!(image.format, PREVIEW_FORMAT_EMPTY);
    assert!(image.data.is_empty());
}

#[test]
fn test_gcv_colour_sensor_preview_keeps_its_wire_format() {
    let (data, _) = yuyv_4x2();
    for sensor in [SensorType::Rgb, SensorType::Unknown] {
        let image = preview_image_for_frame(&frame(data.clone(), 4, 2, PixelFormat::Yuyv, sensor));
        assert_eq!(image.format, 2, "{sensor:?} YUYV stays YUYV");
        assert_eq!(image.data, data);
    }
}

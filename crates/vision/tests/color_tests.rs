//! Tests for camera pixel format color conversions in `soos-vision`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_camera_v4l::PixelFormat;
use soos_vision::{convert_to_rgb, VisionError};

#[test]
fn test_rgb24_passthrough_validation() {
    let width = 4;
    let height = 4;
    let data = vec![128u8; (width * height * 3) as usize];

    let result = convert_to_rgb(&data, width, height, PixelFormat::Rgb24);
    assert!(
        result.is_ok(),
        "RGB24 conversion should succeed for correct buffer size"
    );
    let rgb = result.expect("Valid RGB24");
    assert_eq!(rgb.len(), (width * height * 3) as usize);
    assert_eq!(rgb, data);
}

#[test]
fn test_rgb24_invalid_size_fails_closed() {
    let width = 4;
    let height = 4;
    let data = vec![128u8; 10]; // Too small

    let result = convert_to_rgb(&data, width, height, PixelFormat::Rgb24);
    assert!(matches!(result, Err(VisionError::InvalidBufferSize { .. })));
}

#[test]
fn test_grey_to_rgb_conversion() {
    let width = 2;
    let height = 2;
    let grey_data = vec![50u8, 100u8, 150u8, 200u8];

    let result = convert_to_rgb(&grey_data, width, height, PixelFormat::Grey);
    assert!(result.is_ok(), "Grey conversion should succeed");
    let rgb = result.expect("Valid RGB24");
    assert_eq!(rgb.len(), 12); // 4 pixels * 3 channels

    // Pixel 0: (50, 50, 50)
    assert_eq!(&rgb[0..3], &[50, 50, 50]);
    // Pixel 1: (100, 100, 100)
    assert_eq!(&rgb[3..6], &[100, 100, 100]);
    // Pixel 2: (150, 150, 150)
    assert_eq!(&rgb[6..9], &[150, 150, 150]);
    // Pixel 3: (200, 200, 200)
    assert_eq!(&rgb[9..12], &[200, 200, 200]);
}

#[test]
fn test_grey_invalid_size_fails_closed() {
    let width = 2;
    let height = 2;
    let grey_data = vec![50u8; 3]; // Needs 4 bytes

    let result = convert_to_rgb(&grey_data, width, height, PixelFormat::Grey);
    assert!(matches!(result, Err(VisionError::InvalidBufferSize { .. })));
}

#[test]
fn test_yuyv_to_rgb_neutral_chroma() {
    // 2 pixels YUYV: Y0, U, Y1, V
    // With U=128, V=128 (neutral chroma, grayscale), RGB output should match Y
    let width = 2;
    let height = 1;
    let yuyv = vec![128u8, 128u8, 64u8, 128u8];

    let result = convert_to_rgb(&yuyv, width, height, PixelFormat::Yuyv);
    assert!(result.is_ok(), "YUYV conversion should succeed");
    let rgb = result.expect("Valid RGB24");
    assert_eq!(rgb.len(), 6);

    // Pixel 0: Y0=128, U=128, V=128 -> RGB approximately 128
    for &channel in &rgb[0..3] {
        let diff = (channel as i32 - 128).abs();
        assert!(diff <= 2, "Channel should be close to 128, got {}", channel);
    }

    // Pixel 1: Y1=64, U=128, V=128 -> RGB approximately 64
    for &channel in &rgb[3..6] {
        let diff = (channel as i32 - 64).abs();
        assert!(diff <= 2, "Channel should be close to 64, got {}", channel);
    }
}

#[test]
fn test_yuyv_invalid_size_fails_closed() {
    let width = 2;
    let height = 1;
    let yuyv = vec![128u8; 3]; // Needs 4 bytes

    let result = convert_to_rgb(&yuyv, width, height, PixelFormat::Yuyv);
    assert!(matches!(result, Err(VisionError::InvalidBufferSize { .. })));
}

#[test]
fn test_zero_dimensions_rejected() {
    let data = vec![0u8; 100];
    assert!(matches!(
        convert_to_rgb(&data, 0, 10, PixelFormat::Rgb24),
        Err(VisionError::InvalidDimensions { .. })
    ));
    assert!(matches!(
        convert_to_rgb(&data, 10, 0, PixelFormat::Rgb24),
        Err(VisionError::InvalidDimensions { .. })
    ));
}

#[test]
fn test_mjpeg_corrupt_data_fails_closed() {
    let width = 64;
    let height = 64;
    let corrupt_jpeg = vec![0xFF, 0xD8, 0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF];

    let result = convert_to_rgb(&corrupt_jpeg, width, height, PixelFormat::Mjpeg);
    assert!(result.is_err(), "Corrupt MJPEG stream must fail closed");
}

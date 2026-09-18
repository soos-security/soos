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

#[test]
fn test_nv12_to_rgb_conversion() {
    // 4x4 image in NV12 format
    // Y plane: 16 bytes
    // UV plane: 8 bytes (4 pairs of U, V for four 2x2 blocks)
    // Total size: 24 bytes
    let width = 4;
    let height = 4;
    let mut nv12 = vec![128u8; 16]; // Y plane = 128 (neutral grey)
    nv12.extend_from_slice(&[128u8; 8]); // UV plane = 128 (neutral chroma)

    let result = convert_to_rgb(&nv12, width, height, PixelFormat::Nv12);
    assert!(
        result.is_ok(),
        "NV12 conversion should succeed for valid 4x4 buffer"
    );
    let rgb = result.expect("Valid RGB24");
    assert_eq!(rgb.len(), (width * height * 3) as usize);

    // All pixels should be neutral grey ~128
    for (i, &val) in rgb.iter().enumerate() {
        let diff = (val as i32 - 128).abs();
        assert!(diff <= 2, "Pixel byte {} expected ~128, got {}", i, val);
    }
}

#[test]
fn test_nv12_known_reference_image() {
    // 2x2 image:
    // Y: 4 pixels: [Y00=255, Y01=0, Y10=128, Y11=200]
    // UV: 1 pair: U=128, V=128 (neutral chroma -> R=Y, G=Y, B=Y)
    let width = 2;
    let height = 2;
    let nv12 = vec![
        255, 0, 128, 200, 128, 128, // UV
    ];

    let result = convert_to_rgb(&nv12, width, height, PixelFormat::Nv12);
    assert!(
        result.is_ok(),
        "NV12 known reference conversion should succeed"
    );
    let rgb = result.expect("Valid RGB24");
    assert_eq!(rgb.len(), 12);

    // Pixel (0, 0): Y=255 -> RGB (255, 255, 255)
    assert_eq!(&rgb[0..3], &[255, 255, 255]);
    // Pixel (1, 0): Y=0 -> RGB (0, 0, 0)
    assert_eq!(&rgb[3..6], &[0, 0, 0]);
    // Pixel (0, 1): Y=128 -> RGB (~128, ~128, ~128)
    assert_eq!(&rgb[6..9], &[128, 128, 128]);
    // Pixel (1, 1): Y=200 -> RGB (~200, ~200, ~200)
    assert_eq!(&rgb[9..12], &[200, 200, 200]);
}

#[test]
fn test_nv12_invalid_size_fails_closed() {
    let width = 4;
    let height = 4;
    // Expected 24 bytes, pass 20 bytes
    let nv12_short = vec![128u8; 20];
    let result = convert_to_rgb(&nv12_short, width, height, PixelFormat::Nv12);
    assert!(matches!(result, Err(VisionError::InvalidBufferSize { .. })));

    // Oversized buffer: 30 bytes
    let nv12_long = vec![128u8; 30];
    let result2 = convert_to_rgb(&nv12_long, width, height, PixelFormat::Nv12);
    assert!(matches!(
        result2,
        Err(VisionError::InvalidBufferSize { .. })
    ));
}

#[test]
fn test_nv12_odd_dimensions_rejected() {
    let data = vec![128u8; 100];
    // Odd width or height must fail closed because NV12 requires 2x2 chroma subsampling
    assert!(matches!(
        convert_to_rgb(&data, 3, 4, PixelFormat::Nv12),
        Err(VisionError::InvalidDimensions { .. })
    ));
    assert!(matches!(
        convert_to_rgb(&data, 4, 3, PixelFormat::Nv12),
        Err(VisionError::InvalidDimensions { .. })
    ));
}

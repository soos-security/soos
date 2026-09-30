//! Contract tests for bounded, header-checked MJPEG decoding (review finding VIS-02, GitHub #190).
//!
//! A device-supplied MJPEG frame controls its own SOF header. Before this contract the decoder
//! allocated whatever the header declared (no `read_info` pre-check, unlimited decoding buffer)
//! and only compared decoded byte counts, so transposed frames were accepted and CMYK / 16-bit
//! frames produced buffers of arbitrary size. The contract is:
//! - the compressed input is bounded by [`MAX_MJPEG_COMPRESSED_BYTES`];
//! - the frame header is parsed before any pixel decoding, and its width/height must equal the
//!   dimensions negotiated for the frame, which must not exceed [`MAX_MJPEG_DIMENSION`];
//! - only 8-bit greyscale and YCbCr/RGB JPEGs are accepted; any other decoded format fails closed;
//! - every failure is a typed [`VisionError`], never a panic, and a success always yields exactly
//!   `width * height * 3` bytes.
//!
//! The fixtures below are synthetic 16x8 gradients generated with ImageMagick (non-biometric).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::time::{Duration, Instant};

use proptest::prelude::*;
use soos_camera_v4l::PixelFormat;
use soos_vision::color::{MAX_MJPEG_COMPRESSED_BYTES, MAX_MJPEG_DIMENSION};
use soos_vision::{convert_to_rgb, VisionError};

/// Width of every fixture frame.
const FIXTURE_WIDTH: u32 = 16;
/// Height of every fixture frame.
const FIXTURE_HEIGHT: u32 = 8;

/// Baseline 4:2:0 YCbCr JPEG, 16x8 (`magick -size 16x8 gradient:red-blue -quality 75`).
const RGB_16X8_JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08,
    0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12,
    0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20,
    0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27,
    0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xDB, 0x00, 0x43, 0x01, 0x09, 0x09,
    0x09, 0x0C, 0x0B, 0x0C, 0x18, 0x0D, 0x0D, 0x18, 0x32, 0x21, 0x1C, 0x21, 0x32, 0x32, 0x32, 0x32,
    0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
    0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
    0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0xFF, 0xC0,
    0x00, 0x11, 0x08, 0x00, 0x08, 0x00, 0x10, 0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11,
    0x01, 0xFF, 0xC4, 0x00, 0x15, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0xFF, 0xC4, 0x00, 0x16, 0x10, 0x01, 0x01, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x15, 0x62,
    0xFF, 0xC4, 0x00, 0x15, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x07, 0xFF, 0xC4, 0x00, 0x17, 0x11, 0x00, 0x03, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x16, 0x52,
    0xFF, 0xDA, 0x00, 0x0C, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00, 0x9A, 0x54,
    0xD1, 0x53, 0x40, 0xA3, 0x4C, 0x2D, 0xC0, 0x9D, 0x13, 0x0D, 0x9F, 0xFF, 0xD9,
];

/// Baseline 8-bit greyscale JPEG, 16x8 (`magick -size 16x8 gradient:white-black -colorspace Gray`).
const GREY_16X8_JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08,
    0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12,
    0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20,
    0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27,
    0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x08,
    0x00, 0x10, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x15, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0xFF, 0xC4, 0x00,
    0x17, 0x10, 0x00, 0x03, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x17, 0x64, 0xA2, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00,
    0x8E, 0xEA, 0xB4, 0x1D, 0xD5, 0x68, 0xFF, 0xD9,
];

/// Adobe CMYK JPEG, 16x8 (`magick -size 16x8 gradient:red-blue -colorspace CMYK`).
const CMYK_16X8_JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xEE, 0x00, 0x0E, 0x41, 0x64, 0x6F, 0x62, 0x65, 0x00, 0x64, 0x00, 0x00, 0x00,
    0x00, 0x02, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, 0x07, 0x07,
    0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12, 0x13, 0x0F,
    0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C,
    0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D,
    0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xDB, 0x00, 0x43, 0x01, 0x09, 0x09, 0x09, 0x0C,
    0x0B, 0x0C, 0x18, 0x0D, 0x0D, 0x18, 0x32, 0x21, 0x1C, 0x21, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
    0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
    0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
    0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0xFF, 0xC0, 0x00, 0x14,
    0x08, 0x00, 0x08, 0x00, 0x10, 0x04, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0x04,
    0x22, 0x00, 0xFF, 0xC4, 0x00, 0x16, 0x00, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x07, 0xFF, 0xC4, 0x00, 0x1B, 0x10, 0x01,
    0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x02, 0x05, 0x11, 0x15, 0x26, 0x51, 0xF0, 0xFF, 0xC4, 0x00, 0x15, 0x01, 0x01, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x07, 0xFF, 0xC4,
    0x00, 0x1A, 0x11, 0x00, 0x02, 0x02, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x01, 0x05, 0x16, 0x52, 0x53, 0x91, 0xFF, 0xDA, 0x00, 0x0E, 0x04, 0x01,
    0x00, 0x02, 0x11, 0x03, 0x11, 0x04, 0x00, 0x00, 0x3F, 0x00, 0xB9, 0xB4, 0x4C, 0x77, 0x09, 0x68,
    0x98, 0xEE, 0x10, 0x4B, 0xEC, 0xD2, 0x9B, 0x58, 0x25, 0x52, 0x37, 0x17, 0xD3, 0x49, 0xCB, 0xFA,
    0xA6, 0x5F, 0xD5, 0x07, 0xFF, 0xD9,
];

/// Upper bound on the wall-clock time of a header-only rejection. A full decode of a
/// 65535x65535 header would allocate ~12.9 GB; a header pre-check returns in microseconds.
const HEADER_REJECTION_DEADLINE: Duration = Duration::from_secs(1);

/// Returns a copy of `jpeg` whose baseline SOF0 header declares `width` x `height`.
fn with_sof_dims(jpeg: &[u8], width: u16, height: u16) -> Vec<u8> {
    let mut out = jpeg.to_vec();
    let sof = out
        .windows(2)
        .position(|w| w == [0xFF, 0xC0])
        .expect("fixture carries a baseline SOF0 marker");
    // SOF0 layout: FF C0 | length (2) | precision (1) | height (2, BE) | width (2, BE) | ...
    out[sof + 5..sof + 7].copy_from_slice(&height.to_be_bytes());
    out[sof + 7..sof + 9].copy_from_slice(&width.to_be_bytes());
    out
}

fn expected_rgb_len(width: u32, height: u32) -> usize {
    width as usize * height as usize * 3
}

// ---------------------------------------------------------------------------
// Baseline behaviour that must be preserved
// ---------------------------------------------------------------------------

#[test]
fn test_mjpeg_valid_rgb_frame_decodes_to_exact_rgb_len() {
    let rgb = convert_to_rgb(
        RGB_16X8_JPEG,
        FIXTURE_WIDTH,
        FIXTURE_HEIGHT,
        PixelFormat::Mjpeg,
    )
    .expect("a well-formed 16x8 YCbCr JPEG must decode");
    assert_eq!(rgb.len(), expected_rgb_len(FIXTURE_WIDTH, FIXTURE_HEIGHT));
    // Gradient red -> blue: the first pixel is red dominant, the last pixel blue dominant.
    assert!(
        rgb[0] > rgb[2],
        "first pixel must be red dominant: {:?}",
        &rgb[..3]
    );
    let last = rgb.len() - 3;
    assert!(
        rgb[last + 2] > rgb[last],
        "last pixel must be blue dominant: {:?}",
        &rgb[last..]
    );
}

#[test]
fn test_mjpeg_valid_grey_frame_expands_to_rgb() {
    let rgb = convert_to_rgb(
        GREY_16X8_JPEG,
        FIXTURE_WIDTH,
        FIXTURE_HEIGHT,
        PixelFormat::Mjpeg,
    )
    .expect("a well-formed 16x8 greyscale JPEG must decode");
    assert_eq!(rgb.len(), expected_rgb_len(FIXTURE_WIDTH, FIXTURE_HEIGHT));
    for px in rgb.as_chunks::<3>().0 {
        assert!(
            px[0] == px[1] && px[1] == px[2],
            "greyscale must replicate luma"
        );
    }
}

// ---------------------------------------------------------------------------
// Header / frame dimension agreement
// ---------------------------------------------------------------------------

#[test]
fn test_mjpeg_dimension_mismatch_fails_closed() {
    // Same pixel count (and therefore same decoded byte count), transposed geometry.
    let result = convert_to_rgb(
        RGB_16X8_JPEG,
        FIXTURE_HEIGHT,
        FIXTURE_WIDTH,
        PixelFormat::Mjpeg,
    );
    match result {
        Err(VisionError::MjpegFrameMismatch {
            expected_width,
            expected_height,
            actual_width,
            actual_height,
        }) => {
            assert_eq!((expected_width, expected_height), (8, 16));
            assert_eq!((actual_width, actual_height), (16, 8));
        }
        other => {
            panic!("transposed MJPEG frame must fail closed with MjpegFrameMismatch, got {other:?}")
        }
    }
}

#[test]
fn test_mjpeg_grey_dimension_mismatch_fails_closed() {
    let result = convert_to_rgb(
        GREY_16X8_JPEG,
        FIXTURE_HEIGHT,
        FIXTURE_WIDTH,
        PixelFormat::Mjpeg,
    );
    assert!(
        matches!(result, Err(VisionError::MjpegFrameMismatch { .. })),
        "transposed greyscale MJPEG frame must fail closed, got {result:?}"
    );
}

#[test]
fn test_mjpeg_oversized_header_rejected() {
    let hostile = with_sof_dims(RGB_16X8_JPEG, u16::MAX, u16::MAX);
    let started = Instant::now();
    let result = convert_to_rgb(&hostile, FIXTURE_WIDTH, FIXTURE_HEIGHT, PixelFormat::Mjpeg);
    let elapsed = started.elapsed();
    match result {
        Err(VisionError::MjpegFrameMismatch {
            actual_width,
            actual_height,
            ..
        }) => assert_eq!((actual_width, actual_height), (65_535, 65_535)),
        other => panic!("65535x65535 SOF header must be rejected from the header, got {other:?}"),
    }
    assert!(
        elapsed < HEADER_REJECTION_DEADLINE,
        "header rejection must not decode pixels (took {elapsed:?})"
    );
}

#[test]
fn test_mjpeg_frame_dimensions_above_limit_rejected() {
    // Header and negotiated frame agree, but both exceed the decoder dimension bound.
    let too_wide = u16::try_from(MAX_MJPEG_DIMENSION + 1).expect("bound fits a JPEG header");
    let hostile = with_sof_dims(RGB_16X8_JPEG, too_wide, 8);
    let started = Instant::now();
    let result = convert_to_rgb(&hostile, u32::from(too_wide), 8, PixelFormat::Mjpeg);
    assert!(
        matches!(result, Err(VisionError::InvalidDimensions { .. })),
        "frame wider than MAX_MJPEG_DIMENSION must be rejected, got {result:?}"
    );
    assert!(started.elapsed() < HEADER_REJECTION_DEADLINE);
}

#[test]
fn test_mjpeg_compressed_input_above_limit_rejected() {
    let mut oversized = vec![0u8; MAX_MJPEG_COMPRESSED_BYTES + 1];
    oversized[..RGB_16X8_JPEG.len()].copy_from_slice(RGB_16X8_JPEG);
    let result = convert_to_rgb(
        &oversized,
        FIXTURE_WIDTH,
        FIXTURE_HEIGHT,
        PixelFormat::Mjpeg,
    );
    match result {
        Err(VisionError::MjpegInputTooLarge { max, actual }) => {
            assert_eq!(max, MAX_MJPEG_COMPRESSED_BYTES);
            assert_eq!(actual, MAX_MJPEG_COMPRESSED_BYTES + 1);
        }
        other => panic!("oversized compressed MJPEG must be rejected, got {other:?}"),
    }
}

#[test]
fn test_mjpeg_compressed_input_at_limit_is_not_rejected_for_size() {
    // A frame exactly at the bound is allowed to reach the decoder (trailing bytes after EOI
    // are ignored by the decoder, so this well-formed frame must decode).
    let mut padded = vec![0u8; MAX_MJPEG_COMPRESSED_BYTES];
    padded[..RGB_16X8_JPEG.len()].copy_from_slice(RGB_16X8_JPEG);
    let result = convert_to_rgb(&padded, FIXTURE_WIDTH, FIXTURE_HEIGHT, PixelFormat::Mjpeg);
    assert!(
        !matches!(result, Err(VisionError::MjpegInputTooLarge { .. })),
        "a frame of exactly MAX_MJPEG_COMPRESSED_BYTES must not be rejected for size"
    );
}

#[test]
fn test_mjpeg_limits_are_sane() {
    // 4K UHD frames must be supported; the bound must stay far below the u16 JPEG header range.
    const { assert!(MAX_MJPEG_DIMENSION >= 3840) };
    assert!(MAX_MJPEG_DIMENSION < u32::from(u16::MAX));
    // A compressed frame is never larger than a raw RGB frame of the maximal size.
    let max_rgb = expected_rgb_len(MAX_MJPEG_DIMENSION, MAX_MJPEG_DIMENSION);
    assert!(MAX_MJPEG_COMPRESSED_BYTES <= max_rgb);
    const { assert!(MAX_MJPEG_COMPRESSED_BYTES >= 1024 * 1024) };
}

// ---------------------------------------------------------------------------
// Unsupported decoded pixel formats
// ---------------------------------------------------------------------------

#[test]
fn test_mjpeg_cmyk_frame_rejected() {
    let result = convert_to_rgb(
        CMYK_16X8_JPEG,
        FIXTURE_WIDTH,
        FIXTURE_HEIGHT,
        PixelFormat::Mjpeg,
    );
    assert!(
        matches!(result, Err(VisionError::ColorConversionFailed(_))),
        "UVC MJPEG is always YCbCr; a CMYK frame must fail closed, got {result:?}"
    );
}

#[test]
fn test_mjpeg_empty_and_truncated_input_fail_closed() {
    for len in [0usize, 1, 2, 10, 100, RGB_16X8_JPEG.len() - 2] {
        let result = convert_to_rgb(
            &RGB_16X8_JPEG[..len],
            FIXTURE_WIDTH,
            FIXTURE_HEIGHT,
            PixelFormat::Mjpeg,
        );
        if let Ok(rgb) = &result {
            assert_eq!(rgb.len(), expected_rgb_len(FIXTURE_WIDTH, FIXTURE_HEIGHT));
        }
        if len < 170 {
            assert!(
                result.is_err(),
                "truncated frame of {len} bytes must fail closed"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property tests on the decoder entry point
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Arbitrary bytes (optionally behind a JPEG SOI marker) never panic, and any success
    /// yields exactly `width * height * 3` bytes.
    #[test]
    fn prop_mjpeg_arbitrary_bytes_never_panic(
        body in proptest::collection::vec(any::<u8>(), 0..2048),
        with_soi in any::<bool>(),
        width in 1u32..64,
        height in 1u32..64,
    ) {
        let mut data = if with_soi { vec![0xFF, 0xD8] } else { Vec::new() };
        data.extend_from_slice(&body);
        if let Ok(rgb) = convert_to_rgb(&data, width, height, PixelFormat::Mjpeg) {
            prop_assert_eq!(rgb.len(), expected_rgb_len(width, height));
        }
    }

    /// Mutated real frames with arbitrary SOF dimensions never panic; a header that disagrees
    /// with the negotiated frame is always rejected, and any success has the exact length.
    #[test]
    fn prop_mjpeg_mutated_frames_fail_closed(
        sof_width in any::<u16>(),
        sof_height in any::<u16>(),
        flips in proptest::collection::vec((0usize..RGB_16X8_JPEG.len(), any::<u8>()), 0..8),
        use_grey in any::<bool>(),
    ) {
        let base = if use_grey { GREY_16X8_JPEG } else { RGB_16X8_JPEG };
        let mut data = with_sof_dims(base, sof_width, sof_height);
        for (pos, value) in flips {
            if let Some(slot) = data.get_mut(pos) {
                *slot = value;
            }
        }
        let started = Instant::now();
        let result = convert_to_rgb(&data, FIXTURE_WIDTH, FIXTURE_HEIGHT, PixelFormat::Mjpeg);
        prop_assert!(started.elapsed() < HEADER_REJECTION_DEADLINE);
        if let Ok(rgb) = result {
            prop_assert_eq!(rgb.len(), expected_rgb_len(FIXTURE_WIDTH, FIXTURE_HEIGHT));
        }
    }

    /// Unmutated frames whose SOF declares any geometry other than the negotiated 16x8 are
    /// rejected with `MjpegFrameMismatch` (or an earlier structural error), never accepted.
    #[test]
    fn prop_mjpeg_header_geometry_must_match(
        sof_width in 1u16..=u16::MAX,
        sof_height in 1u16..=u16::MAX,
    ) {
        prop_assume!((u32::from(sof_width), u32::from(sof_height)) != (FIXTURE_WIDTH, FIXTURE_HEIGHT));
        let data = with_sof_dims(RGB_16X8_JPEG, sof_width, sof_height);
        let result = convert_to_rgb(&data, FIXTURE_WIDTH, FIXTURE_HEIGHT, PixelFormat::Mjpeg);
        prop_assert!(
            matches!(result, Err(VisionError::MjpegFrameMismatch { .. })),
            "SOF {}x{} accepted for a 16x8 frame: {:?}", sof_width, sof_height, result.map(|v| v.len())
        );
    }
}

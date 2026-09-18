//! Tests for Sub-issue #22.2: Automatic format negotiation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_camera_v4l::{negotiate_format, CameraError, PixelFormat, FORMAT_PRIORITY};

#[test]
fn test_format_negotiation_prefers_rgb24() {
    let supported = [
        PixelFormat::Grey,
        PixelFormat::Yuyv,
        PixelFormat::Rgb24,
        PixelFormat::Nv12,
        PixelFormat::Mjpeg,
    ];

    // When auto-negotiating (no preferred format), RGB24 has top priority
    let chosen = negotiate_format(&supported, None).expect("Negotiation should succeed");
    assert_eq!(
        chosen,
        PixelFormat::Rgb24,
        "Automatic negotiation must prefer RGB24 when available"
    );
}

#[test]
fn test_format_fallback_on_unsupported() {
    // Device only supports NV12 and Grey
    let supported = [PixelFormat::Nv12, PixelFormat::Grey];

    // Caller preferred Rgb24, but it is unsupported
    let chosen =
        negotiate_format(&supported, Some(PixelFormat::Rgb24)).expect("Fallback should succeed");
    assert_eq!(
        chosen,
        PixelFormat::Nv12,
        "Should fall back to highest-priority supported format (NV12)"
    );

    // Caller preferred Yuyv, but it is unsupported
    let chosen2 =
        negotiate_format(&supported, Some(PixelFormat::Yuyv)).expect("Fallback should succeed");
    assert_eq!(
        chosen2,
        PixelFormat::Nv12,
        "Should fall back to highest-priority supported format (NV12)"
    );
}

#[test]
fn test_format_negotiation_all_priority_order() {
    // Assert the priority order: RGB24 -> YUYV -> NV12 -> MJPEG -> Grey
    assert_eq!(
        FORMAT_PRIORITY,
        [
            PixelFormat::Rgb24,
            PixelFormat::Yuyv,
            PixelFormat::Nv12,
            PixelFormat::Mjpeg,
            PixelFormat::Grey,
        ]
    );

    // Test priority drop-down step by step:
    // Without Rgb24 -> picks Yuyv
    let supp1 = [PixelFormat::Yuyv, PixelFormat::Nv12, PixelFormat::Grey];
    assert_eq!(negotiate_format(&supp1, None).unwrap(), PixelFormat::Yuyv);

    // Without Yuyv -> picks Nv12
    let supp2 = [PixelFormat::Nv12, PixelFormat::Mjpeg, PixelFormat::Grey];
    assert_eq!(negotiate_format(&supp2, None).unwrap(), PixelFormat::Nv12);

    // Without Nv12 -> picks Mjpeg
    let supp3 = [PixelFormat::Mjpeg, PixelFormat::Grey];
    assert_eq!(negotiate_format(&supp3, None).unwrap(), PixelFormat::Mjpeg);

    // Without Mjpeg -> picks Grey
    let supp4 = [PixelFormat::Grey];
    assert_eq!(negotiate_format(&supp4, None).unwrap(), PixelFormat::Grey);
}

#[test]
fn test_format_negotiation_empty_fails() {
    let supported: [PixelFormat; 0] = [];
    let res = negotiate_format(&supported, None);
    assert!(
        matches!(res, Err(CameraError::NoSupportedFormats)),
        "Empty supported formats list must fail closed with NoSupportedFormats"
    );
}

#[test]
fn test_format_negotiation_uses_preferred_when_supported() {
    let supported = [PixelFormat::Rgb24, PixelFormat::Yuyv, PixelFormat::Nv12];
    // Caller explicitly requested Yuyv, and it is supported
    let chosen = negotiate_format(&supported, Some(PixelFormat::Yuyv)).unwrap();
    assert_eq!(chosen, PixelFormat::Yuyv);
}

//! Contract tests of the pure `soos-frame` crate (GitHub #367, spec
//! `AI/architect_spec_macos_port_foundation.md` §2, matrix MAC2): the types moved from
//! `soos-camera-v4l` keep their exact behaviour. The byte-level wipe-on-drop evidence lives in
//! `crates/camera-v4l/tests/frame_drop_wipe_tests.rs` (a test-only allocator probe needs
//! `unsafe`, which business crates never contain).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_frame::{Frame, PixelFormat, SensorType};
use zeroize::Zeroize;

#[test]
fn test_frame_pixel_format_fourcc_strings_are_unchanged() {
    assert_eq!(PixelFormat::Yuyv.fourcc_str(), "YUYV");
    assert_eq!(PixelFormat::Rgb24.fourcc_str(), "RGB3");
    assert_eq!(PixelFormat::Grey.fourcc_str(), "GREY");
    assert_eq!(PixelFormat::Mjpeg.fourcc_str(), "MJPG");
    assert_eq!(PixelFormat::Nv12.fourcc_str(), "NV12");
}

#[test]
fn test_frame_expected_buffer_sizes_are_unchanged_and_saturating() {
    assert_eq!(
        PixelFormat::Yuyv.expected_buffer_size(640, 480),
        Some(614_400)
    );
    assert_eq!(
        PixelFormat::Rgb24.expected_buffer_size(640, 480),
        Some(921_600)
    );
    assert_eq!(
        PixelFormat::Grey.expected_buffer_size(640, 480),
        Some(307_200)
    );
    assert_eq!(
        PixelFormat::Nv12.expected_buffer_size(640, 480),
        Some(460_800)
    );
    assert_eq!(PixelFormat::Mjpeg.expected_buffer_size(640, 480), None);
    assert_eq!(PixelFormat::Grey.expected_buffer_size(0, 480), Some(0));
    // Saturation: never overflows, whatever the platform width of usize.
    let big = PixelFormat::Rgb24.expected_buffer_size(u32::MAX, u32::MAX);
    assert!(big.is_some());
    assert!(
        big.unwrap()
            >= PixelFormat::Grey
                .expected_buffer_size(u32::MAX, u32::MAX)
                .unwrap()
    );
}

#[test]
fn test_frame_new_defaults_to_unknown_sensor_and_keeps_metadata() {
    let frame = Frame::new(vec![1, 2, 3], 3, 1, 42, PixelFormat::Grey, 7);
    assert_eq!(frame.data, [1, 2, 3]);
    assert_eq!((frame.width, frame.height), (3, 1));
    assert_eq!(frame.timestamp_mono_ns, 42);
    assert_eq!(frame.format, PixelFormat::Grey);
    assert_eq!(frame.sequence, 7);
    assert_eq!(frame.sensor_type, SensorType::Unknown);
    let ir = frame.with_sensor_type(SensorType::Infrared);
    assert_eq!(ir.sensor_type, SensorType::Infrared);
    let rgb = ir.clone().with_sensor_type(SensorType::Rgb);
    assert_eq!(rgb.sensor_type, SensorType::Rgb);
    assert_ne!(rgb, ir);
}

#[test]
fn test_frame_age_saturates_and_staleness_is_strict() {
    let frame = Frame::new(Vec::new(), 0, 0, 5_000_000, PixelFormat::Rgb24, 0);
    assert_eq!(frame.age_ms(5_000_000), 0);
    assert_eq!(
        frame.age_ms(4_000_000),
        0,
        "a future timestamp saturates to 0"
    );
    assert_eq!(frame.age_ms(155_999_999), 150);
    assert!(
        !frame.is_stale(155_000_000, 150),
        "exactly at the bound is not stale"
    );
    assert!(frame.is_stale(156_000_000, 150));
    assert_eq!(frame.age_ms(u64::MAX), (u64::MAX - 5_000_000) / 1_000_000);
}

#[test]
fn test_frame_zeroize_empties_data_and_keeps_capacity() {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(&[0xAA_u8; 48]);
    let mut frame = Frame::new(data, 4, 4, 1, PixelFormat::Rgb24, 1);
    let capacity = frame.data.capacity();
    frame.zeroize();
    assert!(frame.data.is_empty(), "zeroize clears the buffer");
    assert_eq!(
        frame.data.capacity(),
        capacity,
        "the wiped allocation is kept"
    );
    // Metadata is not secret and is left untouched.
    assert_eq!((frame.width, frame.height, frame.sequence), (4, 4, 1));
}

#[test]
fn test_frame_types_keep_their_derives() {
    fn assert_traits<T: Clone + std::fmt::Debug + PartialEq + Eq + Send + Sync>() {}
    fn assert_hash_copy<T: Copy + std::hash::Hash>() {}
    assert_traits::<Frame>();
    assert_traits::<PixelFormat>();
    assert_traits::<SensorType>();
    assert_hash_copy::<PixelFormat>();
    assert_hash_copy::<SensorType>();
    assert!(std::mem::needs_drop::<Frame>());
}

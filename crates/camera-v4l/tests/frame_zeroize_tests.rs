//! Tests for memory zeroization of camera frames.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Test suite assertions"
)]

use soos_camera_v4l::{Frame, PixelFormat};
use zeroize::Zeroize;

#[test]
fn test_frame_zeroize_trait() {
    let data = vec![0xAA_u8; 1024];
    let mut frame = Frame::new(data, 640, 480, 1000, PixelFormat::Rgb24, 1);
    assert_eq!(frame.data[0], 0xAA);

    frame.zeroize();
    for (i, &byte) in frame.data.iter().enumerate() {
        assert_eq!(byte, 0, "Frame byte at index {i} was not zeroed");
    }
}

//! Self-tests of the `soos-test-fixtures` generators (GitHub #241, TCI-10): the crate is
//! consumed as a normal dev-dependency, without any `#[path]` include.

#![allow(
    clippy::indexing_slicing,
    reason = "Test assertions index fixed-size fixture buffers"
)]

use soos_camera_v4l::PixelFormat;
use soos_test_fixtures::{onnx, pad, synthetic};

#[test]
fn test_synthetic_frames_have_exact_buffer_sizes() {
    let rgb = synthetic::create_synthetic_rgb_frame(8, 4, [1, 2, 3]);
    assert_eq!(
        (rgb.width, rgb.height, rgb.format),
        (8, 4, PixelFormat::Rgb24)
    );
    assert_eq!(rgb.data.len(), 8 * 4 * 3);
    assert_eq!(&rgb.data[..3], &[1, 2, 3]);

    let yuyv = synthetic::create_synthetic_yuyv_frame(8, 4, 16, 128, 128);
    assert_eq!(yuyv.format, PixelFormat::Yuyv);
    assert_eq!(yuyv.data.len(), 8 * 4 * 2);

    let grey = synthetic::create_synthetic_grey_frame(8, 4, 7);
    assert_eq!(grey.format, PixelFormat::Grey);
    assert!(grey.data.len() == 32 && grey.data.iter().all(|&v| v == 7));
}

#[test]
fn test_pad_presentations_match_requested_geometry_and_differ() {
    let live = pad::create_live_face_frame(64, 48);
    let print = pad::create_printed_photo_frame(64, 48);
    let screen = pad::create_screen_replay_frame(64, 48);
    for frame in [&live, &print, &screen] {
        assert_eq!((frame.width, frame.height), (64, 48));
        assert_eq!(frame.format, PixelFormat::Rgb24);
        assert_eq!(frame.data.len(), 64 * 48 * 3);
    }
    assert_ne!(live.data, print.data);
    assert_ne!(live.data, screen.data);
    assert_ne!(print.data, screen.data);
}

#[test]
fn test_minimal_identity_model_is_deterministic_onnx_bytes() {
    let first = onnx::minimal_identity_model();
    assert_eq!(first, onnx::minimal_identity_model());
    // ModelProto starts with field 1 (ir_version, varint) = 7.
    assert_eq!(&first[..2], &[0x08, 0x07]);
    let text = String::from_utf8_lossy(&first);
    assert!(text.contains("Identity") && text.contains("soos_identity"));
}

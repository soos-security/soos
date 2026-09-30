//! Frame conversion and alignment robustness contracts (GitHub #252, #253, #254).
//!
//! - #253 (VIS-11): YUYV macro-pixels cover two horizontal pixels, so an odd frame width can
//!   never be represented; it must be rejected with `InvalidDimensions` (mirroring NV12)
//!   instead of silently returning a short RGB buffer.
//! - #254 (VIS-12): non-finite landmarks (NaN, infinity, or coordinates whose transform
//!   overflows) must fail alignment with `AlignmentFailed` instead of producing an all-black
//!   112x112 crop that would be embedded (and, during enrollment, stored).
//! - #252 (VIS-10): RGB24 frames are already in the target layout; the borrowing conversion
//!   must not copy them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Test suite utilizes direct assertions and fixture arithmetic"
)]

use std::borrow::Cow;
use std::sync::Arc;

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::{
    BoundingBox, FaceDetection, FaceLandmarks, MockEmbeddingExtractor, MockFaceDetector,
    MockPadDetector, Point2f,
};
use soos_vision::{
    align_face_112, convert_to_rgb, convert_to_rgb_cow, VisionError, VisionPipeline,
    VisionPipelineConfig, TARGET_LANDMARKS_112,
};

// ---------------------------------------------------------------------------
// #253 — YUYV odd width
// ---------------------------------------------------------------------------

#[test]
fn test_yuyv_odd_width_rejected() {
    // width 3, height 1: 6 bytes pass the length check, but only one 4-byte macro-pixel exists.
    let yuyv = vec![128u8; 6];
    let result = convert_to_rgb(&yuyv, 3, 1, PixelFormat::Yuyv);
    assert!(
        matches!(
            result,
            Err(VisionError::InvalidDimensions {
                width: 3,
                height: 1
            })
        ),
        "odd YUYV width must be rejected, got {result:?}"
    );
}

#[test]
fn test_yuyv_odd_width_with_even_pixel_count_rejected() {
    // width 3, height 2: the byte count is a multiple of 4, but macro-pixels straddle rows.
    let yuyv = vec![128u8; 12];
    let result = convert_to_rgb(&yuyv, 3, 2, PixelFormat::Yuyv);
    assert!(
        matches!(
            result,
            Err(VisionError::InvalidDimensions {
                width: 3,
                height: 2
            })
        ),
        "odd YUYV width must be rejected even when width*height is even, got {result:?}"
    );
}

#[test]
fn test_yuyv_even_width_odd_height_accepted() {
    let yuyv = vec![128u8; 2 * 3 * 2];
    let rgb = convert_to_rgb(&yuyv, 2, 3, PixelFormat::Yuyv).expect("even width is valid");
    assert_eq!(rgb.len(), 2 * 3 * 3);
}

// ---------------------------------------------------------------------------
// #252 — borrowing RGB24 conversion
// ---------------------------------------------------------------------------

#[test]
fn test_convert_to_rgb_cow_borrows_rgb24_without_copy() {
    let raw: Vec<u8> = (0..(4 * 2 * 3)).map(|i| i as u8).collect();
    let converted = convert_to_rgb_cow(&raw, 4, 2, PixelFormat::Rgb24).expect("valid RGB24");
    match converted {
        Cow::Borrowed(slice) => {
            assert_eq!(slice.as_ptr(), raw.as_ptr(), "RGB24 must be borrowed");
            assert_eq!(slice, raw.as_slice());
        }
        Cow::Owned(_) => panic!("RGB24 conversion must not copy the frame"),
    }
}

#[test]
fn test_convert_to_rgb_cow_rejects_wrong_rgb24_length() {
    let raw = vec![0u8; 4 * 2 * 3 - 1];
    let result = convert_to_rgb_cow(&raw, 4, 2, PixelFormat::Rgb24);
    assert!(
        matches!(
            result,
            Err(VisionError::InvalidBufferSize {
                expected: 24,
                actual: 23
            })
        ),
        "got {result:?}"
    );
}

#[test]
fn test_convert_to_rgb_cow_converts_other_formats_like_convert_to_rgb() {
    let yuyv = vec![100u8, 90, 150, 200, 30, 128, 60, 128];
    let owned = convert_to_rgb(&yuyv, 2, 2, PixelFormat::Yuyv).expect("valid YUYV");
    let cow = convert_to_rgb_cow(&yuyv, 2, 2, PixelFormat::Yuyv).expect("valid YUYV");
    assert!(matches!(cow, Cow::Owned(_)), "YUYV output is a new buffer");
    assert_eq!(cow.as_ref(), owned.as_slice());

    let grey = vec![7u8; 4];
    let cow = convert_to_rgb_cow(&grey, 2, 2, PixelFormat::Grey).expect("valid Grey");
    assert_eq!(cow.as_ref(), [7u8; 12].as_slice());

    assert!(matches!(
        convert_to_rgb_cow(&yuyv, 0, 2, PixelFormat::Rgb24),
        Err(VisionError::InvalidDimensions { .. })
    ));
}

// ---------------------------------------------------------------------------
// #254 — non-finite landmarks
// ---------------------------------------------------------------------------

fn canonical() -> [Point2f; 5] {
    TARGET_LANDMARKS_112
}

fn landmarks_from(points: [Point2f; 5]) -> FaceLandmarks {
    FaceLandmarks::new(points[0], points[1], points[2], points[3], points[4])
}

fn grey_image() -> Vec<u8> {
    vec![128u8; 112 * 112 * 3]
}

fn assert_alignment_failed(points: [Point2f; 5], label: &str) {
    let result = align_face_112(&grey_image(), 112, 112, &landmarks_from(points));
    assert!(
        matches!(result, Err(VisionError::AlignmentFailed(_))),
        "{label}: expected AlignmentFailed, got {:?}",
        result.map(|crop| crop.len())
    );
}

#[test]
fn test_align_rejects_nan_landmarks() {
    for idx in 0..5 {
        let mut pts = canonical();
        pts[idx].x = f32::NAN;
        assert_alignment_failed(pts, &format!("NaN x at landmark {idx}"));
        let mut pts = canonical();
        pts[idx].y = f32::NAN;
        assert_alignment_failed(pts, &format!("NaN y at landmark {idx}"));
    }
}

#[test]
fn test_align_rejects_infinite_landmarks() {
    let mut pts = canonical();
    pts[2].x = f32::INFINITY;
    assert_alignment_failed(pts, "+inf x");
    let mut pts = canonical();
    pts[4].y = f32::NEG_INFINITY;
    assert_alignment_failed(pts, "-inf y");
}

#[test]
fn test_align_rejects_landmarks_whose_transform_overflows() {
    // Finite but huge coordinates overflow the f32 variance accumulation to infinity.
    let mut pts = canonical();
    pts[0] = Point2f::new(3.0e38, -3.0e38);
    pts[1] = Point2f::new(-3.0e38, 3.0e38);
    assert_alignment_failed(pts, "overflowing coordinates");
}

#[test]
fn test_align_accepts_canonical_landmarks_after_finiteness_guard() {
    let crop = align_face_112(&grey_image(), 112, 112, &landmarks_from(canonical()))
        .expect("canonical landmarks must still align");
    assert_eq!(crop.len(), 112 * 112 * 3);
}

#[test]
fn test_pipeline_rejects_nan_landmarks_before_embedding() {
    let mut pts = canonical();
    pts[1].x = f32::NAN;
    let detection = FaceDetection::with_landmarks(
        BoundingBox::new(20.0, 20.0, 92.0, 92.0),
        0.99,
        landmarks_from(pts),
    );
    let pipeline = VisionPipeline::new(
        Arc::new(MockFaceDetector::new_with_detections(vec![detection])),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig::default(),
    );
    let frame = Frame::new(
        vec![128u8; 320 * 240 * 3],
        320,
        240,
        1_000_000,
        PixelFormat::Rgb24,
        1,
    );
    let result = pipeline.process_frame(&frame);
    assert!(
        matches!(result, Err(VisionError::AlignmentFailed(_))),
        "NaN landmarks must never reach the embedding extractor, got {:?}",
        result.map(|_| ())
    );
}

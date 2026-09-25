//! Contractual test suite for SCRFD face detector with multi-stride output parsing (strides 8, 16, 32).
//!
//! Validates:
//! - Sub-issue #37.1: OrtScrfdDetector struct & FaceDetector trait implementation
//! - Sub-issue #37.2: letterbox_pad utility function with aspect ratio preservation
//! - Sub-issue #37.3: BGR channel ordering and (pixel - 127.5) / 128.0 normalization
//! - Sub-issue #37.4: Multi-stride output tensor parsing across strides 8, 16, 32 with distance-to-border decoding
//! - Sub-issue #37.5: Coordinate un-projection from letterbox space to original image space
//! - Sub-issue #37.6: FaceDetection struct carrying 5-point FaceLandmarks
//! - Sub-issue #37.7: Startup validation rejecting invalid output counts

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::identity_op,
    clippy::erasing_op,
    reason = "Contractual test suite utilizes direct assertions, unwrap, indexing, and geometric offsets"
)]

use proptest::prelude::*;
use soos_inference_ort::detector::{
    letterbox_pad, unproject, BoundingBox, FaceDetection, OrtScrfdDetector,
};
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::landmarks::{FaceLandmarks, Point2f};

#[test]
fn test_face_detection_carries_landmarks() {
    let bbox = BoundingBox::new(50.0, 60.0, 150.0, 180.0);
    let det_without_lmk = FaceDetection::new(bbox, 0.95);
    assert_eq!(det_without_lmk.score, 0.95);
    assert_eq!(det_without_lmk.box_, bbox);
    assert!(det_without_lmk.landmarks.is_none());

    let landmarks = FaceLandmarks::new(
        Point2f::new(75.0, 90.0),
        Point2f::new(125.0, 90.0),
        Point2f::new(100.0, 120.0),
        Point2f::new(80.0, 150.0),
        Point2f::new(120.0, 150.0),
    );

    let det_with_lmk = FaceDetection::with_landmarks(bbox, 0.98, landmarks);
    assert_eq!(det_with_lmk.score, 0.98);
    assert_eq!(det_with_lmk.box_, bbox);
    assert!(det_with_lmk.landmarks.is_some());
    assert_eq!(det_with_lmk.landmarks.unwrap(), landmarks);
}

#[test]
fn test_letterbox_preserves_aspect_ratio() {
    // 640x480 input to 640x640 target
    // Scale = min(640/640, 640/480) = min(1.0, 1.3333) = 1.0
    // new_w = 640, new_h = 480
    // pad_x = (640 - 640)/2 = 0.0
    // pad_y = (640 - 480)/2 = 80.0
    let w = 640u32;
    let h = 480u32;
    let target = 640usize;
    let mut rgb = vec![128u8; (w * h * 3) as usize];

    // Set first pixel of active image to pure Red (R=255, G=0, B=0)
    rgb[0] = 255;
    rgb[1] = 0;
    rgb[2] = 0;

    let (tensor, scale, pad_x, pad_y): (zeroize::Zeroizing<Vec<f32>>, f32, f32, f32) =
        letterbox_pad(&rgb, w, h, target);
    assert!((scale - 1.0).abs() < 1e-5);
    assert!((pad_x - 0.0).abs() < 1e-5);
    assert!((pad_y - 80.0).abs() < 1e-5);
    assert_eq!(tensor.len(), 3 * target * target);

    // Padding check: top 80 rows must be zero (pad value 0)
    for y in 0..80 {
        for x in 0..640 {
            for c in 0..3 {
                let idx = c * target * target + y * target + x;
                assert_eq!(
                    tensor[idx], 0.0,
                    "Padding region at y={y}, x={x} must be 0.0"
                );
            }
        }
    }

    // Active image check: at y=80 (first active row), x=0
    // Channel 0 (B) should be normalized 0 -> (0 - 127.5)/128.0 = -0.99609375
    // Channel 1 (G) should be normalized 0 -> (0 - 127.5)/128.0 = -0.99609375
    // Channel 2 (R) should be normalized 255 -> (255 - 127.5)/128.0 = 0.99609375
    let b_idx = 0 * target * target + 80 * target + 0;
    let g_idx = 1 * target * target + 80 * target + 0;
    let r_idx = 2 * target * target + 80 * target + 0;

    let expected_neg = (0.0 - 127.5) / 128.0;
    let expected_pos = (255.0 - 127.5) / 128.0;
    assert!((tensor[b_idx] - expected_neg).abs() < 1e-4);
    assert!((tensor[g_idx] - expected_neg).abs() < 1e-4);
    assert!((tensor[r_idx] - expected_pos).abs() < 1e-4);
}

#[test]
fn test_prepare_input_bgr_channel_ordering() {
    // 640x640 input, single uniform pixel color: R=200, G=100, B=50
    let w = 640u32;
    let h = 640u32;
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for _ in 0..(w * h) {
        rgb.push(200u8); // R
        rgb.push(100u8); // G
        rgb.push(50u8); // B
    }

    let tensor = OrtScrfdDetector::prepare_input(&rgb, w, h).expect("prepare_input must succeed");
    assert_eq!(tensor.len(), 3 * 640 * 640);

    let expected_b = (50.0 - 127.5) / 128.0;
    let expected_g = (100.0 - 127.5) / 128.0;
    let expected_r = (200.0 - 127.5) / 128.0;

    // Check Channel 0 (B)
    assert!(
        (tensor[0] - expected_b).abs() < 1e-4,
        "Channel 0 must be Blue"
    );
    // Check Channel 1 (G)
    assert!(
        (tensor[640 * 640] - expected_g).abs() < 1e-4,
        "Channel 1 must be Green"
    );
    // Check Channel 2 (R)
    assert!(
        (tensor[2 * 640 * 640] - expected_r).abs() < 1e-4,
        "Channel 2 must be Red"
    );
}

#[test]
fn test_unproject_coordinates_match_original_image() {
    // 1280x720 original frame letterboxed to 640x640:
    // Scale = 640 / 1280 = 0.5
    // new_w = 640, new_h = 360
    // pad_x = 0.0, pad_y = (640 - 360) / 2 = 140.0
    let scale = 0.5f32;
    let pad_x = 0.0f32;
    let pad_y = 140.0f32;

    // A bounding box in letterbox space: x1=100, y1=240, x2=200, y2=340
    let (orig_x1, orig_y1) = unproject(100.0, 240.0, scale, pad_x, pad_y);
    let (orig_x2, orig_y2) = unproject(200.0, 340.0, scale, pad_x, pad_y);

    assert_eq!(orig_x1, 200.0); // (100 - 0) / 0.5
    assert_eq!(orig_y1, 200.0); // (240 - 140) / 0.5
    assert_eq!(orig_x2, 400.0); // (200 - 0) / 0.5
    assert_eq!(orig_y2, 400.0); // (340 - 140) / 0.5

    // Landmark in letterbox space: (150, 290) -> (300, 300)
    let (orig_lm_x, orig_lm_y) = unproject(150.0, 290.0, scale, pad_x, pad_y);
    assert_eq!(orig_lm_x, 300.0);
    assert_eq!(orig_lm_y, 300.0);
}

#[test]
fn test_scrfd_decode_stride8_known_output() {
    // Test pure decoding logic on synthetic stride 8 tensors
    let stride = 8usize;
    let grid_w = 80usize;
    let grid_h = 80usize;
    let anchors_per_cell = 2usize;
    let num_anchors = grid_w * grid_h * anchors_per_cell; // 12800

    let mut scores = vec![-10.0f32; num_anchors]; // low logits (sigmoid(-10) ~ 0)
    let mut bboxes = vec![0.0f32; num_anchors * 4];
    let mut kps = vec![0.0f32; num_anchors * 10];

    // Target detection at row=10, col=20, anchor=0
    let target_idx = (10 * grid_w + 20) * anchors_per_cell + 0;
    scores[target_idx] = 4.0; // sigmoid(4.0) ~ 0.982

    // Distance to border: left=2, top=3, right=4, bottom=5
    bboxes[target_idx * 4 + 0] = 2.0;
    bboxes[target_idx * 4 + 1] = 3.0;
    bboxes[target_idx * 4 + 2] = 4.0;
    bboxes[target_idx * 4 + 3] = 5.0;

    // 5 landmarks offsets relative to (col, row):
    // left eye: (-1.0, -1.0), right eye: (1.0, -1.0), nose: (0.0, 0.0),
    // mouth left: (-1.0, 1.0), mouth right: (1.0, 1.0)
    let kps_offsets = [
        -1.0f32, -1.0, // left eye
        1.0, -1.0, // right eye
        0.0, 0.0, // nose
        -1.0, 1.0, // mouth left
        1.0, 1.0, // mouth right
    ];
    for (i, &val) in kps_offsets.iter().enumerate() {
        kps[target_idx * 10 + i] = val;
    }

    let detections = OrtScrfdDetector::decode_stride(
        stride,
        grid_w,
        grid_h,
        anchors_per_cell,
        &scores,
        &bboxes,
        &kps,
        0.5, // conf_threshold
        1.0, // scale
        0.0, // pad_x
        0.0, // pad_y
        640, // orig_w
        640, // orig_h
    );

    assert_eq!(detections.len(), 1);
    let det = &detections[0];
    assert!(det.score > 0.98);

    // Expected letterbox/orig coords (col=20, row=10, stride=8):
    // x1 = (20 - 2) * 8 = 144
    // y1 = (10 - 3) * 8 = 56
    // x2 = (20 + 4) * 8 = 192
    // y2 = (10 + 5) * 8 = 120
    assert_eq!(det.box_.x1, 144.0);
    assert_eq!(det.box_.y1, 56.0);
    assert_eq!(det.box_.x2, 192.0);
    assert_eq!(det.box_.y2, 120.0);

    // Landmarks:
    let lmk = det.landmarks.expect("landmarks must be present");
    assert_eq!(
        lmk.left_eye,
        Point2f::new((20.0 - 1.0) * 8.0, (10.0 - 1.0) * 8.0)
    ); // (152, 72)
    assert_eq!(
        lmk.right_eye,
        Point2f::new((20.0 + 1.0) * 8.0, (10.0 - 1.0) * 8.0)
    ); // (168, 72)
    assert_eq!(lmk.nose, Point2f::new(20.0 * 8.0, 10.0 * 8.0)); // (160, 80)
    assert_eq!(
        lmk.mouth_left,
        Point2f::new((20.0 - 1.0) * 8.0, (10.0 + 1.0) * 8.0)
    ); // (152, 88)
    assert_eq!(
        lmk.mouth_right,
        Point2f::new((20.0 + 1.0) * 8.0, (10.0 + 1.0) * 8.0)
    ); // (168, 88)
}

#[test]
fn test_scrfd_decode_all_strides() {
    // Decode across strides 8, 16, and 32 with 1 detection per stride
    let strides = [8usize, 16, 32];
    let grid_dims = [(80usize, 80usize), (40, 40), (20, 20)];
    let anchors_per_cell = 2usize;

    let mut stride_tensors = Vec::new();
    for (idx, &stride) in strides.iter().enumerate() {
        let (gw, gh) = grid_dims[idx];
        let num_anchors = gw * gh * anchors_per_cell;
        let mut scores = vec![-10.0f32; num_anchors];
        let mut bboxes = vec![0.0f32; num_anchors * 4];
        let kps = vec![0.0f32; num_anchors * 10];

        // One detection at cell (5, 5), anchor 1
        let t_idx = (5 * gw + 5) * anchors_per_cell + 1;
        scores[t_idx] = 3.0; // sigmoid(3.0) ~ 0.95
        bboxes[t_idx * 4 + 0] = 1.0;
        bboxes[t_idx * 4 + 1] = 1.0;
        bboxes[t_idx * 4 + 2] = 1.0;
        bboxes[t_idx * 4 + 3] = 1.0;

        stride_tensors.push((stride, gw, gh, scores, bboxes, kps));
    }

    let mut all_detections = Vec::new();
    for (stride, gw, gh, scores, bboxes, kps) in &stride_tensors {
        let dets = OrtScrfdDetector::decode_stride(
            *stride,
            *gw,
            *gh,
            anchors_per_cell,
            scores,
            bboxes,
            kps,
            0.5,
            1.0,
            0.0,
            0.0,
            640,
            640,
        );
        all_detections.extend(dets);
    }

    assert_eq!(all_detections.len(), 3);
    // Stride 8: x1 = (5 - 1) * 8 = 32, x2 = (5 + 1) * 8 = 48
    assert_eq!(all_detections[0].box_.x1, 32.0);
    assert_eq!(all_detections[0].box_.x2, 48.0);

    // Stride 16: x1 = (5 - 1) * 16 = 64, x2 = (5 + 1) * 16 = 96
    assert_eq!(all_detections[1].box_.x1, 64.0);
    assert_eq!(all_detections[1].box_.x2, 96.0);

    // Stride 32: x1 = (5 - 1) * 32 = 128, x2 = (5 + 1) * 32 = 192
    assert_eq!(all_detections[2].box_.x1, 128.0);
    assert_eq!(all_detections[2].box_.x2, 192.0);
}

proptest! {
    #[test]
    fn test_letterbox_unproject_roundtrip(
        w in 160u32..1920u32,
        h in 120u32..1080u32,
        target in 320usize..800usize,
    ) {
        let scale = (target as f32 / w as f32).min(target as f32 / h as f32);
        let new_w = (w as f32 * scale).round();
        let new_h = (h as f32 * scale).round();
        let pad_x = ((target as f32 - new_w) / 2.0).max(0.0);
        let pad_y = ((target as f32 - new_h) / 2.0).max(0.0);

        // Test coordinate inside image
        let orig_x = (w as f32) * 0.42;
        let orig_y = (h as f32) * 0.58;

        let proj_x = orig_x * scale + pad_x;
        let proj_y = orig_y * scale + pad_y;

        let (unproj_x, unproj_y): (f32, f32) = unproject(proj_x, proj_y, scale, pad_x, pad_y);

        prop_assert!((unproj_x - orig_x).abs() <= 1.0, "X roundtrip diff > 1px: {} vs {}", unproj_x, orig_x);
        prop_assert!((unproj_y - orig_y).abs() <= 1.0, "Y roundtrip diff > 1px: {} vs {}", unproj_y, orig_y);
    }
}

#[test]
fn test_scrfd_rejects_invalid_output_count() {
    assert!(OrtScrfdDetector::validate_output_count(9).is_ok());
    assert!(matches!(
        OrtScrfdDetector::validate_output_count(2),
        Err(InferenceError::TensorError(_))
    ));
    assert!(matches!(
        OrtScrfdDetector::validate_output_count(0),
        Err(InferenceError::TensorError(_))
    ));
    assert!(matches!(
        OrtScrfdDetector::validate_output_count(10),
        Err(InferenceError::TensorError(_))
    ));
}

#[test]
fn test_scrfd_validates_shape_patterns() {
    let valid_interleaved = vec![
        vec![1, 12800, 1],
        vec![1, 12800, 4],
        vec![1, 12800, 10],
        vec![1, 3200, 1],
        vec![1, 3200, 4],
        vec![1, 3200, 10],
        vec![1, 800, 1],
        vec![1, 800, 4],
        vec![1, 800, 10],
    ];
    assert!(OrtScrfdDetector::validate_output_shapes(&valid_interleaved).is_ok());

    let valid_grouped = vec![
        vec![1, 12800, 1],
        vec![1, 3200, 1],
        vec![1, 800, 1],
        vec![1, 12800, 4],
        vec![1, 3200, 4],
        vec![1, 800, 4],
        vec![1, 12800, 10],
        vec![1, 3200, 10],
        vec![1, 800, 10],
    ];
    assert!(OrtScrfdDetector::validate_output_shapes(&valid_grouped).is_ok());

    let invalid_shapes = vec![
        vec![1, 12800, 1],
        vec![1, 12800, 4],
        vec![1, 12800, 5], // wrong dimension, expected 10
        vec![1, 3200, 1],
        vec![1, 3200, 4],
        vec![1, 3200, 10],
        vec![1, 800, 1],
        vec![1, 800, 4],
        vec![1, 800, 10],
    ];
    assert!(OrtScrfdDetector::validate_output_shapes(&invalid_shapes).is_err());
}

#[test]
fn test_scrfd_supports_preactivated_probabilities() {
    let stride = 8usize;
    let grid_w = 10usize;
    let grid_h = 10usize;
    let anchors_per_cell = 2usize;
    let num_anchors = grid_w * grid_h * anchors_per_cell;

    let mut scores = vec![0.001f32; num_anchors]; // low pre-activated probabilities
    let mut bboxes = vec![0.0f32; num_anchors * 4];
    let kps = vec![0.0f32; num_anchors * 10];

    // High confidence pre-activated probability score
    let target_idx = 5usize;
    scores[target_idx] = 0.7839f32;
    bboxes[target_idx * 4 + 0] = 2.0;
    bboxes[target_idx * 4 + 1] = 2.0;
    bboxes[target_idx * 4 + 2] = 2.0;
    bboxes[target_idx * 4 + 3] = 2.0;

    let detections = OrtScrfdDetector::decode_stride(
        stride,
        grid_w,
        grid_h,
        anchors_per_cell,
        &scores,
        &bboxes,
        &kps,
        0.70, // conf_threshold
        1.0,
        0.0,
        0.0,
        640,
        480,
    );

    assert_eq!(detections.len(), 1);
    assert!((detections[0].score - 0.7839).abs() < 1e-4);
}

//! Contractual test suite for BoundingBox geometry, deterministic NMS, and FaceDetector.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_inference_ort::detector::{nms, BoundingBox, FaceDetection, FaceDetector};
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::landmarks::Point2f;
use soos_inference_ort::mock::MockFaceDetector;

#[test]
fn test_bounding_box_geometry_and_iou() {
    let box1 = BoundingBox::new(10.0, 10.0, 50.0, 50.0);
    assert_eq!(box1.width(), 40.0);
    assert_eq!(box1.height(), 40.0);
    assert_eq!(box1.area(), 1600.0);

    // Identical box -> IoU = 1.0
    assert!((box1.iou(&box1) - 1.0).abs() < 1e-6);

    // Disjoint box -> IoU = 0.0
    let disjoint = BoundingBox::new(100.0, 100.0, 150.0, 150.0);
    assert_eq!(box1.iou(&disjoint), 0.0);

    // Half overlap (20px width overlap out of 40px, same height)
    // Box 1: [10, 10, 50, 50] (area 1600)
    // Box 2: [30, 10, 70, 50] (area 1600)
    // Intersection: [30, 10, 50, 50] -> 20 x 40 = 800
    // Union: 1600 + 1600 - 800 = 2400
    // IoU: 800 / 2400 = 1/3 ≈ 0.333333
    let overlap = BoundingBox::new(30.0, 10.0, 70.0, 50.0);
    let expected_iou = 800.0 / 2400.0;
    assert!((box1.iou(&overlap) - expected_iou).abs() < 1e-6);
}

#[test]
fn test_bounding_box_clamp() {
    let unconstrained = BoundingBox::new(-10.0, -20.0, 650.0, 500.0);
    let clamped = unconstrained.clamp(640.0, 480.0);
    assert_eq!(clamped.x1, 0.0);
    assert_eq!(clamped.y1, 0.0);
    assert_eq!(clamped.x2, 640.0);
    assert_eq!(clamped.y2, 480.0);
}

#[test]
fn test_nms_suppression_and_preservation() {
    // Two highly overlapping detections of the same face
    let primary = FaceDetection::new(BoundingBox::new(100.0, 100.0, 200.0, 200.0), 0.95);
    let redundant = FaceDetection::new(BoundingBox::new(105.0, 105.0, 205.0, 205.0), 0.85);

    // A distinct second face elsewhere in the frame
    let second_face = FaceDetection::new(BoundingBox::new(400.0, 100.0, 500.0, 200.0), 0.90);

    let candidates = vec![redundant.clone(), primary.clone(), second_face.clone()];

    // IoU threshold 0.4
    let selected = nms(&candidates, 0.4);

    assert_eq!(selected.len(), 2);
    // Highest score candidate kept
    assert_eq!(selected[0].score, 0.95);
    assert_eq!(selected[0].box_, primary.box_);
    // Second face kept
    assert_eq!(selected[1].score, 0.90);
    assert_eq!(selected[1].box_, second_face.box_);
}

#[test]
fn test_nms_deterministic_tie_breaking() {
    // Three boxes with EXACTLY identical scores
    let box_a = FaceDetection::new(BoundingBox::new(10.0, 10.0, 50.0, 50.0), 0.90);
    let box_b = FaceDetection::new(BoundingBox::new(100.0, 10.0, 140.0, 50.0), 0.90);
    let box_c = FaceDetection::new(BoundingBox::new(200.0, 10.0, 240.0, 50.0), 0.90);

    // Pass in reverse order
    let candidates = vec![box_c.clone(), box_b.clone(), box_a.clone()];
    let res1 = nms(&candidates, 0.5);

    // Pass in forward order
    let candidates2 = vec![box_a.clone(), box_b.clone(), box_c.clone()];
    let res2 = nms(&candidates2, 0.5);

    // Results must be 100% deterministic regardless of input slice order
    assert_eq!(res1.len(), 3);
    assert_eq!(res2.len(), 3);
    assert_eq!(res1, res2);
    assert_eq!(res1[0].box_, box_a.box_);
    assert_eq!(res1[1].box_, box_b.box_);
    assert_eq!(res1[2].box_, box_c.box_);
}

#[test]
fn test_nms_empty_input() {
    let empty: Vec<FaceDetection> = Vec::new();
    let res = nms(&empty, 0.4);
    assert!(res.is_empty());
}

#[test]
fn test_mock_face_detector_nominal_and_error_handling() {
    let detector = MockFaceDetector::new_centered_face(640, 480, 0.98);

    let dummy_rgb = vec![128u8; 640 * 480 * 3];
    let detections = detector
        .detect(&dummy_rgb, 640, 480)
        .expect("detection should succeed");

    assert_eq!(detections.len(), 1);
    assert_eq!(detections[0].score, 0.98);
    assert_eq!(detections[0].box_.x1, 160.0); // 640 * 0.25
    assert_eq!(detections[0].box_.x2, 480.0); // 640 * 0.75

    // Buffer size mismatch
    let truncated_rgb = vec![128u8; 100];
    let err = detector
        .detect(&truncated_rgb, 640, 480)
        .expect_err("truncated buffer must fail");

    match err {
        InferenceError::InvalidBufferSize { expected, actual } => {
            assert_eq!(expected, 640 * 480 * 3);
            assert_eq!(actual, 100);
        }
        other => panic!("Expected InvalidBufferSize, got {:?}", other),
    }

    // Fault injection
    detector.set_fail_next(true);
    let err2 = detector
        .detect(&dummy_rgb, 640, 480)
        .expect_err("fault injected call must fail");

    match err2 {
        InferenceError::DetectionFailed(msg) => {
            assert!(msg.contains("Simulated"));
        }
        other => panic!("Expected DetectionFailed, got {:?}", other),
    }
}

#[test]
fn test_mock_detector_returns_landmarks() {
    let width = 640;
    let height = 480;
    let detector = MockFaceDetector::new_centered_face(width, height, 0.95);
    let rgb = vec![128u8; (width * height * 3) as usize];
    let detections = detector
        .detect(&rgb, width, height)
        .expect("detection succeeds");

    assert_eq!(detections.len(), 1);
    let det = &detections[0];
    assert!(
        det.landmarks.is_some(),
        "Mock detection must include FaceLandmarks"
    );

    let lm = det.landmarks.unwrap();
    let box_ = det.box_;
    let bw = box_.width();
    let bh = box_.height();
    let scale_x = bw / 112.0;
    let scale_y = bh / 112.0;

    let expected_left_eye = Point2f::new(box_.x1 + 38.2946 * scale_x, box_.y1 + 51.6963 * scale_y);
    let expected_right_eye = Point2f::new(box_.x1 + 73.5318 * scale_x, box_.y1 + 51.5014 * scale_y);
    let expected_nose = Point2f::new(box_.x1 + 56.0252 * scale_x, box_.y1 + 71.7366 * scale_y);
    let expected_mouth_left =
        Point2f::new(box_.x1 + 41.5493 * scale_x, box_.y1 + 92.3655 * scale_y);
    let expected_mouth_right =
        Point2f::new(box_.x1 + 70.7299 * scale_x, box_.y1 + 92.2041 * scale_y);

    assert!(
        (lm.left_eye.x - expected_left_eye.x).abs() < 1e-3,
        "left_eye.x mismatch: got {}, expected {}",
        lm.left_eye.x,
        expected_left_eye.x
    );
    assert!(
        (lm.left_eye.y - expected_left_eye.y).abs() < 1e-3,
        "left_eye.y mismatch: got {}, expected {}",
        lm.left_eye.y,
        expected_left_eye.y
    );
    assert!(
        (lm.right_eye.x - expected_right_eye.x).abs() < 1e-3,
        "right_eye.x mismatch: got {}, expected {}",
        lm.right_eye.x,
        expected_right_eye.x
    );
    assert!(
        (lm.right_eye.y - expected_right_eye.y).abs() < 1e-3,
        "right_eye.y mismatch: got {}, expected {}",
        lm.right_eye.y,
        expected_right_eye.y
    );
    assert!(
        (lm.nose.x - expected_nose.x).abs() < 1e-3,
        "nose.x mismatch: got {}, expected {}",
        lm.nose.x,
        expected_nose.x
    );
    assert!(
        (lm.nose.y - expected_nose.y).abs() < 1e-3,
        "nose.y mismatch: got {}, expected {}",
        lm.nose.y,
        expected_nose.y
    );
    assert!(
        (lm.mouth_left.x - expected_mouth_left.x).abs() < 1e-3,
        "mouth_left.x mismatch: got {}, expected {}",
        lm.mouth_left.x,
        expected_mouth_left.x
    );
    assert!(
        (lm.mouth_left.y - expected_mouth_left.y).abs() < 1e-3,
        "mouth_left.y mismatch: got {}, expected {}",
        lm.mouth_left.y,
        expected_mouth_left.y
    );
    assert!(
        (lm.mouth_right.x - expected_mouth_right.x).abs() < 1e-3,
        "mouth_right.x mismatch: got {}, expected {}",
        lm.mouth_right.x,
        expected_mouth_right.x
    );
    assert!(
        (lm.mouth_right.y - expected_mouth_right.y).abs() < 1e-3,
        "mouth_right.y mismatch: got {}, expected {}",
        lm.mouth_right.y,
        expected_mouth_right.y
    );
}

#[test]
fn test_mock_face_detector_canonical_landmarks_for_box() {
    let box_ = BoundingBox::new(0.0, 0.0, 112.0, 112.0);
    let lm = MockFaceDetector::canonical_landmarks_for_box(&box_);
    assert!((lm.left_eye.x - 38.2946).abs() < 1e-3);
    assert!((lm.left_eye.y - 51.6963).abs() < 1e-3);
    assert!((lm.right_eye.x - 73.5318).abs() < 1e-3);
    assert!((lm.right_eye.y - 51.5014).abs() < 1e-3);
    assert!((lm.nose.x - 56.0252).abs() < 1e-3);
    assert!((lm.nose.y - 71.7366).abs() < 1e-3);
    assert!((lm.mouth_left.x - 41.5493).abs() < 1e-3);
    assert!((lm.mouth_left.y - 92.3655).abs() < 1e-3);
    assert!((lm.mouth_right.x - 70.7299).abs() < 1e-3);
    assert!((lm.mouth_right.y - 92.2041).abs() < 1e-3);
}

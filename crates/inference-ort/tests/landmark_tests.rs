//! Contractual test suite for 5-point facial landmark structures and detector.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_inference_ort::detector::BoundingBox;
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::landmarks::{FaceLandmarks, LandmarkDetector, Point2f};
use soos_inference_ort::mock::MockLandmarkDetector;

#[test]
fn test_point2f_distance() {
    let p1 = Point2f::new(0.0, 0.0);
    let p2 = Point2f::new(3.0, 4.0);
    assert!((p1.distance_to(&p2) - 5.0).abs() < 1e-6);
}

#[test]
fn test_face_landmarks_array_round_trip() {
    let raw = [
        Point2f::new(30.0, 40.0),
        Point2f::new(80.0, 40.0),
        Point2f::new(55.0, 60.0),
        Point2f::new(35.0, 85.0),
        Point2f::new(75.0, 85.0),
    ];

    let landmarks = FaceLandmarks::from_array(raw);
    assert_eq!(landmarks.left_eye, raw[0]);
    assert_eq!(landmarks.right_eye, raw[1]);
    assert_eq!(landmarks.nose, raw[2]);
    assert_eq!(landmarks.mouth_left, raw[3]);
    assert_eq!(landmarks.mouth_right, raw[4]);

    assert_eq!(landmarks.as_array(), raw);
    assert_eq!(landmarks.eye_distance(), 50.0);
    // Perfectly horizontal eyes -> roll angle = 0.0
    assert!((landmarks.roll_angle_rad() - 0.0).abs() < 1e-6);
}

#[test]
fn test_face_landmarks_roll_angle() {
    // 45 degree tilt (dy = dx = 10.0) -> pi / 4 ≈ 0.785398 rad
    let left = Point2f::new(10.0, 10.0);
    let right = Point2f::new(20.0, 20.0);
    let lm = FaceLandmarks::new(
        left,
        right,
        Point2f::new(15.0, 25.0),
        Point2f::new(12.0, 30.0),
        Point2f::new(18.0, 30.0),
    );

    let expected = std::f32::consts::FRAC_PI_4;
    assert!((lm.roll_angle_rad() - expected).abs() < 1e-5);
}

#[test]
fn test_mock_landmark_detector_scaling() {
    let detector = MockLandmarkDetector::new_canonical();
    let dummy_rgb = vec![0u8; 320 * 240 * 3];

    // Bounding box with 2x scale (224x224) starting at (50, 50)
    let bbox = BoundingBox::new(50.0, 50.0, 274.0, 274.0);

    let landmarks = detector
        .detect_landmarks(&dummy_rgb, 320, 240, &bbox)
        .expect("landmark detection failed");

    // Left eye in canonical is (38.29, 51.69)
    // Scaled by 224 / 112 = 2.0 -> (76.58, 103.38)
    // Offset by +50.0 -> (126.58, 153.38)
    assert!((landmarks.left_eye.x - 126.58).abs() < 1e-2);
    assert!((landmarks.left_eye.y - 153.38).abs() < 1e-2);

    // Fault injection
    detector.set_fail_next(true);
    let err = detector
        .detect_landmarks(&dummy_rgb, 320, 240, &bbox)
        .expect_err("fault injected call must fail");

    match err {
        InferenceError::LandmarkFailed(msg) => {
            assert!(msg.contains("Simulated"));
        }
        other => panic!("Expected LandmarkFailed, got: {:?}", other),
    }
}

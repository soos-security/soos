//! Contractual test suite for 5-point facial landmark structures and detector.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_inference_ort::landmarks::{FaceLandmarks, Point2f};

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

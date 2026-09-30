#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_inference_ort::{FaceLandmarks, Point2f};

#[test]
fn test_point2f_geometry_and_distance() {
    let p1 = Point2f::new(0.0, 0.0);
    let p2 = Point2f::new(3.0, 4.0);

    assert_eq!(p1.x, 0.0);
    assert_eq!(p1.y, 0.0);
    assert_eq!(p2.x, 3.0);
    assert_eq!(p2.y, 4.0);

    let dist = p1.distance_to(&p2);
    assert!((dist - 5.0).abs() < 1e-5, "Euclidean distance must be 5.0");
    assert_eq!(p1.distance_to(&p1), 0.0);
}

#[test]
fn test_face_landmarks_array_conversion_and_geometry() {
    let left_eye = Point2f::new(30.0, 40.0);
    let right_eye = Point2f::new(70.0, 40.0);
    let nose = Point2f::new(50.0, 60.0);
    let mouth_left = Point2f::new(35.0, 80.0);
    let mouth_right = Point2f::new(65.0, 80.0);

    let lmk = FaceLandmarks::new(left_eye, right_eye, nose, mouth_left, mouth_right);

    // Assert individual field access
    assert_eq!(lmk.left_eye, left_eye);
    assert_eq!(lmk.right_eye, right_eye);
    assert_eq!(lmk.nose, nose);
    assert_eq!(lmk.mouth_left, mouth_left);
    assert_eq!(lmk.mouth_right, mouth_right);

    // Assert canonical array conversion
    let arr = lmk.as_array();
    assert_eq!(arr.len(), 5);
    assert_eq!(arr[0], left_eye);
    assert_eq!(arr[1], right_eye);
    assert_eq!(arr[2], nose);
    assert_eq!(arr[3], mouth_left);
    assert_eq!(arr[4], mouth_right);

    // Assert roundtrip from array
    let from_arr = FaceLandmarks::from_array(arr);
    assert_eq!(from_arr, lmk);

    // Inter-pupillary eye distance: (70 - 30) = 40.0
    assert!((lmk.eye_distance() - 40.0).abs() < 1e-5);

    // Eye roll angle: horizontal eyes have 0.0 rad roll
    assert!(lmk.roll_angle_rad().abs() < 1e-5);

    // Angled eyes: 45 degree tilt
    let tilted = FaceLandmarks::new(
        Point2f::new(0.0, 0.0),
        Point2f::new(10.0, 10.0),
        nose,
        mouth_left,
        mouth_right,
    );
    let expected_angle = std::f32::consts::FRAC_PI_4; // 45 degrees
    assert!((tilted.roll_angle_rad() - expected_angle).abs() < 1e-5);
}

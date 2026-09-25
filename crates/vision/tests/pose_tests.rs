//! Contractual test suite for facial pose estimation and geometry analysis from 5-point landmarks.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses direct assertions and geometric fixtures"
)]

use soos_inference_ort::landmarks::{FaceLandmarks, Point2f};
use soos_vision::pose::estimate_head_pose;

#[test]
fn test_frontal_pose_estimation() {
    // Perfectly symmetrical frontal face:
    // Left eye at (100, 100), Right eye at (200, 100) -> Roll = 0.0
    // Nose at (150, 150) -> Midpoint between eyes is 150 -> Yaw = 0.0
    // Mouth corners at (110, 200) and (190, 200)
    let landmarks = FaceLandmarks::new(
        Point2f::new(100.0, 100.0),
        Point2f::new(200.0, 100.0),
        Point2f::new(150.0, 150.0),
        Point2f::new(110.0, 200.0),
        Point2f::new(190.0, 200.0),
    );

    let pose = estimate_head_pose(&landmarks);

    // Frontal: yaw, pitch, roll should all be close to 0 degrees
    assert!(
        pose.roll.abs() < 1.0,
        "Expected roll close to 0, got {}",
        pose.roll
    );
    assert!(
        pose.yaw.abs() < 2.0,
        "Expected yaw close to 0, got {}",
        pose.yaw
    );
    assert!(
        pose.pitch.abs() < 5.0,
        "Expected pitch close to 0, got {}",
        pose.pitch
    );
}

#[test]
fn test_roll_head_tilt_estimation() {
    // Head tilted 45 degrees clockwise:
    // delta_y = 100, delta_x = 100 -> atan2(100, 100) = 45.0 degrees
    let landmarks = FaceLandmarks::new(
        Point2f::new(100.0, 100.0),
        Point2f::new(200.0, 200.0),
        Point2f::new(150.0, 200.0),
        Point2f::new(80.0, 180.0),
        Point2f::new(180.0, 280.0),
    );

    let pose = estimate_head_pose(&landmarks);
    assert!(
        (pose.roll - 45.0).abs() < 2.0,
        "Expected roll ~45 deg, got {}",
        pose.roll
    );
}

#[test]
fn test_yaw_left_and_right_turn_estimation() {
    // Face turned to the left (nose shifted toward left eye):
    // Eye midpoint = 150. Nose at 130 (dx = -20)
    let turned_left = FaceLandmarks::new(
        Point2f::new(100.0, 100.0),
        Point2f::new(200.0, 100.0),
        Point2f::new(130.0, 150.0),
        Point2f::new(105.0, 200.0),
        Point2f::new(175.0, 200.0),
    );

    let pose_left = estimate_head_pose(&turned_left);
    assert!(
        pose_left.yaw < -10.0,
        "Expected negative yaw for left turn, got {}",
        pose_left.yaw
    );

    // Face turned to the right (nose shifted toward right eye):
    // Eye midpoint = 150. Nose at 170 (dx = +20)
    let turned_right = FaceLandmarks::new(
        Point2f::new(100.0, 100.0),
        Point2f::new(200.0, 100.0),
        Point2f::new(170.0, 150.0),
        Point2f::new(125.0, 200.0),
        Point2f::new(195.0, 200.0),
    );

    let pose_right = estimate_head_pose(&turned_right);
    assert!(
        pose_right.yaw > 10.0,
        "Expected positive yaw for right turn, got {}",
        pose_right.yaw
    );
}

#[test]
fn test_pitch_up_and_down_estimation() {
    // Face tilted upward (nose moves closer to eye line vertically):
    let tilted_up = FaceLandmarks::new(
        Point2f::new(100.0, 100.0),
        Point2f::new(200.0, 100.0),
        Point2f::new(150.0, 125.0), // closer to eye line (100)
        Point2f::new(110.0, 200.0),
        Point2f::new(190.0, 200.0),
    );

    let pose_up = estimate_head_pose(&tilted_up);
    assert!(
        pose_up.pitch < -5.0,
        "Expected negative pitch for upward tilt, got {}",
        pose_up.pitch
    );

    // Face tilted downward (nose moves closer to mouth line vertically):
    let tilted_down = FaceLandmarks::new(
        Point2f::new(100.0, 100.0),
        Point2f::new(200.0, 100.0),
        Point2f::new(150.0, 175.0), // closer to mouth line (200)
        Point2f::new(110.0, 200.0),
        Point2f::new(190.0, 200.0),
    );

    let pose_down = estimate_head_pose(&tilted_down);
    assert!(
        pose_down.pitch > 5.0,
        "Expected positive pitch for downward tilt, got {}",
        pose_down.pitch
    );
}

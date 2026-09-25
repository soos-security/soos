//! Head pose estimation and face geometry analysis derived from 5-point facial landmarks.
//!
//! Provides deterministic geometric calculation of head rotation angles (Yaw, Pitch, Roll)
//! and alignment quality metrics with zero external neural model overhead.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "Geometric coordinate calculations and angular conversions"
)]

use soos_inference_ort::landmarks::FaceLandmarks;

/// Estimated 3D head rotation angles in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadPose {
    /// Horizontal rotation in degrees.
    /// Negative indicates turned left; positive indicates turned right.
    pub yaw: f32,
    /// Vertical rotation in degrees.
    /// Negative indicates tilted upward; positive indicates tilted downward.
    pub pitch: f32,
    /// In-plane tilt angle in degrees.
    /// 0.0 indicates eyes are horizontally level.
    pub roll: f32,
}

/// Face geometry metrics in pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceGeometry {
    /// Horizontal center of the face (midpoint of eye line).
    pub center_x: f32,
    /// Vertical center of the face.
    pub center_y: f32,
    /// Inter-pupillary Euclidean distance in pixels.
    pub eye_distance: f32,
    /// Estimated face height from eyes to mouth.
    pub face_height: f32,
}

/// Estimates head rotation angles (`yaw`, `pitch`, `roll`) from 5 facial landmarks.
pub fn estimate_head_pose(landmarks: &FaceLandmarks) -> HeadPose {
    let le = &landmarks.left_eye;
    let re = &landmarks.right_eye;
    let nose = &landmarks.nose;
    let ml = &landmarks.mouth_left;
    let mr = &landmarks.mouth_right;

    // 1. Roll: In-plane rotation angle of the eye axis
    let delta_x = re.x - le.x;
    let delta_y = re.y - le.y;
    let roll = delta_y.atan2(delta_x).to_degrees();

    // 2. Inter-pupillary distance & eye midpoint
    let eye_dist = (delta_x * delta_x + delta_y * delta_y).sqrt();
    let eye_mid_x = (le.x + re.x) * 0.5;
    let eye_mid_y = (le.y + re.y) * 0.5;

    // 3. Yaw: Asymmetry of nose tip relative to the eye midpoint
    let yaw = if eye_dist > 1.0 {
        let nose_offset_x = nose.x - eye_mid_x;
        let half_eye_dist = eye_dist * 0.5;
        let ratio = nose_offset_x / half_eye_dist;
        (ratio * 45.0).clamp(-90.0, 90.0)
    } else {
        0.0
    };

    // 4. Pitch: Vertical ratio of nose tip between eye line and mouth line
    let mouth_mid_x = (ml.x + mr.x) * 0.5;
    let mouth_mid_y = (ml.y + mr.y) * 0.5;
    let face_height =
        ((mouth_mid_x - eye_mid_x).powi(2) + (mouth_mid_y - eye_mid_y).powi(2)).sqrt();

    let pitch = if face_height > 1.0 {
        // Vertical distance from eye line to nose
        let nose_vert_dist = nose.y - eye_mid_y;
        let ratio = nose_vert_dist / face_height;
        // Expected frontal baseline ratio is ~0.50
        ((ratio - 0.50) * 80.0).clamp(-60.0, 60.0)
    } else {
        0.0
    };

    HeadPose { yaw, pitch, roll }
}

/// Computes spatial geometry metrics for face positioning and distance evaluation.
pub fn compute_face_geometry(landmarks: &FaceLandmarks) -> FaceGeometry {
    let delta_x = landmarks.right_eye.x - landmarks.left_eye.x;
    let delta_y = landmarks.right_eye.y - landmarks.left_eye.y;
    let eye_distance = (delta_x * delta_x + delta_y * delta_y).sqrt();

    let center_x = (landmarks.left_eye.x + landmarks.right_eye.x) * 0.5;
    let center_y = (landmarks.left_eye.y + landmarks.right_eye.y + landmarks.nose.y) / 3.0;

    let mouth_mid_y = (landmarks.mouth_left.y + landmarks.mouth_right.y) * 0.5;
    let eye_mid_y = (landmarks.left_eye.y + landmarks.right_eye.y) * 0.5;
    let face_height = (mouth_mid_y - eye_mid_y).abs();

    FaceGeometry {
        center_x,
        center_y,
        eye_distance,
        face_height,
    }
}

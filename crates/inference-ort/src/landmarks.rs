//! 5-point facial landmark structures and detector trait.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Facial landmark geometry, coordinate scaling, and pixel buffer calculations"
)]

/// 2D floating-point coordinate representing a landmark point on an image plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2f {
    pub x: f32,
    pub y: f32,
}

impl Point2f {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Computes Euclidean distance to another point.
    pub fn distance_to(&self, other: &Point2f) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// 5-point facial landmarks:
/// 1. Left eye center
/// 2. Right eye center
/// 3. Nose tip
/// 4. Left mouth corner
/// 5. Right mouth corner
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceLandmarks {
    pub left_eye: Point2f,
    pub right_eye: Point2f,
    pub nose: Point2f,
    pub mouth_left: Point2f,
    pub mouth_right: Point2f,
}

impl FaceLandmarks {
    pub fn new(
        left_eye: Point2f,
        right_eye: Point2f,
        nose: Point2f,
        mouth_left: Point2f,
        mouth_right: Point2f,
    ) -> Self {
        Self {
            left_eye,
            right_eye,
            nose,
            mouth_left,
            mouth_right,
        }
    }

    /// Converts the 5 landmark points to an array in fixed canonical order.
    pub fn as_array(&self) -> [Point2f; 5] {
        [
            self.left_eye,
            self.right_eye,
            self.nose,
            self.mouth_left,
            self.mouth_right,
        ]
    }

    /// Constructs `FaceLandmarks` from an array of 5 points.
    pub fn from_array(points: [Point2f; 5]) -> Self {
        let [left_eye, right_eye, nose, mouth_left, mouth_right] = points;
        Self {
            left_eye,
            right_eye,
            nose,
            mouth_left,
            mouth_right,
        }
    }

    /// Returns the inter-pupillary distance between left and right eyes.
    pub fn eye_distance(&self) -> f32 {
        self.left_eye.distance_to(&self.right_eye)
    }

    /// Computes the roll angle in radians between eye centers.
    pub fn roll_angle_rad(&self) -> f32 {
        let dy = self.right_eye.y - self.left_eye.y;
        let dx = self.right_eye.x - self.left_eye.x;
        dy.atan2(dx)
    }
}

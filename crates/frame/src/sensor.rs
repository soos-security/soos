//! Sensor classification tag carried by every frame.

/// Camera sensor classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensorType {
    /// Standard color capture sensor (RGB / YUYV / NV12 / MJPEG).
    Rgb,
    /// Infrared or greyscale-only capture sensor.
    Infrared,
    /// Sensor capability or type could not be determined.
    Unknown,
}

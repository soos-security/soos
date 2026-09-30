//! Camera manager configuration and builder.

use crate::frame::PixelFormat;
use crate::sensor::SensorPreference;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Configuration options for camera streaming, resolution, backoff, and power management.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraConfig {
    /// Device path, preferring persistent `/dev/v4l/by-id/...` nodes over indices.
    pub device_path: PathBuf,
    /// Requested capture width in pixels.
    pub width: u32,
    /// Requested capture height in pixels.
    pub height: u32,
    /// Requested pixel format.
    pub format: PixelFormat,
    /// Enable automatic priority-based format negotiation (RGB24 -> YUYV -> NV12 -> MJPEG -> Grey).
    pub auto_format: bool,
    /// Sensor selection preference on multi-camera hardware (RGB vs IR).
    pub sensor_preference: SensorPreference,
    /// Active capture rate in frames per second.
    pub fps: u32,
    /// Throttled idle rate in frames per second during prolonged inactivity.
    pub idle_fps: u32,
    /// Inactivity threshold before dropping capture rate to `idle_fps`.
    pub idle_timeout: Duration,
    /// Number of initial frames discarded after device startup for auto-exposure stabilization.
    pub warmup_frames: usize,
    /// Initial exponential backoff delay when encountering hardware errors.
    pub min_backoff: Duration,
    /// Maximum ceiling for exponential backoff delay.
    pub max_backoff: Duration,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            device_path: PathBuf::from(crate::resolver::AUTO_CAMERA_DEVICE),
            width: 640,
            height: 480,
            format: PixelFormat::Yuyv,
            auto_format: true,
            sensor_preference: SensorPreference::PreferIr,
            fps: 30,
            idle_fps: 5,
            idle_timeout: Duration::from_secs(10),
            warmup_frames: 20,
            min_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
        }
    }
}

impl CameraConfig {
    /// Returns the explicitly configured device, or `None` when `device_path` is an
    /// auto-detection sentinel (see [`crate::resolver::is_auto_camera_device`]).
    pub fn explicit_device(&self) -> Option<&Path> {
        let path = self.device_path.as_path();
        (!crate::resolver::is_auto_camera_device(path)).then_some(path)
    }
}

/// Builder for constructing customized `CameraConfig` instances.
#[derive(Debug, Default)]
pub struct CameraConfigBuilder {
    config: CameraConfig,
}

impl CameraConfigBuilder {
    /// Creates a new builder initialized with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the camera device path.
    pub fn device_path<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.config.device_path = path.as_ref().to_path_buf();
        self
    }

    /// Sets the target frame resolution.
    pub fn resolution(mut self, width: u32, height: u32) -> Self {
        self.config.width = width;
        self.config.height = height;
        self
    }

    /// Sets the preferred pixel format.
    pub fn format(mut self, format: PixelFormat) -> Self {
        self.config.format = format;
        self
    }

    /// Enables or disables automatic format negotiation.
    pub fn auto_format(mut self, auto: bool) -> Self {
        self.config.auto_format = auto;
        self
    }

    /// Sets the sensor selection preference (RGB vs IR).
    pub fn sensor_preference(mut self, preference: SensorPreference) -> Self {
        self.config.sensor_preference = preference;
        self
    }

    /// Sets the active streaming frames per second.
    pub fn fps(mut self, fps: u32) -> Self {
        self.config.fps = fps.max(1);
        self
    }

    /// Sets the throttled idle frames per second.
    pub fn idle_fps(mut self, idle_fps: u32) -> Self {
        self.config.idle_fps = idle_fps.max(1);
        self
    }

    /// Sets the inactivity timeout before switching to idle FPS.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.config.idle_timeout = timeout;
        self
    }

    /// Sets the number of initial warmup frames to discard for exposure convergence.
    pub fn warmup_frames(mut self, frames: usize) -> Self {
        self.config.warmup_frames = frames;
        self
    }

    /// Sets the minimum and maximum exponential backoff delays.
    pub fn backoff_limits(mut self, min: Duration, max: Duration) -> Self {
        self.config.min_backoff = min;
        self.config.max_backoff = max.max(min);
        self
    }

    /// Finalizes and returns the `CameraConfig`.
    pub fn build(self) -> CameraConfig {
        self.config
    }
}

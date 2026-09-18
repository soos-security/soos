//! `soos-camera-v4l` — V4L2 MMAP warm camera capture manager with lock-free
//! `ArcSwap` snapshotting and mock-camera simulation.

#![deny(clippy::undocumented_unsafe_blocks)]

pub mod config;
pub mod error;
pub mod frame;
pub mod manager;
pub mod mock;
pub mod sensor;
pub mod v4l_impl;

pub use config::{CameraConfig, CameraConfigBuilder};
pub use error::CameraError;
pub use frame::{Frame, PixelFormat};
pub use manager::CameraManager;
pub use mock::MockCameraManager;
pub use sensor::{
    classify_sensor, select_camera_device, CameraDeviceInfo, SensorPreference, SensorType,
};
pub use v4l_impl::{
    fourcc_to_pixel_format, negotiate_format, pixel_format_to_fourcc, V4lCameraManager,
    FORMAT_PRIORITY,
};

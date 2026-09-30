//! `soos-camera-v4l` — V4L2 MMAP warm camera capture manager with lock-free
//! `ArcSwap` snapshotting and mock-camera simulation.

#![deny(clippy::undocumented_unsafe_blocks)]

pub mod capture;
pub mod config;
pub mod error;
pub mod frame;
pub mod manager;
pub mod mock;
pub mod resolver;
pub mod sensor;
pub mod stable_path;
pub mod status;
pub mod v4l_impl;

pub use config::{CameraConfig, CameraConfigBuilder};
pub use error::CameraError;
pub use frame::{Frame, PixelFormat};
pub use manager::{CameraHealth, CameraManager};
pub use mock::MockCameraManager;
pub use resolver::{
    is_auto_camera_device, parse_sensor_preference, resolve_camera_device, CameraEnumerator,
    CameraResolution, CameraResolutionSource, SystemCameraEnumerator, AUTO_CAMERA_DEVICE,
};
pub use sensor::{
    classify_sensor, enumerate_capture_devices, select_camera_device, CameraDeviceInfo,
    SensorPreference, SensorType,
};
pub use stable_path::{stable_device_path, DEFAULT_BY_ID_DIR};
pub use status::{CameraErrorKind, CameraStatus, CameraStatusCell};
pub use v4l_impl::{
    fourcc_to_pixel_format, negotiate_format, pixel_format_to_fourcc, plan_capture, CapturePlan,
    DevicePathResolver, V4lCameraManager, FORMAT_PRIORITY,
};

//! `soos-camera-v4l` — V4L2 MMAP warm camera capture manager with lock-free
//! `ArcSwap` snapshotting and mock-camera simulation.

#![deny(clippy::undocumented_unsafe_blocks)]

pub mod capture;
pub mod config;
pub mod deep_grey;
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
pub use deep_grey::{
    delivered_formats, select_wire_format, CaptureWireFormat, DeepGreyFormat, DEEP_GREY_PRIORITY,
};
pub use error::CameraError;
pub use frame::{Frame, PixelFormat};
pub use manager::{CameraHealth, CameraManager};
pub use mock::MockCameraManager;
pub use resolver::{
    is_auto_camera_device, parse_sensor_preference, resolve_camera_device, CameraEnumerator,
    CameraResolution, CameraResolutionSource, SystemCameraEnumerator, AUTO_CAMERA_DEVICE,
};
pub use sensor::{
    capture_device_from_probe, classify_sensor, classify_sensor_with_hints,
    enumerate_capture_devices, is_ir_frame_size_signature, select_camera_device,
    select_camera_device_with, CameraDeviceInfo, SensorHints, SensorPreference, SensorType,
    IR_SIGNATURE_MAX_HEIGHT, IR_SIGNATURE_MAX_WIDTH, MAX_FRAME_SIZE_HINTS,
};
pub use stable_path::{stable_device_path, DEFAULT_BY_ID_DIR};
pub use status::{CameraErrorKind, CameraStatus, CameraStatusCell};
pub use v4l_impl::{
    fourcc_to_pixel_format, negotiate_format, pixel_format_to_fourcc, plan_capture,
    plan_capture_with_hints, CapturePlan, DevicePathResolver, V4lCameraManager, FORMAT_PRIORITY,
};

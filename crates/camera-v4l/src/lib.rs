//! `soos-camera-v4l` — V4L2 MMAP warm camera capture manager with lock-free
//! `ArcSwap` snapshotting and mock-camera simulation.

#![deny(clippy::undocumented_unsafe_blocks)]

pub mod capture;
pub mod config;
pub mod daemon_config;
pub mod deep_grey;
pub mod diagnostics;
pub mod error;
pub mod frame;
pub mod manager;
pub mod mock;
pub mod resolver;
pub mod sensor;
pub mod stable_path;
pub mod status;
pub mod v4l_guard;
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
    by_id_stem, explain_camera_resolution, is_auto_camera_device, parse_sensor_preference,
    resolve_camera_device, CameraEnumerator, CameraResolution, CameraResolutionReport,
    CameraResolutionSource, CandidateClassification, SelectionReason, SystemCameraEnumerator,
    AUTO_CAMERA_DEVICE,
};
pub use sensor::{
    capture_device_from_probe, classify_sensor, classify_sensor_with_hints,
    enumerate_capture_devices, enumerate_capture_devices_with, explain_sensor_classification,
    is_ir_frame_size_signature, select_camera_device, select_camera_device_with, CameraDeviceInfo,
    ClassificationReason, SensorHints, SensorPreference, SensorType, SystemV4lNodeProbe,
    V4lNodeCapabilities, V4lNodeProbe, IR_SIGNATURE_MAX_HEIGHT, IR_SIGNATURE_MAX_WIDTH,
    MAX_FRAME_SIZE_HINTS, MAX_SYSFS_ENTRIES, MAX_VIDEO_NODES, SYSFS_VIDEO4LINUX_DIR,
};
pub use stable_path::{stable_device_path, DEFAULT_BY_ID_DIR};
pub use status::{CameraErrorKind, CameraStatus, CameraStatusCell};
pub use v4l_impl::{
    fourcc_to_pixel_format, negotiate_format, pixel_format_to_fourcc, plan_capture,
    plan_capture_with_hints, supervisor_sensor_hints, CapturePlan, DevicePathResolver,
    V4lCameraManager, FORMAT_PRIORITY,
};

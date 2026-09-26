//! Sensor classification and multi-camera device selection (RGB vs IR).

use crate::frame::PixelFormat;
use std::path::PathBuf;

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

/// Preference for selecting camera devices on multi-sensor hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SensorPreference {
    /// Prefer RGB color sensor; fallback to Unknown then Infrared if RGB is unavailable.
    #[default]
    PreferRgb,
    /// Prefer Infrared sensor; fallback to Unknown then RGB if Infrared is unavailable.
    PreferIr,
    /// Select the first candidate without sensor-type filtering.
    Any,
}

/// Metadata describing an enumerated video capture device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDeviceInfo {
    /// Filesystem device path (e.g. `/dev/v4l/by-id/...` or `/dev/video0`).
    pub path: PathBuf,
    /// V4L2 device card name from driver capabilities.
    pub card_name: String,
    /// List of pixel formats supported by this device.
    pub supported_formats: Vec<PixelFormat>,
}

impl CameraDeviceInfo {
    /// Classifies the sensor type of this device based on card name and supported formats.
    pub fn sensor_type(&self) -> SensorType {
        classify_sensor(&self.card_name, &self.supported_formats)
    }
}

/// Classifies a camera sensor as RGB, Infrared, or Unknown.
pub fn classify_sensor(card_name: &str, supported_formats: &[PixelFormat]) -> SensorType {
    let lower = card_name.to_ascii_lowercase();

    // Check known infrared markers in V4L2 device names
    if lower.contains("infrared")
        || lower.contains("ir camera")
        || lower.contains("ir-camera")
        || lower.contains(": ir")
        || lower.contains(" ir ")
    {
        return SensorType::Infrared;
    }

    // Check format capabilities: if it supports color formats, classify as RGB
    let has_color_format = supported_formats.iter().any(|f| {
        matches!(
            f,
            PixelFormat::Rgb24 | PixelFormat::Yuyv | PixelFormat::Nv12 | PixelFormat::Mjpeg
        )
    });

    if has_color_format {
        return SensorType::Rgb;
    }

    // If formats are non-empty but lack color (e.g. Grey only), classify as Infrared
    if !supported_formats.is_empty() {
        return SensorType::Infrared;
    }

    SensorType::Unknown
}

/// Selects the best camera device from candidates according to sensor preference.
pub fn select_camera_device(
    devices: &[CameraDeviceInfo],
    preference: SensorPreference,
) -> Option<&CameraDeviceInfo> {
    if devices.is_empty() {
        return None;
    }

    match preference {
        SensorPreference::Any => devices.first(),
        SensorPreference::PreferRgb => devices
            .iter()
            .find(|d| d.sensor_type() == SensorType::Rgb)
            .or_else(|| {
                devices
                    .iter()
                    .find(|d| d.sensor_type() == SensorType::Unknown)
            })
            .or_else(|| devices.first()),
        SensorPreference::PreferIr => devices
            .iter()
            .find(|d| d.sensor_type() == SensorType::Infrared)
            .or_else(|| {
                devices
                    .iter()
                    .find(|d| d.sensor_type() == SensorType::Unknown)
            })
            .or_else(|| devices.first()),
    }
}

/// Enumerates all physical video capture devices, querying capabilities and supported formats.
pub fn enumerate_capture_devices() -> Vec<CameraDeviceInfo> {
    let mut devices = Vec::new();
    let sys_v4l = std::path::Path::new("/sys/class/video4linux");
    if sys_v4l.is_dir() {
        if let Ok(entries) = std::fs::read_dir(sys_v4l) {
            let mut node_names: Vec<_> = entries
                .filter_map(|e| {
                    e.ok()
                        .map(|ent| ent.file_name().to_string_lossy().into_owned())
                })
                .filter(|name| name.starts_with("video"))
                .collect();
            // Sort deterministically (video0, video1, video2...)
            node_names.sort_by_key(|n| {
                n.strip_prefix("video")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(u32::MAX)
            });

            for name in node_names {
                let dev_path = PathBuf::from(format!("/dev/{name}"));
                if let Ok(dev) = v4l::Device::with_path(&dev_path) {
                    if let Ok(caps) = dev.query_caps() {
                        if caps
                            .capabilities
                            .contains(v4l::capability::Flags::VIDEO_CAPTURE)
                        {
                            let enum_fmts =
                                v4l::video::Capture::enum_formats(&dev).unwrap_or_default();
                            let supported_formats: Vec<PixelFormat> = enum_fmts
                                .into_iter()
                                .filter_map(|desc| {
                                    crate::v4l_impl::fourcc_to_pixel_format(desc.fourcc)
                                })
                                .collect();

                            devices.push(CameraDeviceInfo {
                                path: dev_path,
                                card_name: caps.card,
                                supported_formats,
                            });
                        }
                    }
                }
            }
        }
    }
    devices
}

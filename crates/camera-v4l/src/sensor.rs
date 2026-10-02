//! Sensor classification and multi-camera device selection (RGB vs IR).

use crate::frame::PixelFormat;
use std::path::{Path, PathBuf};

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
    /// Prefer Infrared sensor; fallback to Unknown then RGB if Infrared is unavailable.
    #[default]
    PreferIr,
    /// Prefer RGB color sensor; fallback to Unknown then Infrared if RGB is unavailable.
    PreferRgb,
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

    /// Classifies the sensor type of this device with additional [`SensorHints`]
    /// (by-id link name, frame sizes), see [`classify_sensor_with_hints`].
    pub fn sensor_type_with_hints(&self, hints: &SensorHints) -> SensorType {
        classify_sensor_with_hints(&self.card_name, &self.supported_formats, hints)
    }
}

/// Upper bound on the number of frame sizes kept in [`SensorHints::frame_sizes`].
pub const MAX_FRAME_SIZE_HINTS: usize = 64;

/// Largest frame width of the IR frame-size signature (see [`is_ir_frame_size_signature`]).
pub const IR_SIGNATURE_MAX_WIDTH: u32 = 640;

/// Largest frame height of the IR frame-size signature (see [`is_ir_frame_size_signature`]).
pub const IR_SIGNATURE_MAX_HEIGHT: u32 = 400;

/// Classification signals beyond the (31-byte, often identical) V4L2 card name
/// (review finding CAM-13, GitHub #195).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SensorHints {
    /// File name of the persistent `/dev/v4l/by-id/` link of the node, when one exists. It is
    /// built by udev from the untruncated USB product string (e.g. `..._Integrated_IR_Camera_...`).
    pub by_id_name: Option<String>,
    /// Frame sizes (`width`, `height`) enumerated with `VIDIOC_ENUM_FRAMESIZES`, bounded by
    /// [`MAX_FRAME_SIZE_HINTS`]. Empty when unknown.
    pub frame_sizes: Vec<(u32, u32)>,
}

/// Returns `true` when `name` contains a whole `IR` token or the word `infrared`.
///
/// Tokens are separated by any non-alphanumeric character, so `Integrated_IR_Camera`,
/// `IR Camera` and `usb-Cam_IR-video-index0` match while `Chicony` or `Firmware` do not.
pub(crate) fn has_ir_token(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("infrared")
        || lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| token == "ir")
}

/// Card-name IR markers, including the 31-byte truncation of `" IR"` to `" I"`.
fn card_name_has_ir_marker(card_name: &str) -> bool {
    let lower = card_name.to_ascii_lowercase();
    // Note: V4L2 caps.card is capped at 31 characters, so device names like
    // "USB2.0 FHD UVC WebCam: USB2.0 IR" are truncated to "USB2.0 FHD UVC WebCam: USB2.0 I".
    lower.contains("infrared")
        || lower.contains("ir camera")
        || lower.contains("ir-camera")
        || lower.contains(": ir")
        || lower.contains(" ir ")
        || lower.ends_with(": ir")
        || lower.ends_with(" i")
        || has_ir_token(card_name)
}

/// Returns `true` when every enumerated frame size is at most
/// [`IR_SIGNATURE_MAX_WIDTH`] x [`IR_SIGNATURE_MAX_HEIGHT`] (e.g. 340x340, 400x400, 640x360),
/// the typical signature of a Windows-Hello IR module. An empty list is not a signature, and
/// any VGA (640x480) or larger size rules it out.
pub fn is_ir_frame_size_signature(frame_sizes: &[(u32, u32)]) -> bool {
    !frame_sizes.is_empty()
        && frame_sizes
            .iter()
            .all(|&(w, h)| w <= IR_SIGNATURE_MAX_WIDTH && h <= IR_SIGNATURE_MAX_HEIGHT)
}

fn has_colour_format(supported_formats: &[PixelFormat]) -> bool {
    supported_formats.iter().any(|f| {
        matches!(
            f,
            PixelFormat::Rgb24 | PixelFormat::Yuyv | PixelFormat::Nv12 | PixelFormat::Mjpeg
        )
    })
}

/// Classifies a camera sensor as RGB, Infrared, or Unknown from its card name and formats.
///
/// Equivalent to [`classify_sensor_with_hints`] with empty [`SensorHints`].
pub fn classify_sensor(card_name: &str, supported_formats: &[PixelFormat]) -> SensorType {
    classify_sensor_with_hints(card_name, supported_formats, &SensorHints::default())
}

/// Classifies a camera sensor with an ordered scorer (strongest signal first):
///
/// 1. an `IR` token in the by-id link name,
/// 2. an IR marker in the card name,
/// 3. a non-empty format list without any colour format (greyscale only),
/// 4. the IR frame-size signature ([`is_ir_frame_size_signature`]),
/// 5. any colour format means [`SensorType::Rgb`]; nothing at all means [`SensorType::Unknown`].
pub fn classify_sensor_with_hints(
    card_name: &str,
    supported_formats: &[PixelFormat],
    hints: &SensorHints,
) -> SensorType {
    explain_sensor_classification(card_name, supported_formats, hints).0
}

/// Rule of the ordered scorer that decided a classification (GitHub #256, CAM-17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassificationReason {
    /// Rule 1: a whole `IR` token or `infrared` in the by-id link name.
    ByIdIrToken,
    /// Rule 2: an IR marker in the (possibly truncated) card name.
    CardNameIrMarker,
    /// Rule 3: a non-empty format list without any colour format.
    GreyscaleOnlyFormats,
    /// Rule 4: the IR frame-size signature ([`is_ir_frame_size_signature`]).
    IrFrameSizeSignature,
    /// Rule 5: at least one colour format.
    ColourFormats,
    /// No rule applied (no format, no hint): [`SensorType::Unknown`].
    NoSignal,
}

impl ClassificationReason {
    /// Stable snake_case label used by `soos-admin camera` and in logs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ByIdIrToken => "by_id_ir_token",
            Self::CardNameIrMarker => "card_name_ir_marker",
            Self::GreyscaleOnlyFormats => "greyscale_only_formats",
            Self::IrFrameSizeSignature => "ir_frame_size_signature",
            Self::ColourFormats => "colour_formats",
            Self::NoSignal => "no_signal",
        }
    }
}

/// Same ordered scorer as [`classify_sensor_with_hints`], also returning the rule that decided.
pub fn explain_sensor_classification(
    card_name: &str,
    supported_formats: &[PixelFormat],
    hints: &SensorHints,
) -> (SensorType, ClassificationReason) {
    if hints.by_id_name.as_deref().is_some_and(has_ir_token) {
        return (SensorType::Infrared, ClassificationReason::ByIdIrToken);
    }
    if card_name_has_ir_marker(card_name) {
        return (SensorType::Infrared, ClassificationReason::CardNameIrMarker);
    }
    let has_colour = has_colour_format(supported_formats);
    if !supported_formats.is_empty() && !has_colour {
        return (
            SensorType::Infrared,
            ClassificationReason::GreyscaleOnlyFormats,
        );
    }
    if is_ir_frame_size_signature(&hints.frame_sizes) {
        return (
            SensorType::Infrared,
            ClassificationReason::IrFrameSizeSignature,
        );
    }
    if has_colour {
        return (SensorType::Rgb, ClassificationReason::ColourFormats);
    }
    (SensorType::Unknown, ClassificationReason::NoSignal)
}

/// Selects the best camera device from candidates according to sensor preference.
pub fn select_camera_device(
    devices: &[CameraDeviceInfo],
    preference: SensorPreference,
) -> Option<&CameraDeviceInfo> {
    select_camera_device_with(devices, preference, CameraDeviceInfo::sensor_type)
}

/// Selects the best camera device using a caller-supplied classifier (e.g. one that applies
/// [`SensorHints`]); the fallback order is the one of [`select_camera_device`].
pub fn select_camera_device_with<F>(
    devices: &[CameraDeviceInfo],
    preference: SensorPreference,
    classify: F,
) -> Option<&CameraDeviceInfo>
where
    F: Fn(&CameraDeviceInfo) -> SensorType,
{
    let wanted = match preference {
        SensorPreference::Any => return devices.first(),
        SensorPreference::PreferRgb => SensorType::Rgb,
        SensorPreference::PreferIr => SensorType::Infrared,
    };
    devices
        .iter()
        .find(|d| classify(d) == wanted)
        .or_else(|| devices.iter().find(|d| classify(d) == SensorType::Unknown))
        .or_else(|| devices.first())
}

/// Builds the enumeration entry of one probed node, or `None` when it must be skipped.
///
/// A node is kept only when it has the `VIDEO_CAPTURE` capability and delivers at least one
/// pixel format ([`crate::delivered_formats`]); deep-greyscale IR formats (Y8I, Y10, Y12, Y16)
/// count as [`PixelFormat::Grey`], so Y16-only IR nodes are no longer invisible (GitHub #195).
pub fn capture_device_from_probe(
    path: PathBuf,
    card_name: String,
    has_video_capture: bool,
    fourccs: &[v4l::FourCC],
) -> Option<CameraDeviceInfo> {
    if !has_video_capture {
        return None;
    }
    let supported_formats = crate::deep_grey::delivered_formats(fourccs);
    if supported_formats.is_empty() {
        return None;
    }
    Some(CameraDeviceInfo {
        path,
        card_name,
        supported_formats,
    })
}

/// Enumerates the frame sizes of an open device for every fourcc, bounded by
/// [`MAX_FRAME_SIZE_HINTS`] distinct sizes in total. Stepwise ranges contribute their maximum
/// size only.
///
/// Each fourcc is walked through the guarded, bounded `VIDIOC_ENUM_FRAMESIZES` wrapper: at most
/// [`MAX_FRAME_SIZE_HINTS`] indices per fourcc (a fourcc cannot contribute more distinct sizes
/// than the total bound), so a driver that never reports the end of its list cannot loop
/// forever. A truncated list keeps its first indices and is logged at debug level with the
/// node path only.
pub(crate) fn device_frame_sizes(
    device: &v4l::Device,
    device_path: &Path,
    fourccs: &[v4l::FourCC],
) -> Vec<(u32, u32)> {
    use v4l::framesize::FrameSizeEnum;

    let max_indices = u32::try_from(MAX_FRAME_SIZE_HINTS).unwrap_or(u32::MAX);
    let mut sizes: Vec<(u32, u32)> = Vec::new();
    for &fourcc in fourccs {
        let Ok(found) = crate::v4l_guard::enum_framesizes_guarded(device, fourcc, max_indices)
        else {
            continue;
        };
        if found.truncated {
            tracing::debug!(
                "Size enumeration on '{}' stopped at the bound of {} entries for one pixel format",
                device_path.display(),
                max_indices
            );
        }
        for frame_size in found.items {
            let size = match frame_size {
                FrameSizeEnum::Discrete(d) => (d.width, d.height),
                FrameSizeEnum::Stepwise(s) => (s.max_width, s.max_height),
            };
            if !sizes.contains(&size) {
                if sizes.len() >= MAX_FRAME_SIZE_HINTS {
                    return sizes;
                }
                sizes.push(size);
            }
        }
    }
    sizes
}

/// Opens `path` and enumerates its frame sizes (empty on any error).
pub(crate) fn frame_sizes_at(path: &std::path::Path) -> Vec<(u32, u32)> {
    let Ok(guarded) = crate::v4l_guard::open_device_guarded(path) else {
        return Vec::new();
    };
    let Some(device) = guarded.get() else {
        return Vec::new();
    };
    let fourccs = crate::v4l_guard::enum_formats_guarded(device, path);
    device_frame_sizes(device, path, &fourccs)
}

/// Driver names (`v4l2_capability.driver`, compared trimmed and case-insensitively) of
/// virtual capture devices whose frames any local process can write: v4l2loopback (which
/// reports `"v4l2 loopback"`), the akvcam virtual webcam and the vivid / vimc test drivers
/// (GitHub #307, CAM-NEW-3).
pub const VIRTUAL_CAPTURE_DRIVERS: [&str; 5] =
    ["v4l2 loopback", "v4l2loopback", "akvcam", "vivid", "vimc"];

/// `device_caps` bits of a node that accepts frames from user space: `VIDEO_OUTPUT`,
/// `VBI_OUTPUT`, `SLICED_VBI_OUTPUT`, `VIDEO_OUTPUT_OVERLAY`, `VIDEO_OUTPUT_MPLANE`,
/// `SDR_OUTPUT` and `META_OUTPUT`.
pub const V4L2_OUTPUT_CAPABLE_CAPS: u32 =
    0x0000_0002 | 0x0000_0020 | 0x0000_0080 | 0x0000_0200 | 0x0000_2000 | 0x0040_0000 | 0x0800_0000;

/// `device_caps` bits of a memory-to-memory node (`VIDEO_M2M_MPLANE`, `VIDEO_M2M`): its
/// "capture" queue returns what user space wrote to its output queue.
pub const V4L2_MEM_TO_MEM_CAPS: u32 = 0x0000_4000 | 0x0000_8000;

/// Why a V4L2 node is never used as a biometric camera (GitHub #307, CAM-NEW-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VirtualNodeRejection {
    /// Memory-to-memory node (`VIDEO_M2M` / `VIDEO_M2M_MPLANE`).
    MemToMem,
    /// The node also accepts frames from user space (an output capability bit).
    OutputCapable,
    /// The driver is a virtual capture driver ([`VIRTUAL_CAPTURE_DRIVERS`]).
    VirtualDriver,
}

impl VirtualNodeRejection {
    /// Stable snake_case label (`soos-admin camera list` status).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MemToMem => "rejected_mem_to_mem",
            Self::OutputCapable => "rejected_output_capable",
            Self::VirtualDriver => "rejected_virtual_driver",
        }
    }

    /// Human-readable reason, used in logs and errors.
    pub const fn description(self) -> &'static str {
        match self {
            Self::MemToMem => "memory-to-memory node (its frames come from user space)",
            Self::OutputCapable => "the node accepts frames written by user space (output capability)",
            Self::VirtualDriver => {
                "virtual capture driver (v4l2loopback or vivid): any local process can inject frames"
            }
        }
    }
}

/// Returns why a node with `driver` and `device_caps` must not be used as a biometric camera,
/// or `None` for a real capture device. Applied by the enumeration (auto-selection and
/// re-resolution), the diagnostics and, unless `CameraConfig::allow_virtual_device` is set,
/// the capture supervisor when it opens any device, explicit or not.
pub fn virtual_node_rejection(driver: &str, device_caps: u32) -> Option<VirtualNodeRejection> {
    if device_caps & V4L2_MEM_TO_MEM_CAPS != 0 {
        return Some(VirtualNodeRejection::MemToMem);
    }
    if device_caps & V4L2_OUTPUT_CAPABLE_CAPS != 0 {
        return Some(VirtualNodeRejection::OutputCapable);
    }
    let driver = driver.trim();
    VIRTUAL_CAPTURE_DRIVERS
        .iter()
        .any(|virtual_driver| driver.eq_ignore_ascii_case(virtual_driver))
        .then_some(VirtualNodeRejection::VirtualDriver)
}

/// Sysfs directory listing the V4L2 device nodes.
pub const SYSFS_VIDEO4LINUX_DIR: &str = "/sys/class/video4linux";

/// Upper bound on the sysfs entries read by one enumeration (bounded directory scan).
pub const MAX_SYSFS_ENTRIES: usize = 256;

/// Upper bound on the `video<N>` nodes opened and queried by one enumeration.
pub const MAX_VIDEO_NODES: usize = 64;

/// Capabilities reported by one V4L2 node (`VIDIOC_QUERYCAP` + `VIDIOC_ENUM_FMT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V4lNodeCapabilities {
    /// Driver card name (`v4l2_capability.card`, at most 31 characters).
    pub card_name: String,
    /// Whether the node advertises `V4L2_CAP_VIDEO_CAPTURE`.
    pub video_capture: bool,
    /// Pixel formats the node can deliver (unknown FourCCs removed, deep greyscale as `Grey`).
    pub supported_formats: Vec<PixelFormat>,
    /// Driver name (`v4l2_capability.driver`), checked against [`VIRTUAL_CAPTURE_DRIVERS`].
    pub driver: String,
    /// Capabilities of this node (`v4l2_capability.device_caps`), checked for output and
    /// memory-to-memory bits (GitHub #307).
    pub device_caps: u32,
}

/// Queries the capabilities of one V4L2 node.
///
/// Production uses [`SystemV4lNodeProbe`]; tests inject a fixture table so enumeration is
/// hermetic (GitHub #198, review finding CAM-16).
pub trait V4lNodeProbe {
    /// Returns the node capabilities, or `None` when the node cannot be opened or queried
    /// (`EACCES`, `ENODEV`, vanished node, ioctl failure).
    fn probe(&self, dev_path: &Path) -> Option<V4lNodeCapabilities>;
}

/// Probe backed by the real V4L2 ioctls of the `v4l` crate.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemV4lNodeProbe;

impl V4lNodeProbe for SystemV4lNodeProbe {
    fn probe(&self, dev_path: &Path) -> Option<V4lNodeCapabilities> {
        // Guarded open: a NUL byte in the path is refused instead of panicking in `v4l`, and
        // the descriptor is closed through the guard (GitHub #314, CAM-NEW-4).
        let guarded = crate::v4l_guard::open_device_guarded(dev_path).ok()?;
        let dev = guarded.get()?;
        // `v4l` 0.14 panics on non-UTF-8 capability strings: such a node is skipped like any
        // node whose ioctls fail, instead of unwinding through the daemon (GitHub #287).
        let caps = crate::v4l_guard::query_caps_guarded(dev).ok()?;
        let video_capture = caps
            .capabilities
            .contains(v4l::capability::Flags::VIDEO_CAPTURE);
        // Deep-greyscale IR formats (Y8I, Y10, Y12, Y16) are delivered as Grey, so Y16-only IR
        // nodes stay visible (GitHub #195).
        let supported_formats = if video_capture {
            let fourccs = crate::v4l_guard::enum_formats_guarded(dev, dev_path);
            crate::deep_grey::delivered_formats(&fourccs)
        } else {
            Vec::new()
        };
        Some(V4lNodeCapabilities {
            card_name: caps.card,
            video_capture,
            supported_formats,
            driver: caps.driver,
            device_caps: caps.capabilities.bits(),
        })
    }
}

/// Parses a sysfs entry name `video<decimal>` into its index (`None` for any other entry).
fn video_node_index(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("video")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Returns the `video<N>` entry names of `sysfs_dir` in numeric order, reading at most
/// [`MAX_SYSFS_ENTRIES`] entries and returning at most [`MAX_VIDEO_NODES`] names. A missing or
/// unreadable directory yields an empty list. Shared by the enumeration and the diagnostics.
pub(crate) fn scan_video_node_names(sysfs_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(sysfs_dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<(u32, String)> = entries
        .take(MAX_SYSFS_ENTRIES)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            video_node_index(&name).map(|idx| (idx, name))
        })
        .collect();
    nodes.sort();
    nodes
        .into_iter()
        .take(MAX_VIDEO_NODES)
        .map(|(_, name)| name)
        .collect()
}

/// Enumerates the video capture nodes listed in `sysfs_dir`, probing `dev_dir/<node>`.
///
/// Only `video<N>` entries are considered, in numeric order, at most [`MAX_SYSFS_ENTRIES`]
/// entries are read and at most [`MAX_VIDEO_NODES`] nodes are probed. A node is listed only
/// when it advertises `V4L2_CAP_VIDEO_CAPTURE` **and** at least one decodable pixel format, so
/// uvcvideo metadata nodes and codec nodes are never candidates, and when
/// [`virtual_node_rejection`] accepts it: virtual drivers (v4l2loopback, vivid) and nodes with
/// an output or memory-to-memory capability are never auto-selected (GitHub #307). A missing
/// or unreadable `sysfs_dir` yields an empty list.
pub fn enumerate_capture_devices_with(
    sysfs_dir: &Path,
    dev_dir: &Path,
    probe: &dyn V4lNodeProbe,
) -> Vec<CameraDeviceInfo> {
    scan_video_node_names(sysfs_dir)
        .into_iter()
        .filter_map(|name| {
            let path = dev_dir.join(name);
            let caps = probe.probe(&path)?;
            if !caps.video_capture || caps.supported_formats.is_empty() {
                return None;
            }
            if let Some(rejection) = virtual_node_rejection(&caps.driver, caps.device_caps) {
                tracing::debug!(
                    node = %path.display(),
                    reason = rejection.as_str(),
                    "Skipping V4L2 node that is not a physical camera"
                );
                return None;
            }
            Some(CameraDeviceInfo {
                path,
                card_name: caps.card_name,
                supported_formats: caps.supported_formats,
            })
        })
        .collect()
}

/// Enumerates all physical video capture devices, querying capabilities and supported formats.
///
/// Thin wrapper over [`enumerate_capture_devices_with`] reading [`SYSFS_VIDEO4LINUX_DIR`] and
/// probing `/dev/video<N>` with [`SystemV4lNodeProbe`].
pub fn enumerate_capture_devices() -> Vec<CameraDeviceInfo> {
    enumerate_capture_devices_with(
        Path::new(SYSFS_VIDEO4LINUX_DIR),
        Path::new("/dev"),
        &SystemV4lNodeProbe,
    )
}

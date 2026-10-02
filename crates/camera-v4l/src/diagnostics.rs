//! Metadata-only camera diagnostics for `soos-admin camera list|probe` (GitHub #256, CAM-17).
//!
//! Every V4L2 node listed in sysfs is queried through the injectable [`V4lDeviceProbe`] trait
//! (production: [`SystemV4lDeviceProbe`], tests: fixture tables), and the shared resolver
//! ([`explain_camera_resolution`]) decides which node soos would use and why. Only metadata
//! ioctls are issued: `VIDIOC_QUERYCAP`, `VIDIOC_ENUM_FMT` and `VIDIOC_ENUM_FRAMESIZES`. No
//! format is negotiated, no buffer is mapped, no stream is started and no frame is read, so the
//! diagnostics never touch biometric data and never disturb a daemon that is streaming.

use crate::deep_grey::delivered_formats;
use crate::frame::PixelFormat;
use crate::resolver::{
    explain_camera_resolution, CameraEnumerator, CameraResolution, SelectionReason,
};
use crate::sensor::{
    device_frame_sizes, explain_sensor_classification, scan_video_node_names,
    virtual_node_rejection, CameraDeviceInfo, ClassificationReason, SensorHints, SensorPreference,
    SensorType, VirtualNodeRejection, MAX_FRAME_SIZE_HINTS,
};
use std::path::{Path, PathBuf};

/// Upper bound on the fourccs kept per node (`VIDIOC_ENUM_FMT` results).
pub const MAX_DIAGNOSTIC_FOURCCS: usize = 64;

/// Upper bound on the characters kept from a driver-supplied string (card, driver, bus_info are
/// 32-byte fields in `struct v4l2_capability`).
pub const MAX_V4L_TEXT_CHARS: usize = 32;

/// `V4L2_CAP_VIDEO_CAPTURE`.
pub const V4L2_CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
/// `V4L2_CAP_VIDEO_OUTPUT`.
pub const V4L2_CAP_VIDEO_OUTPUT: u32 = 0x0000_0002;
/// `V4L2_CAP_VIDEO_OVERLAY`.
pub const V4L2_CAP_VIDEO_OVERLAY: u32 = 0x0000_0004;
/// `V4L2_CAP_VBI_CAPTURE`.
pub const V4L2_CAP_VBI_CAPTURE: u32 = 0x0000_0010;
/// `V4L2_CAP_VIDEO_CAPTURE_MPLANE`.
pub const V4L2_CAP_VIDEO_CAPTURE_MPLANE: u32 = 0x0000_1000;
/// `V4L2_CAP_VIDEO_M2M_MPLANE`.
pub const V4L2_CAP_VIDEO_M2M_MPLANE: u32 = 0x0000_4000;
/// `V4L2_CAP_VIDEO_M2M`.
pub const V4L2_CAP_VIDEO_M2M: u32 = 0x0000_8000;
/// `V4L2_CAP_EXT_PIX_FORMAT`.
pub const V4L2_CAP_EXT_PIX_FORMAT: u32 = 0x0020_0000;
/// `V4L2_CAP_META_CAPTURE`.
pub const V4L2_CAP_META_CAPTURE: u32 = 0x0080_0000;
/// `V4L2_CAP_READWRITE`.
pub const V4L2_CAP_READWRITE: u32 = 0x0100_0000;
/// `V4L2_CAP_STREAMING`.
pub const V4L2_CAP_STREAMING: u32 = 0x0400_0000;
/// `V4L2_CAP_TOUCH`.
pub const V4L2_CAP_TOUCH: u32 = 0x1000_0000;
/// `V4L2_CAP_IO_MC`.
pub const V4L2_CAP_IO_MC: u32 = 0x2000_0000;
/// `V4L2_CAP_DEVICE_CAPS`.
pub const V4L2_CAP_DEVICE_CAPS: u32 = 0x8000_0000;

/// Named `device_caps` bits, in ascending bit order.
const DEVICE_CAPS_NAMES: [(u32, &str); 14] = [
    (V4L2_CAP_VIDEO_CAPTURE, "VIDEO_CAPTURE"),
    (V4L2_CAP_VIDEO_OUTPUT, "VIDEO_OUTPUT"),
    (V4L2_CAP_VIDEO_OVERLAY, "VIDEO_OVERLAY"),
    (V4L2_CAP_VBI_CAPTURE, "VBI_CAPTURE"),
    (V4L2_CAP_VIDEO_CAPTURE_MPLANE, "VIDEO_CAPTURE_MPLANE"),
    (V4L2_CAP_VIDEO_M2M_MPLANE, "VIDEO_M2M_MPLANE"),
    (V4L2_CAP_VIDEO_M2M, "VIDEO_M2M"),
    (V4L2_CAP_EXT_PIX_FORMAT, "EXT_PIX_FORMAT"),
    (V4L2_CAP_META_CAPTURE, "META_CAPTURE"),
    (V4L2_CAP_READWRITE, "READWRITE"),
    (V4L2_CAP_STREAMING, "STREAMING"),
    (V4L2_CAP_TOUCH, "TOUCH"),
    (V4L2_CAP_IO_MC, "IO_MC"),
    (V4L2_CAP_DEVICE_CAPS, "DEVICE_CAPS"),
];

/// Returns the names of the known `device_caps` bits set in `caps`, in ascending bit order.
pub fn device_caps_flag_names(caps: u32) -> Vec<&'static str> {
    DEVICE_CAPS_NAMES
        .iter()
        .filter(|(bit, _)| caps & bit == *bit)
        .map(|(_, name)| *name)
        .collect()
}

/// Renders a fourcc for display: printable ASCII kept, anything else shown as `.`, trailing
/// padding spaces removed (`b"Y16 "` becomes `Y16`).
pub fn fourcc_label(fourcc: [u8; 4]) -> String {
    let text: String = fourcc
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                char::from(b)
            } else {
                '.'
            }
        })
        .collect();
    text.trim_end_matches(' ').to_string()
}

/// Returns `true` for characters that could alter a terminal or reorder displayed text.
fn is_unsafe_display_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Makes a device-supplied string safe to print: control characters and bidirectional
/// overrides become `?`, and at most [`MAX_V4L_TEXT_CHARS`] characters are kept. A USB device
/// chooses its own product string, so it must never reach a terminal unfiltered.
pub fn sanitize_v4l_text(text: &str) -> String {
    sanitize_display_text(&text.chars().take(MAX_V4L_TEXT_CHARS).collect::<String>())
}

/// Replaces control characters and bidirectional overrides with `?` without truncating (used
/// for paths such as by-id link names, which udev derives from device-supplied strings).
pub fn sanitize_display_text(text: &str) -> String {
    text.chars()
        .map(|c| if is_unsafe_display_char(c) { '?' } else { c })
        .collect()
}

/// Metadata of one V4L2 node (`VIDIOC_QUERYCAP`, `VIDIOC_ENUM_FMT`, `VIDIOC_ENUM_FRAMESIZES`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V4lNodeDetails {
    /// Driver name (`v4l2_capability.driver`).
    pub driver: String,
    /// Card name (`v4l2_capability.card`, at most 31 characters, often truncated).
    pub card_name: String,
    /// Bus location (`v4l2_capability.bus_info`).
    pub bus_info: String,
    /// Capabilities of this node (`v4l2_capability.device_caps`).
    pub device_caps: u32,
    /// Every enumerated fourcc, including the ones soos cannot decode.
    pub fourccs: Vec<[u8; 4]>,
    /// Enumerated frame sizes (a stepwise range contributes its maximum only).
    pub frame_sizes: Vec<(u32, u32)>,
}

/// Why a node could not be probed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProbeFailure {
    /// `EACCES` / `EPERM`: the caller may not open the node (not root, not in `video`).
    PermissionDenied,
    /// `EBUSY`: the driver refused a second open.
    Busy,
    /// `ENOENT` / `ENODEV` / `ENXIO`: the node vanished or was never there.
    NotFound,
    /// Any other failure, with its errno when there is one.
    Other(Option<i32>),
}

impl ProbeFailure {
    /// Classifies an I/O error from `open(2)` or an ioctl.
    pub fn from_io_error(error: &std::io::Error) -> Self {
        match error.raw_os_error() {
            Some(libc::EACCES | libc::EPERM) => Self::PermissionDenied,
            Some(libc::EBUSY) => Self::Busy,
            Some(libc::ENOENT | libc::ENODEV | libc::ENXIO) => Self::NotFound,
            errno => Self::Other(errno),
        }
    }

    /// Stable snake_case label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::Busy => "busy",
            Self::NotFound => "not_found",
            Self::Other(_) => "io_error",
        }
    }
}

/// Queries the metadata of one V4L2 node. Tests inject fixture tables (GitHub #198, CAM-16).
pub trait V4lDeviceProbe {
    /// Returns the node metadata or why it could not be read.
    fn details(&self, dev_path: &Path) -> Result<V4lNodeDetails, ProbeFailure>;
}

/// Probe backed by the real `VIDIOC_QUERYCAP`, `VIDIOC_ENUM_FMT` and `VIDIOC_ENUM_FRAMESIZES`
/// ioctls of the `v4l` crate. It never sets a format and never streams.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemV4lDeviceProbe;

impl V4lDeviceProbe for SystemV4lDeviceProbe {
    fn details(&self, dev_path: &Path) -> Result<V4lNodeDetails, ProbeFailure> {
        // `v4l` 0.14 unwraps `str::from_utf8` on the capability strings, so a device reporting
        // a non-UTF-8 card name would panic inside the crate: report it as a probe failure.
        // The individual ioctls are guarded as well; this outer guard also covers the
        // frame-size enumeration (shared `crate::v4l_guard`, GitHub #287).
        crate::v4l_guard::guard_v4l_call(|| system_details(dev_path))
            .unwrap_or(Err(ProbeFailure::Other(None)))
    }
}

/// Character-device major number of every V4L2 node (`Documentation/admin-guide/devices.txt`).
const V4L2_CHAR_MAJOR: u32 = 81;

/// Refuses any path that is not a V4L2 character device before `open(2)`.
///
/// `soos-admin camera probe <DEVICE>` often runs as root and accepts an arbitrary path;
/// opening some other device nodes has side effects (a tape drive rewinds on close, a tty
/// changes its modem lines). A refused path reports `ENOTTY`, the errno the V4L2 ioctls would
/// return on it.
fn ensure_v4l2_char_device(dev_path: &Path) -> Result<(), ProbeFailure> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let metadata = std::fs::metadata(dev_path).map_err(|e| ProbeFailure::from_io_error(&e))?;
    if metadata.file_type().is_char_device() && libc::major(metadata.rdev()) == V4L2_CHAR_MAJOR {
        Ok(())
    } else {
        Err(ProbeFailure::Other(Some(libc::ENOTTY)))
    }
}

/// Body of [`SystemV4lDeviceProbe::details`] (metadata ioctls only).
fn system_details(dev_path: &Path) -> Result<V4lNodeDetails, ProbeFailure> {
    ensure_v4l2_char_device(dev_path)?;
    let guarded = crate::v4l_guard::open_device_guarded(dev_path)
        .map_err(|e| ProbeFailure::from_io_error(&e))?;
    let device = guarded.get().ok_or(ProbeFailure::NotFound)?;
    let caps = crate::v4l_guard::query_caps_guarded(device)
        .map_err(|e| ProbeFailure::from_io_error(&e))?;
    let mut fourccs: Vec<v4l::FourCC> = crate::v4l_guard::enum_formats_guarded(device, dev_path);
    fourccs.truncate(MAX_DIAGNOSTIC_FOURCCS);
    let frame_sizes = device_frame_sizes(device, dev_path, &fourccs);
    Ok(V4lNodeDetails {
        driver: caps.driver,
        card_name: caps.card,
        bus_info: caps.bus,
        device_caps: caps.capabilities.bits(),
        fourccs: fourccs.iter().map(|f| f.repr).collect(),
        frame_sizes,
    })
}

/// Whether a node is a capture candidate for soos, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeStatus {
    /// `VIDEO_CAPTURE` with at least one decodable pixel format: the resolver considers it.
    Candidate,
    /// No `V4L2_CAP_VIDEO_CAPTURE` (uvcvideo metadata node, output or codec node).
    NotVideoCapture,
    /// Capture node without any pixel format soos can decode.
    NoDecodableFormat,
    /// Capture node that is not a physical camera (virtual driver, output or memory-to-memory
    /// capability, GitHub #307); never auto-selected.
    Rejected(VirtualNodeRejection),
    /// The node could not be opened or queried.
    ProbeFailed(ProbeFailure),
}

impl NodeStatus {
    /// Stable snake_case label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::NotVideoCapture => "not_video_capture",
            Self::NoDecodableFormat => "no_decodable_format",
            Self::Rejected(reason) => reason.as_str(),
            Self::ProbeFailed(_) => "probe_failed",
        }
    }
}

/// Diagnostics of one V4L2 node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeDiagnostics {
    /// Kernel node path (`/dev/videoN`).
    pub node: PathBuf,
    /// Persistent `/dev/v4l/by-id/` alias, when one resolves to the node.
    pub by_id_alias: Option<PathBuf>,
    /// Sanitized, bounded metadata (`None` when the probe failed).
    pub details: Option<V4lNodeDetails>,
    /// Pixel formats soos can deliver from this node (deep greyscale counted as `Grey`).
    pub supported_formats: Vec<PixelFormat>,
    /// Candidate status.
    pub status: NodeStatus,
    /// Sensor type the resolver assigned (candidates only).
    pub sensor_type: Option<SensorType>,
    /// Scorer rule that decided `sensor_type` (candidates only).
    pub classification_reason: Option<ClassificationReason>,
    /// `true` when the by-id `IR` token was not used because another capture node shares its
    /// stem (see [`crate::resolver::by_id_stem`]).
    pub by_id_hint_ignored: bool,
}

/// Full diagnostics: every node plus the shared resolver's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDiagnostics {
    /// Sensor preference the decision was made with.
    pub preference: SensorPreference,
    /// Every `video<N>` node listed in sysfs, in numeric order (bounded by `MAX_VIDEO_NODES`).
    pub nodes: Vec<NodeDiagnostics>,
    /// The decision, identical to `resolve_camera_device` over the same inventory.
    pub resolution: CameraResolution,
    /// Kernel node of the auto-selected device (`None` for an explicit device or no node).
    pub selected_node: Option<PathBuf>,
    /// Why this device was chosen.
    pub reason: SelectionReason,
}

/// Bounds and sanitizes probe output before it is stored or rendered.
fn bounded_details(mut details: V4lNodeDetails) -> V4lNodeDetails {
    details.driver = sanitize_v4l_text(&details.driver);
    details.card_name = sanitize_v4l_text(&details.card_name);
    details.bus_info = sanitize_v4l_text(&details.bus_info);
    details.fourccs.truncate(MAX_DIAGNOSTIC_FOURCCS);
    details.frame_sizes.truncate(MAX_FRAME_SIZE_HINTS);
    details
}

/// Derives the decodable formats and the candidate status of probed metadata.
fn status_of(details: &V4lNodeDetails) -> (Vec<PixelFormat>, NodeStatus) {
    if details.device_caps & V4L2_CAP_VIDEO_CAPTURE == 0 {
        return (Vec::new(), NodeStatus::NotVideoCapture);
    }
    let fourccs: Vec<v4l::FourCC> = details.fourccs.iter().map(v4l::FourCC::new).collect();
    let formats = delivered_formats(&fourccs);
    // Same rule as the enumeration the resolver consults (GitHub #307).
    if let Some(rejection) = virtual_node_rejection(&details.driver, details.device_caps) {
        return (formats, NodeStatus::Rejected(rejection));
    }
    if formats.is_empty() {
        (formats, NodeStatus::NoDecodableFormat)
    } else {
        (formats, NodeStatus::Candidate)
    }
}

/// Probes `path` and builds its unclassified diagnostics entry.
fn probe_node(node: PathBuf, probe: &dyn V4lDeviceProbe) -> NodeDiagnostics {
    let (details, supported_formats, status) = match probe.details(&node) {
        Ok(raw) => {
            let details = bounded_details(raw);
            let (formats, status) = status_of(&details);
            (Some(details), formats, status)
        }
        Err(failure) => (None, Vec::new(), NodeStatus::ProbeFailed(failure)),
    };
    NodeDiagnostics {
        node,
        by_id_alias: None,
        details,
        supported_formats,
        status,
        sensor_type: None,
        classification_reason: None,
        by_id_hint_ignored: false,
    }
}

/// Returns the alias resolving to `node` (matched on the canonical or the raw node path).
fn alias_of(node: &Path, aliases: &[(PathBuf, PathBuf)]) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(node).unwrap_or_else(|_| node.to_path_buf());
    aliases
        .iter()
        .find(|(_, target)| *target == canonical || target == node)
        .map(|(alias, _)| alias.clone())
}

/// Candidate inventory handed to the shared resolver: exactly the nodes already probed.
struct ProbedInventory<'a> {
    nodes: &'a [NodeDiagnostics],
    aliases: &'a [(PathBuf, PathBuf)],
}

impl CameraEnumerator for ProbedInventory<'_> {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        self.nodes
            .iter()
            .filter(|n| n.status == NodeStatus::Candidate)
            .filter_map(|n| {
                let details = n.details.as_ref()?;
                Some(CameraDeviceInfo {
                    path: n.node.clone(),
                    card_name: details.card_name.clone(),
                    supported_formats: n.supported_formats.clone(),
                })
            })
            .collect()
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        self.aliases.to_vec()
    }

    fn frame_sizes(&self, device: &CameraDeviceInfo) -> Vec<(u32, u32)> {
        self.nodes
            .iter()
            .find(|n| n.node == device.path)
            .and_then(|n| n.details.as_ref())
            .map(|d| d.frame_sizes.clone())
            .unwrap_or_default()
    }
}

/// Lists every `video<N>` node of `sysfs_dir` (probing `dev_dir/<node>` once each) and reports
/// the shared resolver's decision for `preference` and the optional explicit device.
///
/// `aliases` are `(by-id link, canonical target)` pairs, normally
/// `SystemCameraEnumerator::by_id_aliases` (the single bounded by-id scanner). An explicit
/// non-sentinel `explicit` path wins exactly as in `resolve_camera_device`; the inventory is
/// still listed and classified so support can compare.
pub fn collect_camera_diagnostics(
    sysfs_dir: &Path,
    dev_dir: &Path,
    aliases: &[(PathBuf, PathBuf)],
    probe: &dyn V4lDeviceProbe,
    preference: SensorPreference,
    explicit: Option<&Path>,
) -> CameraDiagnostics {
    let mut nodes: Vec<NodeDiagnostics> = scan_video_node_names(sysfs_dir)
        .into_iter()
        .map(|name| probe_node(dev_dir.join(name), probe))
        .collect();
    for node in &mut nodes {
        node.by_id_alias = alias_of(&node.node, aliases);
    }

    let inventory = ProbedInventory {
        nodes: &nodes,
        aliases,
    };
    let auto = explain_camera_resolution(None, preference, &inventory);
    let decision = explain_camera_resolution(explicit, preference, &inventory);

    let selected_node = decision
        .selected
        .and_then(|i| decision.candidates.get(i))
        .map(|c| c.device.path.clone());
    for candidate in &auto.candidates {
        if let Some(node) = nodes.iter_mut().find(|n| n.node == candidate.device.path) {
            node.sensor_type = Some(candidate.sensor_type);
            node.classification_reason = Some(candidate.reason);
            node.by_id_hint_ignored = candidate.by_id_hint_ignored;
        }
    }

    CameraDiagnostics {
        preference,
        nodes,
        resolution: decision.resolution,
        selected_node,
        reason: decision.reason,
    }
}

/// Probes one device path (a `/dev/videoN` node or a by-id link) and classifies it in isolation.
///
/// A by-id link is probed through its canonical target and reported as the alias. The
/// shared-stem rule needs the sibling nodes, so it is applied only by
/// [`collect_camera_diagnostics`].
pub fn probe_camera_node(
    path: &Path,
    aliases: &[(PathBuf, PathBuf)],
    probe: &dyn V4lDeviceProbe,
) -> NodeDiagnostics {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let by_id_alias = aliases
        .iter()
        .find(|(alias, _)| alias == path)
        .map(|(alias, _)| alias.clone())
        .or_else(|| alias_of(&target, aliases));
    let mut node = probe_node(target, probe);
    node.by_id_alias = by_id_alias;
    if node.status == NodeStatus::Candidate {
        if let Some(details) = &node.details {
            let hints = SensorHints {
                by_id_name: node
                    .by_id_alias
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned()),
                frame_sizes: details.frame_sizes.clone(),
            };
            let (sensor, reason) =
                explain_sensor_classification(&details.card_name, &node.supported_formats, &hints);
            node.sensor_type = Some(sensor);
            node.classification_reason = Some(reason);
        }
    }
    node
}

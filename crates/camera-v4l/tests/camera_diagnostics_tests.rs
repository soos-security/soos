//! Hermetic contract for camera diagnostics and explained resolution
//! (GitHub #256 / CAM-17, #195 / CAM-13, #198 / CAM-16; matrix rows CDX1-CDX6).
//!
//! Every V4L2 query goes through the injectable [`V4lDeviceProbe`] trait and the sysfs listing
//! is a temporary directory, so node listing, probe failures, classification reasons and the
//! resolver decision are proven without any camera on the host.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use soos_camera_v4l::diagnostics::{
    collect_camera_diagnostics, device_caps_flag_names, fourcc_label, probe_camera_node,
    sanitize_v4l_text, CameraDiagnostics, NodeStatus, ProbeFailure, V4lDeviceProbe, V4lNodeDetails,
    MAX_DIAGNOSTIC_FOURCCS, MAX_V4L_TEXT_CHARS, V4L2_CAP_DEVICE_CAPS, V4L2_CAP_EXT_PIX_FORMAT,
    V4L2_CAP_META_CAPTURE, V4L2_CAP_STREAMING, V4L2_CAP_VIDEO_CAPTURE,
};
use soos_camera_v4l::{
    by_id_stem, classify_sensor_with_hints, explain_camera_resolution,
    explain_sensor_classification, resolve_camera_device, CameraDeviceInfo, CameraEnumerator,
    CameraResolutionSource, ClassificationReason, PixelFormat, SelectionReason, SensorHints,
    SensorPreference, SensorType, AUTO_CAMERA_DEVICE, MAX_FRAME_SIZE_HINTS, MAX_VIDEO_NODES,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEV: &str = "/dev/soos-hermetic";
const CAPTURE_CAPS: u32 =
    V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_EXT_PIX_FORMAT | V4L2_CAP_STREAMING | V4L2_CAP_DEVICE_CAPS;
const META_CAPS: u32 =
    V4L2_CAP_META_CAPTURE | V4L2_CAP_EXT_PIX_FORMAT | V4L2_CAP_STREAMING | V4L2_CAP_DEVICE_CAPS;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeProbe {
    table: BTreeMap<PathBuf, Result<V4lNodeDetails, ProbeFailure>>,
    probed: RefCell<Vec<PathBuf>>,
}

impl FakeProbe {
    fn node(
        mut self,
        name: &str,
        card: &str,
        caps: u32,
        fourccs: &[&[u8; 4]],
        frame_sizes: &[(u32, u32)],
    ) -> Self {
        self.table.insert(
            Path::new(DEV).join(name),
            Ok(details(card, caps, fourccs, frame_sizes)),
        );
        self
    }

    fn failing(mut self, name: &str, failure: ProbeFailure) -> Self {
        self.table.insert(Path::new(DEV).join(name), Err(failure));
        self
    }
}

impl V4lDeviceProbe for FakeProbe {
    fn details(&self, dev_path: &Path) -> Result<V4lNodeDetails, ProbeFailure> {
        self.probed.borrow_mut().push(dev_path.to_path_buf());
        self.table
            .get(dev_path)
            .cloned()
            .unwrap_or(Err(ProbeFailure::NotFound))
    }
}

fn details(
    card: &str,
    caps: u32,
    fourccs: &[&[u8; 4]],
    frame_sizes: &[(u32, u32)],
) -> V4lNodeDetails {
    V4lNodeDetails {
        driver: "uvcvideo".to_string(),
        card_name: card.to_string(),
        bus_info: "usb-0000:00:14.0-8".to_string(),
        device_caps: caps,
        fourccs: fourccs.iter().map(|f| **f).collect(),
        frame_sizes: frame_sizes.to_vec(),
    }
}

fn fake_sysfs(entries: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in entries {
        std::fs::create_dir(dir.path().join(name)).unwrap();
    }
    dir
}

fn alias(link: &str, node: &str) -> (PathBuf, PathBuf) {
    (PathBuf::from(link), Path::new(DEV).join(node))
}

fn node<'a>(
    diag: &'a CameraDiagnostics,
    name: &str,
) -> &'a soos_camera_v4l::diagnostics::NodeDiagnostics {
    let path = Path::new(DEV).join(name);
    diag.nodes
        .iter()
        .find(|n| n.node == path)
        .unwrap_or_else(|| panic!("node {name} missing from diagnostics"))
}

/// Typical Windows-Hello laptop plus the failure modes support sees in the field.
fn field_laptop() -> (tempfile::TempDir, FakeProbe, Vec<(PathBuf, PathBuf)>) {
    let sysfs = fake_sysfs(&[
        "video0",
        "video1",
        "video2",
        "video3",
        "video4",
        "video5",
        "v4l-subdev0",
    ]);
    let probe = FakeProbe::default()
        .node(
            "video0",
            "Integrated Camera: Integrated C",
            CAPTURE_CAPS,
            &[b"MJPG", b"YUYV"],
            &[(1280, 720), (640, 480)],
        )
        .node(
            "video1",
            "Integrated Camera: Integrated C",
            META_CAPS,
            &[b"UVCH"],
            &[],
        )
        .node(
            "video2",
            "Integrated Camera: Integrated I",
            CAPTURE_CAPS,
            &[b"Y16 "],
            &[(400, 400)],
        )
        .failing("video3", ProbeFailure::PermissionDenied)
        .failing("video4", ProbeFailure::Busy)
        .node(
            "video5",
            "Codec: Some Encoder",
            CAPTURE_CAPS,
            &[b"H264"],
            &[(1920, 1080)],
        );
    let aliases = vec![
        alias(
            "/dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index0",
            "video0",
        ),
        alias(
            "/dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index1",
            "video1",
        ),
        alias(
            "/dev/v4l/by-id/usb-Chicony_Integrated_IR_Camera-video-index0",
            "video2",
        ),
    ];
    (sysfs, probe, aliases)
}

struct FakeEnumerator {
    devices: Vec<CameraDeviceInfo>,
    aliases: Vec<(PathBuf, PathBuf)>,
    frame_sizes: BTreeMap<PathBuf, Vec<(u32, u32)>>,
}

impl CameraEnumerator for FakeEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        self.devices.clone()
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        self.aliases.clone()
    }

    fn frame_sizes(&self, device: &CameraDeviceInfo) -> Vec<(u32, u32)> {
        self.frame_sizes
            .get(&device.path)
            .cloned()
            .unwrap_or_default()
    }
}

fn device(path: &str, card: &str, formats: &[PixelFormat]) -> CameraDeviceInfo {
    CameraDeviceInfo {
        path: PathBuf::from(path),
        card_name: card.to_string(),
        supported_formats: formats.to_vec(),
    }
}

// ---------------------------------------------------------------------------
// CDX1: every classification is explained by the rule that decided it
// ---------------------------------------------------------------------------

#[test]
fn test_cdx_explain_reports_each_scorer_rule() {
    let hints_by_id = SensorHints {
        by_id_name: Some("usb-Acme_IR_Camera-video-index0".to_string()),
        frame_sizes: Vec::new(),
    };
    let hints_small = SensorHints {
        by_id_name: None,
        frame_sizes: vec![(640, 360)],
    };
    let none = SensorHints::default();
    let rows: [(
        &str,
        &[PixelFormat],
        &SensorHints,
        SensorType,
        ClassificationReason,
    ); 6] = [
        (
            "Integrated Camera: Integrated C",
            &[PixelFormat::Yuyv],
            &hints_by_id,
            SensorType::Infrared,
            ClassificationReason::ByIdIrToken,
        ),
        (
            "USB2.0 FHD UVC WebCam: USB2.0 I",
            &[PixelFormat::Yuyv],
            &none,
            SensorType::Infrared,
            ClassificationReason::CardNameIrMarker,
        ),
        (
            "Integrated Camera: Integrated C",
            &[PixelFormat::Grey],
            &none,
            SensorType::Infrared,
            ClassificationReason::GreyscaleOnlyFormats,
        ),
        (
            "Integrated Camera: Integrated C",
            &[PixelFormat::Yuyv],
            &hints_small,
            SensorType::Infrared,
            ClassificationReason::IrFrameSizeSignature,
        ),
        (
            "Integrated Camera: Integrated C",
            &[PixelFormat::Mjpeg, PixelFormat::Yuyv],
            &none,
            SensorType::Rgb,
            ClassificationReason::ColourFormats,
        ),
        (
            "Integrated Camera: Integrated C",
            &[],
            &none,
            SensorType::Unknown,
            ClassificationReason::NoSignal,
        ),
    ];
    for (card, formats, hints, sensor, reason) in rows {
        assert_eq!(
            explain_sensor_classification(card, formats, hints),
            (sensor, reason),
            "card {card:?}, formats {formats:?}, hints {hints:?}"
        );
        assert_eq!(
            classify_sensor_with_hints(card, formats, hints),
            sensor,
            "explain_sensor_classification must agree with classify_sensor_with_hints"
        );
    }
    let labels: Vec<&str> = [
        ClassificationReason::ByIdIrToken,
        ClassificationReason::CardNameIrMarker,
        ClassificationReason::GreyscaleOnlyFormats,
        ClassificationReason::IrFrameSizeSignature,
        ClassificationReason::ColourFormats,
        ClassificationReason::NoSignal,
    ]
    .iter()
    .map(|r| r.as_str())
    .collect();
    assert_eq!(
        labels,
        [
            "by_id_ir_token",
            "card_name_ir_marker",
            "greyscale_only_formats",
            "ir_frame_size_signature",
            "colour_formats",
            "no_signal",
        ]
    );
}

// ---------------------------------------------------------------------------
// CDX2: a by-id IR token shared by several capture nodes is not decisive
// ---------------------------------------------------------------------------

#[test]
fn test_cdx_by_id_stem_strips_video_index_suffix() {
    assert_eq!(
        by_id_stem("usb-Acme_HD_IR_Camera_0001-video-index0"),
        "usb-Acme_HD_IR_Camera_0001"
    );
    assert_eq!(
        by_id_stem("usb-Acme_HD_IR_Camera_0001-video-index12"),
        "usb-Acme_HD_IR_Camera_0001"
    );
    assert_eq!(by_id_stem("usb-Acme-video-index"), "usb-Acme-video-index");
    assert_eq!(by_id_stem("usb-Acme-video-indexX"), "usb-Acme-video-indexX");
    assert_eq!(by_id_stem("custom-cam"), "custom-cam");
}

/// Composite RGB+IR module whose USB product string carries `IR`: udev gives both interfaces
/// the same by-id stem, so the token says nothing about which node is the IR one. The resolver
/// must fall through to the next rules (candid review finding 3 of walkthrough 109).
fn composite_module() -> FakeEnumerator {
    FakeEnumerator {
        devices: vec![
            device(
                "/dev/video0",
                "HD IR Camera: HD IR Camera",
                &[PixelFormat::Mjpeg, PixelFormat::Yuyv],
            ),
            device(
                "/dev/video2",
                "HD IR Camera: HD IR Camera",
                &[PixelFormat::Grey],
            ),
        ],
        aliases: vec![
            (
                PathBuf::from("/dev/v4l/by-id/usb-Acme_HD_IR_Camera_0001-video-index0"),
                PathBuf::from("/dev/video0"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-Acme_HD_IR_Camera_0001-video-index2"),
                PathBuf::from("/dev/video2"),
            ),
        ],
        frame_sizes: BTreeMap::new(),
    }
}

#[test]
fn test_cdx_shared_by_id_stem_ir_token_is_not_decisive() {
    // The card names also carry "IR" on such modules; use neutral names to isolate the by-id rule.
    let mut enumerator = composite_module();
    for d in &mut enumerator.devices {
        d.card_name = "Acme HD Camera: Acme HD Camera".to_string();
    }

    let ir = resolve_camera_device(None, SensorPreference::PreferIr, &enumerator);
    assert_eq!(
        ir.path,
        PathBuf::from("/dev/v4l/by-id/usb-Acme_HD_IR_Camera_0001-video-index2"),
        "PreferIr must pick the greyscale node, not the first node sharing the IR by-id stem"
    );
    assert_eq!(ir.sensor_type, Some(SensorType::Infrared));

    let rgb = resolve_camera_device(None, SensorPreference::PreferRgb, &enumerator);
    assert_eq!(
        rgb.path,
        PathBuf::from("/dev/v4l/by-id/usb-Acme_HD_IR_Camera_0001-video-index0")
    );
    assert_eq!(rgb.sensor_type, Some(SensorType::Rgb));

    let report = explain_camera_resolution(None, SensorPreference::PreferIr, &enumerator);
    assert_eq!(report.candidates.len(), 2);
    for candidate in &report.candidates {
        assert!(
            candidate.by_id_hint_ignored,
            "{}: the shared by-id stem must be reported as ignored",
            candidate.device.path.display()
        );
        assert_eq!(candidate.hints.by_id_name, None);
    }
    assert_eq!(
        report.candidates[0].reason,
        ClassificationReason::ColourFormats
    );
    assert_eq!(
        report.candidates[1].reason,
        ClassificationReason::GreyscaleOnlyFormats
    );
}

#[test]
fn test_cdx_distinct_by_id_stems_keep_the_ir_token() {
    let enumerator = FakeEnumerator {
        devices: vec![
            device(
                "/dev/video0",
                "Integrated Camera: Integrated C",
                &[PixelFormat::Yuyv],
            ),
            device(
                "/dev/video2",
                "Integrated Camera: Integrated C",
                &[PixelFormat::Yuyv],
            ),
        ],
        aliases: vec![
            (
                PathBuf::from("/dev/v4l/by-id/usb-Vendor_Integrated_Camera-video-index0"),
                PathBuf::from("/dev/video0"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-Vendor_Integrated_IR_Camera-video-index0"),
                PathBuf::from("/dev/video2"),
            ),
        ],
        frame_sizes: BTreeMap::new(),
    };
    let report = explain_camera_resolution(None, SensorPreference::PreferIr, &enumerator);
    assert!(report.candidates.iter().all(|c| !c.by_id_hint_ignored));
    assert_eq!(
        report.candidates[1].reason,
        ClassificationReason::ByIdIrToken
    );
    assert_eq!(
        report.resolution.path,
        PathBuf::from("/dev/v4l/by-id/usb-Vendor_Integrated_IR_Camera-video-index0")
    );
}

// ---------------------------------------------------------------------------
// CDX3: the resolver decision carries its reason and equals resolve_camera_device
// ---------------------------------------------------------------------------

#[test]
fn test_cdx_explain_resolution_reports_selection_reasons() {
    let rgb = device("/dev/video0", "RGB Cam: RGB Cam", &[PixelFormat::Yuyv]);
    let ir = device("/dev/video2", "Cam: IR", &[PixelFormat::Grey]);
    let unknown = device("/dev/video4", "Mystery", &[]);
    let inventory = |devices: Vec<CameraDeviceInfo>| FakeEnumerator {
        devices,
        aliases: Vec::new(),
        frame_sizes: BTreeMap::new(),
    };

    /// (case name, inventory, explicit device, preference, expected reason, expected path)
    type Case<'a> = (
        &'a str,
        FakeEnumerator,
        Option<&'a Path>,
        SensorPreference,
        SelectionReason,
        &'a str,
    );
    let cases: Vec<Case<'_>> = vec![
        (
            "IR present",
            inventory(vec![rgb.clone(), ir.clone()]),
            None,
            SensorPreference::PreferIr,
            SelectionReason::PreferredSensorMatched,
            "/dev/video2",
        ),
        (
            "IR missing, unknown present",
            inventory(vec![rgb.clone(), unknown.clone()]),
            None,
            SensorPreference::PreferIr,
            SelectionReason::FallbackUnknownSensor,
            "/dev/video4",
        ),
        (
            "IR missing, RGB only",
            inventory(vec![rgb.clone()]),
            None,
            SensorPreference::PreferIr,
            SelectionReason::FallbackFirstCandidate,
            "/dev/video0",
        ),
        (
            "any preference",
            inventory(vec![rgb.clone(), ir.clone()]),
            None,
            SensorPreference::Any,
            SelectionReason::AnyPreferenceFirstCandidate,
            "/dev/video0",
        ),
        (
            "no capture node",
            inventory(Vec::new()),
            None,
            SensorPreference::PreferIr,
            SelectionReason::NoCaptureNode,
            AUTO_CAMERA_DEVICE,
        ),
        (
            "explicit device",
            inventory(vec![rgb.clone(), ir.clone()]),
            Some(Path::new("/dev/video7")),
            SensorPreference::PreferIr,
            SelectionReason::ExplicitDevice,
            "/dev/video7",
        ),
    ];

    for (name, enumerator, explicit, preference, reason, path) in cases {
        let report = explain_camera_resolution(explicit, preference, &enumerator);
        assert_eq!(report.reason, reason, "{name}");
        assert_eq!(report.resolution.path, PathBuf::from(path), "{name}");
        assert_eq!(
            report.resolution,
            resolve_camera_device(explicit, preference, &enumerator),
            "{name}: explain_camera_resolution must equal resolve_camera_device"
        );
    }

    let labels: Vec<&str> = [
        SelectionReason::ExplicitDevice,
        SelectionReason::PreferredSensorMatched,
        SelectionReason::FallbackUnknownSensor,
        SelectionReason::FallbackFirstCandidate,
        SelectionReason::AnyPreferenceFirstCandidate,
        SelectionReason::NoCaptureNode,
    ]
    .iter()
    .map(|r| r.as_str())
    .collect();
    assert_eq!(
        labels,
        [
            "explicit_device",
            "preferred_sensor_matched",
            "fallback_unknown_sensor",
            "fallback_first_candidate",
            "any_preference_first_candidate",
            "no_capture_node",
        ]
    );
}

// ---------------------------------------------------------------------------
// CDX4: diagnostics list every node with its status, metadata and classification
// ---------------------------------------------------------------------------

#[test]
fn test_cdx_collect_lists_every_node_with_status() {
    let (sysfs, probe, aliases) = field_laptop();
    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &aliases,
        &probe,
        SensorPreference::PreferIr,
        None,
    );

    let listed: Vec<String> = diag
        .nodes
        .iter()
        .map(|n| n.node.display().to_string())
        .collect();
    assert_eq!(
        listed,
        (0..6)
            .map(|i| format!("{DEV}/video{i}"))
            .collect::<Vec<_>>(),
        "every video<N> node is listed in numeric order, non-video entries are ignored"
    );

    let rgb = node(&diag, "video0");
    assert_eq!(rgb.status, NodeStatus::Candidate);
    assert_eq!(rgb.sensor_type, Some(SensorType::Rgb));
    assert_eq!(
        rgb.classification_reason,
        Some(ClassificationReason::ColourFormats)
    );
    assert_eq!(
        rgb.by_id_alias,
        Some(PathBuf::from(
            "/dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index0"
        ))
    );
    let rgb_details = rgb.details.as_ref().unwrap();
    assert_eq!(rgb_details.driver, "uvcvideo");
    assert_eq!(rgb_details.bus_info, "usb-0000:00:14.0-8");
    assert_eq!(rgb_details.frame_sizes, vec![(1280, 720), (640, 480)]);
    assert_eq!(
        rgb.supported_formats,
        vec![PixelFormat::Mjpeg, PixelFormat::Yuyv]
    );

    let meta = node(&diag, "video1");
    assert_eq!(meta.status, NodeStatus::NotVideoCapture);
    assert_eq!(meta.sensor_type, None);
    assert!(
        meta.details.is_some(),
        "a metadata node still shows its QUERYCAP data"
    );

    let ir = node(&diag, "video2");
    assert_eq!(ir.status, NodeStatus::Candidate);
    assert_eq!(ir.supported_formats, vec![PixelFormat::Grey]);
    assert_eq!(ir.sensor_type, Some(SensorType::Infrared));
    assert_eq!(
        ir.classification_reason,
        Some(ClassificationReason::ByIdIrToken)
    );

    assert_eq!(
        node(&diag, "video3").status,
        NodeStatus::ProbeFailed(ProbeFailure::PermissionDenied)
    );
    assert_eq!(
        node(&diag, "video4").status,
        NodeStatus::ProbeFailed(ProbeFailure::Busy)
    );
    assert!(node(&diag, "video4").details.is_none());
    assert_eq!(node(&diag, "video5").status, NodeStatus::NoDecodableFormat);

    assert_eq!(diag.preference, SensorPreference::PreferIr);
    assert_eq!(diag.reason, SelectionReason::PreferredSensorMatched);
    assert_eq!(diag.resolution.source, CameraResolutionSource::AutoDetected);
    assert_eq!(
        diag.resolution.path,
        PathBuf::from("/dev/v4l/by-id/usb-Chicony_Integrated_IR_Camera-video-index0")
    );
    assert_eq!(diag.selected_node, Some(Path::new(DEV).join("video2")));
    assert_eq!(diag.resolution.sensor_type, Some(SensorType::Infrared));

    let labels: Vec<&str> = diag.nodes.iter().map(|n| n.status.as_str()).collect();
    assert_eq!(
        labels,
        [
            "candidate",
            "not_video_capture",
            "candidate",
            "probe_failed",
            "probe_failed",
            "no_decodable_format",
        ]
    );
    assert_eq!(ProbeFailure::PermissionDenied.as_str(), "permission_denied");
    assert_eq!(ProbeFailure::Busy.as_str(), "busy");
    assert_eq!(ProbeFailure::NotFound.as_str(), "not_found");
    assert_eq!(ProbeFailure::Other(Some(5)).as_str(), "io_error");
}

/// The diagnostics decision is the shared resolver's decision over the same inventory.
#[test]
fn test_cdx_collect_matches_shared_resolver() {
    let (sysfs, probe, aliases) = field_laptop();
    for preference in [
        SensorPreference::PreferIr,
        SensorPreference::PreferRgb,
        SensorPreference::Any,
    ] {
        let diag = collect_camera_diagnostics(
            sysfs.path(),
            Path::new(DEV),
            &aliases,
            &probe,
            preference,
            None,
        );
        let mut frame_sizes = BTreeMap::new();
        let devices: Vec<CameraDeviceInfo> = diag
            .nodes
            .iter()
            .filter(|n| n.status == NodeStatus::Candidate)
            .map(|n| {
                let d = n.details.as_ref().unwrap();
                frame_sizes.insert(n.node.clone(), d.frame_sizes.clone());
                CameraDeviceInfo {
                    path: n.node.clone(),
                    card_name: d.card_name.clone(),
                    supported_formats: n.supported_formats.clone(),
                }
            })
            .collect();
        let enumerator = FakeEnumerator {
            devices,
            aliases: aliases.clone(),
            frame_sizes,
        };
        assert_eq!(
            diag.resolution,
            resolve_camera_device(None, preference, &enumerator),
            "{preference:?}"
        );
    }
}

#[test]
fn test_cdx_collect_explicit_device_overrides_selection() {
    let (sysfs, probe, aliases) = field_laptop();
    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &aliases,
        &probe,
        SensorPreference::PreferIr,
        Some(Path::new("/dev/video9")),
    );
    assert_eq!(diag.reason, SelectionReason::ExplicitDevice);
    assert_eq!(diag.resolution.source, CameraResolutionSource::Explicit);
    assert_eq!(diag.resolution.path, PathBuf::from("/dev/video9"));
    assert_eq!(diag.selected_node, None);
    assert_eq!(diag.nodes.len(), 6, "the inventory is still listed");

    // A sentinel is not explicit: auto-detection runs.
    let auto = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &aliases,
        &probe,
        SensorPreference::PreferIr,
        Some(Path::new("auto")),
    );
    assert_eq!(auto.reason, SelectionReason::PreferredSensorMatched);
}

#[test]
fn test_cdx_collect_without_capture_node_reports_fallback() {
    let sysfs = fake_sysfs(&["video0"]);
    let probe = FakeProbe::default().failing("video0", ProbeFailure::PermissionDenied);
    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &[],
        &probe,
        SensorPreference::PreferIr,
        None,
    );
    assert_eq!(diag.reason, SelectionReason::NoCaptureNode);
    assert_eq!(diag.resolution.source, CameraResolutionSource::Fallback);
    assert_eq!(diag.resolution.path, PathBuf::from(AUTO_CAMERA_DEVICE));
    assert_eq!(diag.selected_node, None);

    let missing = collect_camera_diagnostics(
        &sysfs.path().join("does-not-exist"),
        Path::new(DEV),
        &[],
        &probe,
        SensorPreference::PreferIr,
        None,
    );
    assert!(missing.nodes.is_empty());
    assert_eq!(missing.reason, SelectionReason::NoCaptureNode);
}

#[test]
fn test_cdx_collect_frame_size_signature_on_identical_names() {
    let sysfs = fake_sysfs(&["video0", "video2"]);
    let probe = FakeProbe::default()
        .node(
            "video0",
            "Integrated Camera: Integrated C",
            CAPTURE_CAPS,
            &[b"MJPG", b"YUYV"],
            &[(1280, 720), (640, 480)],
        )
        .node(
            "video2",
            "Integrated Camera: Integrated C",
            CAPTURE_CAPS,
            &[b"YUYV"],
            &[(640, 360)],
        );
    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &[],
        &probe,
        SensorPreference::PreferIr,
        None,
    );
    let ir = node(&diag, "video2");
    assert_eq!(
        ir.classification_reason,
        Some(ClassificationReason::IrFrameSizeSignature)
    );
    assert_eq!(diag.resolution.path, Path::new(DEV).join("video2"));
}

// ---------------------------------------------------------------------------
// CDX5: bounded, hermetic probing and safe text
// ---------------------------------------------------------------------------

#[test]
fn test_cdx_collect_bounds_probed_nodes_and_lists() {
    let names: Vec<String> = (0..MAX_VIDEO_NODES + 10)
        .map(|i| format!("video{i}"))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let sysfs = fake_sysfs(&refs);

    let many_fourccs: Vec<[u8; 4]> = (0..200_u32)
        .map(|i| {
            let b = i.to_le_bytes();
            [b'Z', b[0], b[1], b'Z']
        })
        .collect();
    let mut fourccs = vec![*b"YUYV"];
    fourccs.extend(many_fourccs);
    let frame_sizes: Vec<(u32, u32)> = (0..200).map(|i| (1000 + i, 1000 + i)).collect();
    let mut probe = FakeProbe::default();
    probe.table.insert(
        Path::new(DEV).join("video0"),
        Ok(V4lNodeDetails {
            driver: "uvcvideo".to_string(),
            card_name: "Big".to_string(),
            bus_info: "usb-1".to_string(),
            device_caps: CAPTURE_CAPS,
            fourccs,
            frame_sizes,
        }),
    );

    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &[],
        &probe,
        SensorPreference::PreferIr,
        None,
    );
    assert!(probe.probed.borrow().len() <= MAX_VIDEO_NODES);
    assert!(diag.nodes.len() <= MAX_VIDEO_NODES);
    let big = node(&diag, "video0").details.as_ref().unwrap();
    assert!(big.fourccs.len() <= MAX_DIAGNOSTIC_FOURCCS);
    assert!(big.frame_sizes.len() <= MAX_FRAME_SIZE_HINTS);
    assert_eq!(big.fourccs[0], *b"YUYV");
}

#[test]
fn test_cdx_probe_failure_from_io_error() {
    let cases = [
        (libc::EACCES, ProbeFailure::PermissionDenied),
        (libc::EPERM, ProbeFailure::PermissionDenied),
        (libc::EBUSY, ProbeFailure::Busy),
        (libc::ENOENT, ProbeFailure::NotFound),
        (libc::ENODEV, ProbeFailure::NotFound),
        (libc::ENXIO, ProbeFailure::NotFound),
        (libc::EIO, ProbeFailure::Other(Some(libc::EIO))),
    ];
    for (errno, expected) in cases {
        assert_eq!(
            ProbeFailure::from_io_error(&std::io::Error::from_raw_os_error(errno)),
            expected,
            "errno {errno}"
        );
    }
    assert_eq!(
        ProbeFailure::from_io_error(&std::io::Error::other("no errno")),
        ProbeFailure::Other(None)
    );
}

#[test]
fn test_cdx_probe_node_reports_details_and_classification() {
    let probe = FakeProbe::default().node(
        "video2",
        "Integrated Camera: Integrated C",
        CAPTURE_CAPS,
        &[b"GREY"],
        &[(340, 340)],
    );
    let aliases = vec![alias(
        "/dev/v4l/by-id/usb-Vendor_Cam-video-index0",
        "video2",
    )];
    let report = probe_camera_node(&Path::new(DEV).join("video2"), &aliases, &probe);
    assert_eq!(report.status, NodeStatus::Candidate);
    assert_eq!(
        report.by_id_alias,
        Some(PathBuf::from("/dev/v4l/by-id/usb-Vendor_Cam-video-index0"))
    );
    assert_eq!(report.sensor_type, Some(SensorType::Infrared));
    assert_eq!(
        report.classification_reason,
        Some(ClassificationReason::GreyscaleOnlyFormats)
    );

    let failed = probe_camera_node(&Path::new(DEV).join("video9"), &aliases, &probe);
    assert_eq!(
        failed.status,
        NodeStatus::ProbeFailed(ProbeFailure::NotFound)
    );
    assert_eq!(failed.sensor_type, None);
    assert!(failed.details.is_none());
}

#[test]
fn test_cdx_text_helpers_are_safe_for_terminals() {
    assert_eq!(fourcc_label(*b"YUYV"), "YUYV");
    assert_eq!(fourcc_label(*b"Y16 "), "Y16");
    assert_eq!(fourcc_label(*b"Y8  "), "Y8");
    assert_eq!(fourcc_label([0x1b, b'[', 0x07, 0xff]), ".[..");

    let hostile = "Cam\u{1b}[31mRED\u{7}\nnext\u{202e}rtl";
    let cleaned = sanitize_v4l_text(hostile);
    assert!(!cleaned.chars().any(char::is_control), "{cleaned:?}");
    assert!(!cleaned.contains('\u{202e}'), "bidi overrides are replaced");
    assert!(cleaned.starts_with("Cam?[31mRED?"));
    let long = "x".repeat(500);
    assert_eq!(sanitize_v4l_text(&long).chars().count(), MAX_V4L_TEXT_CHARS);

    assert_eq!(
        device_caps_flag_names(CAPTURE_CAPS),
        [
            "VIDEO_CAPTURE",
            "EXT_PIX_FORMAT",
            "STREAMING",
            "DEVICE_CAPS"
        ]
    );
    assert_eq!(
        device_caps_flag_names(META_CAPS),
        ["EXT_PIX_FORMAT", "META_CAPTURE", "STREAMING", "DEVICE_CAPS"]
    );
    assert!(device_caps_flag_names(0).is_empty());
}

/// Hostile names reported by a device are sanitized in the collected diagnostics.
#[test]
fn test_cdx_collect_sanitizes_device_strings() {
    let sysfs = fake_sysfs(&["video0"]);
    let mut probe = FakeProbe::default();
    probe.table.insert(
        Path::new(DEV).join("video0"),
        Ok(V4lNodeDetails {
            driver: "uvc\u{1b}video".to_string(),
            card_name: "Cam\u{1b}]0;pwned\u{7}".to_string(),
            bus_info: "usb\n1".to_string(),
            device_caps: CAPTURE_CAPS,
            fourccs: vec![*b"YUYV"],
            frame_sizes: vec![(640, 480)],
        }),
    );
    let diag = collect_camera_diagnostics(
        sysfs.path(),
        Path::new(DEV),
        &[],
        &probe,
        SensorPreference::PreferIr,
        None,
    );
    let d = node(&diag, "video0").details.as_ref().unwrap();
    for text in [&d.driver, &d.card_name, &d.bus_info] {
        assert!(!text.chars().any(char::is_control), "{text:?}");
    }
}

// ---------------------------------------------------------------------------
// Real probe: only V4L2 character devices are opened (candid review, CDX11)
// ---------------------------------------------------------------------------

/// `soos-admin camera probe <DEVICE>` often runs as root: the real probe must refuse any path
/// that is not a V4L2 character device (major 81) before `open(2)`, so probing a tape drive,
/// tty or regular file has no side effect. A refused path reports `ENOTTY`.
#[test]
fn test_cdx_system_probe_refuses_non_v4l2_nodes_before_open() {
    use soos_camera_v4l::diagnostics::SystemV4lDeviceProbe;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let regular = dir.path().join("not-a-camera");
    std::fs::write(&regular, b"x").unwrap();
    // Unreadable: an open(2) attempt would fail with EACCES (non-root), the guard with ENOTTY.
    std::fs::set_permissions(&regular, std::fs::Permissions::from_mode(0o000)).unwrap();

    let probe = SystemV4lDeviceProbe;
    assert_eq!(
        probe.details(&regular).unwrap_err(),
        ProbeFailure::Other(Some(libc::ENOTTY)),
        "a regular file must be refused before open"
    );
    assert_eq!(
        probe.details(Path::new("/dev/null")).unwrap_err(),
        ProbeFailure::Other(Some(libc::ENOTTY)),
        "a character device with a non-V4L2 major must be refused"
    );
    assert_eq!(
        probe.details(&dir.path().join("missing")).unwrap_err(),
        ProbeFailure::NotFound,
        "a missing path is still reported as not found"
    );
}

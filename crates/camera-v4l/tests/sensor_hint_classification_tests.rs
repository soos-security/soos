//! Contract tests for review finding CAM-13 (GitHub #195): IR/RGB classification must not rely
//! only on the 31-byte truncated V4L2 card name and on "any colour fourcc means RGB", and IR
//! nodes that only expose deep-greyscale fourccs (Y8I, Y10, Y12, Y16) must stay visible.
//!
//! Ordered scorer (strongest signal first):
//! 1. an IR token in the persistent `/dev/v4l/by-id/` link name,
//! 2. an IR marker in the card name,
//! 3. a greyscale-only format list,
//! 4. an IR frame-size signature (every size at most 640x400, e.g. 340x340, 400x400, 640x360),
//! 5. otherwise any colour format means RGB, nothing means Unknown.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_camera_v4l::{
    capture_device_from_probe, classify_sensor, classify_sensor_with_hints, delivered_formats,
    is_ir_frame_size_signature, plan_capture_with_hints, resolve_camera_device, select_wire_format,
    CameraConfigBuilder, CameraDeviceInfo, CameraEnumerator, CameraResolutionSource,
    CaptureWireFormat, DeepGreyFormat, PixelFormat, SensorHints, SensorPreference, SensorType,
};
use std::path::PathBuf;
use v4l::FourCC;

fn hints(by_id: Option<&str>, sizes: &[(u32, u32)]) -> SensorHints {
    SensorHints {
        by_id_name: by_id.map(str::to_string),
        frame_sizes: sizes.to_vec(),
    }
}

// ---------------------------------------------------------------------------
// Ordered scorer
// ---------------------------------------------------------------------------

/// Real-world card names (as truncated by V4L2) and by-id names from common Windows-Hello
/// laptops. Each row: (label, card name, by-id name, formats, frame sizes, expected).
#[test]
fn test_ccp_classify_real_world_table() {
    let yuyv_mjpg = [PixelFormat::Yuyv, PixelFormat::Mjpeg];
    let grey = [PixelFormat::Grey];
    let yuyv = [PixelFormat::Yuyv];
    let hd = [(1280, 720), (640, 480)];
    #[allow(
        clippy::type_complexity,
        reason = "Table-driven test rows are read once and kept inline for clarity"
    )]
    let rows: &[(
        &str,
        &str,
        Option<&str>,
        &[PixelFormat],
        &[(u32, u32)],
        SensorType,
    )] = &[
        (
            "Asus G14 IR (truncated ' I')",
            "USB2.0 FHD UVC WebCam: USB2.0 I",
            Some("usb-SunplusIT_Inc_USB2.0_FHD_UVC_WebCam-video-index2"),
            &grey,
            &[(640, 360)],
            SensorType::Infrared,
        ),
        (
            "Asus G14 RGB",
            "USB2.0 FHD UVC WebCam: USB2.0 F",
            Some("usb-SunplusIT_Inc_USB2.0_FHD_UVC_WebCam-video-index0"),
            &yuyv_mjpg,
            &[(1920, 1080), (1280, 720)],
            SensorType::Rgb,
        ),
        (
            "ThinkPad IR, identical truncated card name, YUYV only, IR by-id link",
            "Integrated Camera: Integrated C",
            Some("usb-Chicony_Electronics_Co._Ltd._Integrated_IR_Camera_0001-video-index0"),
            &yuyv,
            &[(640, 480)],
            SensorType::Infrared,
        ),
        (
            "ThinkPad RGB, identical truncated card name",
            "Integrated Camera: Integrated C",
            Some("usb-Chicony_Electronics_Co._Ltd._Integrated_Camera_0001-video-index0"),
            &yuyv_mjpg,
            &hd,
            SensorType::Rgb,
        ),
        (
            "Dell IR, identical card name, YUYV with an IR frame-size signature",
            "Integrated_Webcam_HD: Integrate",
            Some("usb-CN0FFMHCLOG0081AA1B0A00_Integrated_Webcam_HD_200901010001-video-index2"),
            &yuyv,
            &[(640, 360), (340, 340)],
            SensorType::Infrared,
        ),
        (
            "Dell RGB, identical card name",
            "Integrated_Webcam_HD: Integrate",
            Some("usb-CN0FFMHCLOG0081AA1B0A00_Integrated_Webcam_HD_200901010001-video-index0"),
            &yuyv_mjpg,
            &hd,
            SensorType::Rgb,
        ),
        (
            "Surface front IR",
            "Microsoft IR Camera Front",
            None,
            &yuyv,
            &[],
            SensorType::Infrared,
        ),
        (
            "Surface front RGB",
            "Microsoft Camera Front",
            None,
            &yuyv_mjpg,
            &[],
            SensorType::Rgb,
        ),
        (
            "Underscore-separated IR token in the card name",
            "Integrated_IR_Camera: Integrat",
            None,
            &yuyv,
            &[],
            SensorType::Infrared,
        ),
        (
            "VGA-only RGB webcam is not mistaken for IR",
            "Generic USB Webcam",
            None,
            &yuyv,
            &[(640, 480), (320, 240)],
            SensorType::Rgb,
        ),
        (
            "No signal at all",
            "Unknown Device",
            None,
            &[],
            &[],
            SensorType::Unknown,
        ),
    ];

    for (label, card, by_id, formats, sizes, expected) in rows {
        assert_eq!(
            classify_sensor_with_hints(card, formats, &hints(*by_id, sizes)),
            *expected,
            "row '{label}' misclassified"
        );
    }
}

#[test]
fn test_ccp_by_id_ir_token_beats_colour_formats() {
    let h = hints(Some("usb-Vendor_Integrated_IR_Camera-video-index0"), &[]);
    assert_eq!(
        classify_sensor_with_hints("Integrated Camera: Integrated C", &[PixelFormat::Yuyv], &h),
        SensorType::Infrared
    );
}

#[test]
fn test_ccp_ir_token_requires_whole_token() {
    // "Chicony", "Firmware", "Circle" contain the letters "ir" but no IR token.
    let h = hints(Some("usb-Chicony_Firmware_Circle_Camera-video-index0"), &[]);
    assert_eq!(
        classify_sensor_with_hints("Circle Camera", &[PixelFormat::Yuyv], &h),
        SensorType::Rgb
    );
}

#[test]
fn test_ccp_frame_size_signature_bounds() {
    assert!(is_ir_frame_size_signature(&[(640, 360)]));
    assert!(is_ir_frame_size_signature(&[(340, 340), (400, 400)]));
    assert!(is_ir_frame_size_signature(&[(640, 400)]));
    assert!(!is_ir_frame_size_signature(&[]));
    assert!(!is_ir_frame_size_signature(&[(640, 480)]));
    assert!(!is_ir_frame_size_signature(&[(640, 360), (1280, 720)]));
}

#[test]
fn test_ccp_empty_hints_match_legacy_classifier() {
    let cases: &[(&str, &[PixelFormat])] = &[
        ("Integrated Camera: IR", &[PixelFormat::Grey]),
        ("SunplusIT Inc: IR Camera", &[PixelFormat::Yuyv]),
        ("Camera Sensor 0", &[PixelFormat::Grey]),
        ("Camera Sensor 1", &[PixelFormat::Yuyv]),
        ("USB2.0 FHD UVC WebCam: USB2.0 I", &[]),
        ("Unknown", &[]),
    ];
    for (card, formats) in cases {
        assert_eq!(
            classify_sensor_with_hints(card, formats, &SensorHints::default()),
            classify_sensor(card, formats),
            "empty hints must reproduce classify_sensor for '{card}'"
        );
    }
}

#[test]
fn test_ccp_device_info_sensor_type_with_hints() {
    let device = CameraDeviceInfo {
        path: PathBuf::from("/dev/video2"),
        card_name: "Integrated Camera: Integrated C".to_string(),
        supported_formats: vec![PixelFormat::Yuyv],
    };
    assert_eq!(device.sensor_type(), SensorType::Rgb);
    assert_eq!(
        device.sensor_type_with_hints(&hints(None, &[(640, 360)])),
        SensorType::Infrared
    );
}

// ---------------------------------------------------------------------------
// Resolver uses the hints (by-id names and frame sizes from the enumerator)
// ---------------------------------------------------------------------------

/// Host-like laptop: both nodes report the identical truncated card name and YUYV.
struct IdenticalNamesEnumerator {
    with_frame_sizes: bool,
}

impl CameraEnumerator for IdenticalNamesEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        vec![
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video0"),
                card_name: "Integrated Camera: Integrated C".to_string(),
                supported_formats: vec![PixelFormat::Mjpeg, PixelFormat::Yuyv],
            },
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video2"),
                card_name: "Integrated Camera: Integrated C".to_string(),
                supported_formats: vec![PixelFormat::Yuyv],
            },
        ]
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        vec![
            (
                PathBuf::from("/dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index0"),
                PathBuf::from("/dev/video0"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index2"),
                PathBuf::from("/dev/video2"),
            ),
        ]
    }

    fn frame_sizes(&self, device: &CameraDeviceInfo) -> Vec<(u32, u32)> {
        if !self.with_frame_sizes {
            return Vec::new();
        }
        if device.path == std::path::Path::new("/dev/video2") {
            vec![(640, 360)]
        } else {
            vec![(1280, 720), (640, 480)]
        }
    }
}

#[test]
fn test_ccp_resolver_prefer_ir_uses_frame_size_hint_on_identical_names() {
    let resolution = resolve_camera_device(
        None,
        SensorPreference::PreferIr,
        &IdenticalNamesEnumerator {
            with_frame_sizes: true,
        },
    );
    assert_eq!(resolution.source, CameraResolutionSource::AutoDetected);
    assert_eq!(
        resolution.path,
        PathBuf::from("/dev/v4l/by-id/usb-Chicony_Integrated_Camera-video-index2"),
        "PreferIr must pick the node with the IR frame-size signature"
    );
    assert_eq!(resolution.sensor_type, Some(SensorType::Infrared));
}

#[test]
fn test_ccp_resolver_without_hints_keeps_legacy_first_rgb_fallback() {
    let resolution = resolve_camera_device(
        None,
        SensorPreference::PreferIr,
        &IdenticalNamesEnumerator {
            with_frame_sizes: false,
        },
    );
    assert_eq!(resolution.sensor_type, Some(SensorType::Rgb));
}

/// IR node identified only by its by-id link name.
struct ByIdNamedEnumerator;

impl CameraEnumerator for ByIdNamedEnumerator {
    fn capture_devices(&self) -> Vec<CameraDeviceInfo> {
        vec![
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video0"),
                card_name: "Integrated Camera: Integrated C".to_string(),
                supported_formats: vec![PixelFormat::Yuyv],
            },
            CameraDeviceInfo {
                path: PathBuf::from("/dev/video2"),
                card_name: "Integrated Camera: Integrated C".to_string(),
                supported_formats: vec![PixelFormat::Yuyv],
            },
        ]
    }

    fn by_id_aliases(&self) -> Vec<(PathBuf, PathBuf)> {
        vec![
            (
                PathBuf::from("/dev/v4l/by-id/usb-Vendor_Integrated_Camera-video-index0"),
                PathBuf::from("/dev/video0"),
            ),
            (
                PathBuf::from("/dev/v4l/by-id/usb-Vendor_Integrated_IR_Camera-video-index0"),
                PathBuf::from("/dev/video2"),
            ),
        ]
    }
}

#[test]
fn test_ccp_resolver_prefer_ir_uses_by_id_ir_token() {
    let resolution = resolve_camera_device(None, SensorPreference::PreferIr, &ByIdNamedEnumerator);
    assert_eq!(
        resolution.path,
        PathBuf::from("/dev/v4l/by-id/usb-Vendor_Integrated_IR_Camera-video-index0")
    );
    assert_eq!(resolution.sensor_type, Some(SensorType::Infrared));

    let rgb = resolve_camera_device(None, SensorPreference::PreferRgb, &ByIdNamedEnumerator);
    assert_eq!(
        rgb.path,
        PathBuf::from("/dev/v4l/by-id/usb-Vendor_Integrated_Camera-video-index0")
    );
    assert_eq!(rgb.sensor_type, Some(SensorType::Rgb));
}

// ---------------------------------------------------------------------------
// Deep-greyscale fourccs keep IR nodes visible
// ---------------------------------------------------------------------------

#[test]
fn test_enumerate_keeps_y16_only_ir_node() {
    let device = capture_device_from_probe(
        PathBuf::from("/dev/video2"),
        "Integrated Camera: Integrated C".to_string(),
        true,
        &[FourCC::new(b"Y16 ")],
    )
    .expect("A Y16-only IR node must not be dropped from enumeration");
    assert_eq!(device.supported_formats, vec![PixelFormat::Grey]);
    assert_eq!(device.sensor_type(), SensorType::Infrared);
}

#[test]
fn test_ccp_probe_keeps_every_deep_grey_fourcc() {
    for code in [b"Y8I ", b"Y10 ", b"Y12 ", b"Y16 "] {
        let device = capture_device_from_probe(
            PathBuf::from("/dev/video4"),
            "Camera".to_string(),
            true,
            &[FourCC::new(code)],
        );
        assert!(
            device.is_some(),
            "fourcc {:?} must keep the node visible",
            std::str::from_utf8(code)
        );
    }
}

#[test]
fn test_ccp_probe_drops_non_capture_and_unknown_nodes() {
    assert!(capture_device_from_probe(
        PathBuf::from("/dev/video1"),
        "Integrated Camera: Integrated C".to_string(),
        false,
        &[FourCC::new(b"YUYV")],
    )
    .is_none());
    assert!(capture_device_from_probe(
        PathBuf::from("/dev/video3"),
        "Metadata".to_string(),
        true,
        &[FourCC::new(b"UVCH")],
    )
    .is_none());
}

#[test]
fn test_ccp_delivered_formats_add_grey_once_for_deep_grey() {
    let formats = delivered_formats(&[
        FourCC::new(b"YUYV"),
        FourCC::new(b"Y16 "),
        FourCC::new(b"Y8I "),
    ]);
    assert_eq!(formats, vec![PixelFormat::Yuyv, PixelFormat::Grey]);

    let native = delivered_formats(&[FourCC::new(b"GREY"), FourCC::new(b"Y16 ")]);
    assert_eq!(native, vec![PixelFormat::Grey]);
}

#[test]
fn test_ccp_wire_format_prefers_native_grey_then_deep_grey() {
    assert_eq!(
        select_wire_format(
            PixelFormat::Grey,
            &[FourCC::new(b"GREY"), FourCC::new(b"Y16 ")]
        ),
        CaptureWireFormat::Native(PixelFormat::Grey)
    );
    assert_eq!(
        select_wire_format(PixelFormat::Grey, &[FourCC::new(b"Y10 ")]),
        CaptureWireFormat::DeepGrey(DeepGreyFormat::Y10)
    );
    assert_eq!(
        select_wire_format(
            PixelFormat::Grey,
            &[FourCC::new(b"Y8I "), FourCC::new(b"Y16 ")]
        ),
        CaptureWireFormat::DeepGrey(DeepGreyFormat::Y16)
    );
    assert_eq!(
        select_wire_format(PixelFormat::Yuyv, &[FourCC::new(b"YUYV")]),
        CaptureWireFormat::Native(PixelFormat::Yuyv)
    );
    assert_eq!(
        CaptureWireFormat::DeepGrey(DeepGreyFormat::Y12).fourcc(),
        FourCC::new(b"Y12 ")
    );
}

#[test]
fn test_ccp_deep_grey_conversion_to_grey8() {
    // 2x1 frame; little-endian 16-bit containers.
    let y16 = [0x34, 0x12, 0xFF, 0xFF];
    assert_eq!(
        DeepGreyFormat::Y16.to_grey8(&y16, 2, 1, 4),
        Some(vec![0x12, 0xFF])
    );
    let y10 = [0xFF, 0x03, 0x00, 0x02]; // 1023, 512
    assert_eq!(
        DeepGreyFormat::Y10.to_grey8(&y10, 2, 1, 4),
        Some(vec![255, 128])
    );
    let y12 = [0xFF, 0x0F, 0x00, 0x08]; // 4095, 2048
    assert_eq!(
        DeepGreyFormat::Y12.to_grey8(&y12, 2, 1, 4),
        Some(vec![255, 128])
    );
    // Y8I: interleaved left/right; the left sensor is kept.
    let y8i = [10, 200, 20, 210];
    assert_eq!(
        DeepGreyFormat::Y8i.to_grey8(&y8i, 2, 1, 4),
        Some(vec![10, 20])
    );
}

#[test]
fn test_ccp_deep_grey_conversion_honours_stride_and_rejects_short_buffers() {
    // 1x2 frame with a padded 4-byte stride (2 bytes of payload + 2 bytes of padding per row).
    let padded = [0x00, 0x10, 0xEE, 0xEE, 0x00, 0x20, 0xEE, 0xEE];
    assert_eq!(
        DeepGreyFormat::Y16.to_grey8(&padded, 1, 2, 4),
        Some(vec![0x10, 0x20])
    );
    // A stride smaller than the row size is replaced by the packed row size.
    assert_eq!(
        DeepGreyFormat::Y16.to_grey8(&[0, 1, 0, 2], 2, 1, 0),
        Some(vec![1, 2])
    );
    assert_eq!(DeepGreyFormat::Y16.to_grey8(&[0, 1, 0], 2, 1, 4), None);
    assert_eq!(DeepGreyFormat::Y16.to_grey8(&[], 0, 0, 0), None);
}

#[test]
fn test_ccp_plan_capture_with_hints_tags_yuyv_ir_node() {
    let config = CameraConfigBuilder::new().build();
    let plan = plan_capture_with_hints(
        "Integrated Camera: Integrated C",
        &[PixelFormat::Yuyv],
        &hints(Some("usb-Vendor_Integrated_IR_Camera-video-index0"), &[]),
        &config,
    )
    .unwrap();
    assert_eq!(plan.sensor_type, SensorType::Infrared);
    assert_eq!(plan.format, PixelFormat::Yuyv);

    let grey = plan_capture_with_hints(
        "Integrated Camera: Integrated C",
        &[PixelFormat::Yuyv, PixelFormat::Grey],
        &hints(None, &[(640, 360)]),
        &config,
    )
    .unwrap();
    assert_eq!(grey.sensor_type, SensorType::Infrared);
    assert_eq!(grey.format, PixelFormat::Grey);
}

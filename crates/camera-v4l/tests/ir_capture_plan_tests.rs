//! Contract tests: the capture supervisor classifies the opened node and stamps its frames with
//! the sensor type, preferring `Grey` on infrared nodes (candid review finding 2 on #169).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_camera_v4l::{plan_capture, CameraConfigBuilder, PixelFormat, SensorType};

/// Card name of the IR node of a common dual-sensor webcam (V4L2 truncates it to 31 bytes).
const IR_CARD: &str = "USB2.0 FHD UVC WebCam: USB2.0 I";
const RGB_CARD: &str = "USB2.0 FHD UVC WebCam: USB2.0 F";

#[test]
fn test_ir_node_advertising_yuyv_and_grey_is_planned_as_grey_infrared() {
    let config = CameraConfigBuilder::new().build();
    let plan = plan_capture(IR_CARD, &[PixelFormat::Yuyv, PixelFormat::Grey], &config).unwrap();
    assert_eq!(plan.sensor_type, SensorType::Infrared);
    assert_eq!(
        plan.format,
        PixelFormat::Grey,
        "auto negotiation must prefer the native Grey format on an IR node"
    );
}

#[test]
fn test_ir_node_without_grey_keeps_infrared_classification() {
    let config = CameraConfigBuilder::new().build();
    let plan = plan_capture(IR_CARD, &[PixelFormat::Yuyv, PixelFormat::Mjpeg], &config).unwrap();
    assert_eq!(plan.format, PixelFormat::Yuyv);
    assert_eq!(
        plan.sensor_type,
        SensorType::Infrared,
        "an IR node streaming YUYV must still be tagged Infrared (fail-closed PAD policy)"
    );
}

#[test]
fn test_explicit_format_on_ir_node_is_honored_and_still_tagged_infrared() {
    let config = CameraConfigBuilder::new()
        .format(PixelFormat::Yuyv)
        .auto_format(false)
        .build();
    let plan = plan_capture(IR_CARD, &[PixelFormat::Yuyv, PixelFormat::Grey], &config).unwrap();
    assert_eq!(plan.format, PixelFormat::Yuyv);
    assert_eq!(plan.sensor_type, SensorType::Infrared);
}

#[test]
fn test_rgb_node_keeps_colour_priority() {
    let config = CameraConfigBuilder::new().build();
    let plan = plan_capture(
        RGB_CARD,
        &[PixelFormat::Mjpeg, PixelFormat::Yuyv, PixelFormat::Grey],
        &config,
    )
    .unwrap();
    assert_eq!(plan.sensor_type, SensorType::Rgb);
    assert_eq!(plan.format, PixelFormat::Yuyv);
}

#[test]
fn test_grey_only_node_is_infrared() {
    let config = CameraConfigBuilder::new().build();
    let plan = plan_capture("Integrated Camera", &[PixelFormat::Grey], &config).unwrap();
    assert_eq!(plan.sensor_type, SensorType::Infrared);
    assert_eq!(plan.format, PixelFormat::Grey);
}

#[test]
fn test_node_without_enumerated_formats_uses_configured_format() {
    let config = CameraConfigBuilder::new()
        .format(PixelFormat::Mjpeg)
        .build();
    let plan = plan_capture("Unknown Device", &[], &config).unwrap();
    assert_eq!(plan.format, PixelFormat::Mjpeg);
    assert_eq!(plan.sensor_type, SensorType::Unknown);
}

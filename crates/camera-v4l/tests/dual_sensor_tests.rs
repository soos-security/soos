//! Tests for Sub-issue #22.4: IR camera filtering for dual-sensor devices.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_camera_v4l::{
    classify_sensor, select_camera_device, CameraDeviceInfo, PixelFormat, SensorPreference,
    SensorType,
};
use std::path::PathBuf;

#[test]
fn test_sensor_classification_by_card_name() {
    assert_eq!(
        classify_sensor("Integrated Camera: IR", &[PixelFormat::Grey]),
        SensorType::Infrared
    );
    assert_eq!(
        classify_sensor("SunplusIT Inc: IR Camera", &[PixelFormat::Yuyv]),
        SensorType::Infrared
    );
    assert_eq!(
        classify_sensor("ThinkPad Infrared Sensor", &[]),
        SensorType::Infrared
    );
    assert_eq!(
        classify_sensor("Integrated Camera: RGB", &[PixelFormat::Yuyv]),
        SensorType::Rgb
    );
    assert_eq!(
        classify_sensor(
            "Logitech Webcam C920",
            &[PixelFormat::Rgb24, PixelFormat::Mjpeg]
        ),
        SensorType::Rgb
    );
}

#[test]
fn test_sensor_classification_by_formats() {
    // Sensor without IR in name, but only supporting Grey format is classified as Infrared
    assert_eq!(
        classify_sensor("Camera Sensor 0", &[PixelFormat::Grey]),
        SensorType::Infrared
    );

    // Sensor supporting color format is classified as RGB
    assert_eq!(
        classify_sensor("Camera Sensor 1", &[PixelFormat::Yuyv]),
        SensorType::Rgb
    );
    assert_eq!(
        classify_sensor("Camera Sensor 2", &[PixelFormat::Nv12]),
        SensorType::Rgb
    );
}

#[test]
fn test_dual_sensor_prefers_rgb() {
    let ir_device = CameraDeviceInfo {
        path: PathBuf::from(
            "/dev/v4l/by-id/usb-Chicony_Electronics_Co._Ltd._Integrated_IR_Camera-video-index0",
        ),
        card_name: "Integrated Camera: Infrared".to_string(),
        supported_formats: vec![PixelFormat::Grey],
    };

    let rgb_device = CameraDeviceInfo {
        path: PathBuf::from(
            "/dev/v4l/by-id/usb-Chicony_Electronics_Co._Ltd._Integrated_Camera-video-index0",
        ),
        card_name: "Integrated Camera: RGB".to_string(),
        supported_formats: vec![PixelFormat::Yuyv, PixelFormat::Mjpeg],
    };

    // Candidate list where IR camera appears first in enumeration
    let candidates = vec![ir_device.clone(), rgb_device.clone()];

    // Default preference must prefer RGB sensor
    let selected = select_camera_device(&candidates, SensorPreference::PreferRgb)
        .expect("Must select a camera device");

    assert_eq!(
        selected.path, rgb_device.path,
        "Automatic dual-sensor selection must choose RGB camera over IR camera"
    );
    assert_eq!(selected.sensor_type(), SensorType::Rgb);
}

#[test]
fn test_dual_sensor_override_prefers_ir() {
    let ir_device = CameraDeviceInfo {
        path: PathBuf::from(
            "/dev/v4l/by-id/usb-Chicony_Electronics_Co._Ltd._Integrated_IR_Camera-video-index0",
        ),
        card_name: "Integrated Camera: Infrared".to_string(),
        supported_formats: vec![PixelFormat::Grey],
    };

    let rgb_device = CameraDeviceInfo {
        path: PathBuf::from(
            "/dev/v4l/by-id/usb-Chicony_Electronics_Co._Ltd._Integrated_Camera-video-index0",
        ),
        card_name: "Integrated Camera: RGB".to_string(),
        supported_formats: vec![PixelFormat::Yuyv, PixelFormat::Mjpeg],
    };

    let candidates = vec![rgb_device.clone(), ir_device.clone()];

    // Config override explicitly requesting IR sensor
    let selected = select_camera_device(&candidates, SensorPreference::PreferIr)
        .expect("Must select a camera device");

    assert_eq!(
        selected.path, ir_device.path,
        "Configuration override must select IR camera when PreferIr is specified"
    );
    assert_eq!(selected.sensor_type(), SensorType::Infrared);
}

#[test]
fn test_default_sensor_preference_is_prefer_ir() {
    use soos_camera_v4l::CameraConfig;

    assert_eq!(
        SensorPreference::default(),
        SensorPreference::PreferIr,
        "Default SensorPreference must be PreferIr to prioritize anti-spoofing IR hardware"
    );

    assert_eq!(
        CameraConfig::default().sensor_preference,
        SensorPreference::PreferIr,
        "CameraConfig default sensor preference must be PreferIr"
    );

    let ir_device = CameraDeviceInfo {
        path: PathBuf::from("/dev/video2"),
        card_name: "USB2.0 FHD UVC WebCam: USB2.0 I".to_string(),
        supported_formats: vec![PixelFormat::Grey],
    };

    let rgb_device = CameraDeviceInfo {
        path: PathBuf::from("/dev/video0"),
        card_name: "USB2.0 FHD UVC WebCam: USB2.0 F".to_string(),
        supported_formats: vec![PixelFormat::Yuyv, PixelFormat::Mjpeg],
    };

    let candidates = vec![rgb_device.clone(), ir_device.clone()];
    let selected =
        select_camera_device(&candidates, SensorPreference::default()).expect("Must select device");
    assert_eq!(
        selected.path, ir_device.path,
        "Default selection must prefer IR camera when present"
    );
    assert_eq!(selected.sensor_type(), SensorType::Infrared);
}

#[test]
fn test_sensor_classification_truncated_ir_card_name() {
    // 31-character truncated V4L2 card name ending with " I" from "IR"
    assert_eq!(
        classify_sensor("USB2.0 FHD UVC WebCam: USB2.0 I", &[]),
        SensorType::Infrared,
        "Card name ending with ' I' truncated from ' IR' must be classified as Infrared"
    );
    assert_eq!(
        classify_sensor("USB2.0 FHD UVC WebCam: USB2.0 I", &[PixelFormat::Grey]),
        SensorType::Infrared
    );
}

#[test]
fn test_select_camera_device_ignores_empty_format_nodes() {
    let metadata_node = CameraDeviceInfo {
        path: PathBuf::from("/dev/video1"),
        card_name: "USB2.0 FHD UVC WebCam: USB2.0 F".to_string(),
        supported_formats: vec![],
    };

    let ir_device = CameraDeviceInfo {
        path: PathBuf::from("/dev/video2"),
        card_name: "USB2.0 FHD UVC WebCam: USB2.0 I".to_string(),
        supported_formats: vec![PixelFormat::Grey],
    };

    let candidates = vec![metadata_node.clone(), ir_device.clone()];
    let selected = select_camera_device(&candidates, SensorPreference::PreferIr)
        .expect("Must select valid device");
    assert_eq!(selected.path, ir_device.path);
}

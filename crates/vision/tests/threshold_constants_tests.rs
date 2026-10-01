//! Single-source detection and PAD threshold contract (GitHub #251 VIS-09, #215 PAD-10).
//!
//! Enforces (verification matrix rows VTD4, VTD5):
//! - Every detection/PAD threshold used by `soos-daemon`, `soos-enroll` and `soos-gui` is a
//!   named constant of `soos-vision`, carried by `VisionPipelineConfig`.
//! - The single liveness decision (`VisionPipelineConfig::pad_passes`) is the one applied by
//!   the pipeline and shown by the GUI: live, finite, and `score >= effective threshold`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    reason = "Contract test suite uses direct assertions"
)]

use soos_inference_ort::{AttackType, PadResult};
use soos_vision::{
    PadInputModality, VisionPipelineConfig, DEFAULT_IR_PAD_THRESHOLD, DEFAULT_MATCH_THRESHOLD,
    DEFAULT_MIN_FACE_CONFIDENCE, DEFAULT_NMS_IOU_THRESHOLD, DEFAULT_PAD_THRESHOLD,
};

#[test]
fn test_vision_threshold_constants_values() {
    // Daemon (authentication) values; the match default is 0.50 since the SFace switch
    // (owner decision 2026-10-01, GitHub #278).
    assert_eq!(DEFAULT_MIN_FACE_CONFIDENCE, 0.70);
    assert_eq!(DEFAULT_MATCH_THRESHOLD, 0.50);
    assert_eq!(DEFAULT_PAD_THRESHOLD, 0.85);
    assert_eq!(DEFAULT_NMS_IOU_THRESHOLD, 0.45);
}

#[test]
fn test_vision_default_config_uses_named_constants() {
    let cfg = VisionPipelineConfig::default();
    assert_eq!(cfg.min_face_confidence, DEFAULT_MIN_FACE_CONFIDENCE);
    assert_eq!(cfg.match_threshold, DEFAULT_MATCH_THRESHOLD);
    assert_eq!(cfg.pad_threshold, DEFAULT_PAD_THRESHOLD);
    assert_eq!(cfg.nms_iou_threshold, DEFAULT_NMS_IOU_THRESHOLD);
    assert_eq!(cfg.ir_pad_threshold, DEFAULT_IR_PAD_THRESHOLD);
}

#[test]
fn test_pad_passes_single_threshold_colour() {
    let cfg = VisionPipelineConfig::default();
    let color = PadInputModality::Color;
    // The former GUI display threshold (0.80) must no longer show LIVE.
    assert!(!cfg.pad_passes(&PadResult::live(0.82), color));
    assert!(!cfg.pad_passes(&PadResult::live(0.849), color));
    assert!(cfg.pad_passes(&PadResult::live(0.85), color));
    assert!(cfg.pad_passes(&PadResult::live(0.99), color));
}

#[test]
fn test_pad_passes_fail_closed() {
    let cfg = VisionPipelineConfig::default();
    let color = PadInputModality::Color;
    assert!(!cfg.pad_passes(&PadResult::spoof(0.99, AttackType::PrintPhoto), color));
    assert!(!cfg.pad_passes(&PadResult::live(f32::NAN), color));
    assert!(!cfg.pad_passes(&PadResult::live(f32::INFINITY), color));
    let nan_cfg = VisionPipelineConfig {
        pad_threshold: f32::NAN,
        ..Default::default()
    };
    assert!(!nan_cfg.pad_passes(&PadResult::live(0.99), color));
}

#[test]
fn test_pad_passes_monochrome_uses_effective_ir_threshold() {
    let cfg = VisionPipelineConfig::default();
    let mono = PadInputModality::Monochrome;
    let ir = cfg.effective_pad_threshold(mono);
    assert!(ir >= cfg.pad_threshold);
    assert!(!cfg.pad_passes(&PadResult::live(0.90), mono) || ir <= 0.90);
    assert!(cfg.pad_passes(&PadResult::live(ir), mono));
}

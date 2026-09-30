//! Vision and policy default thresholds must be one value (GitHub #215 PAD-10, #251 VIS-09;
//! verification matrix row VTD9).
//!
//! The daemon applies the PAD threshold twice by design (per frame in `VisionPipeline`, per
//! request in the `soos-policy` PAD consensus); both defaults must agree, and the daemon's
//! validated operator thresholds must be copied into the vision configuration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    reason = "Contract test suite uses direct assertions"
)]

use soos_policy::{
    FrameClass, FrameEvaluation, PadAggregator, PadConsensusConfig, ThresholdConfig,
};

#[test]
fn test_vision_and_policy_default_thresholds_agree() {
    assert_eq!(
        soos_vision::DEFAULT_PAD_THRESHOLD,
        ThresholdConfig::DEFAULT_PAD_THRESHOLD
    );
    assert_eq!(
        soos_vision::DEFAULT_MATCH_THRESHOLD,
        ThresholdConfig::DEFAULT_MATCH_THRESHOLD
    );
    let daemon = soos_daemon::config::DaemonConfig::default();
    assert_eq!(
        daemon.pipeline.vision.pad_threshold,
        daemon.pipeline.thresholds.pad_threshold()
    );
    assert_eq!(
        daemon.pipeline.vision.nms_iou_threshold,
        soos_vision::DEFAULT_NMS_IOU_THRESHOLD
    );
}

#[test]
fn test_policy_pad_threshold_is_enforced_by_consensus() {
    let aggregator = PadAggregator::new(PadConsensusConfig::default(), ThresholdConfig::default());
    // A score between the former GUI/enrollment threshold (0.80) and 0.85 is a spoof.
    let below = FrameEvaluation::new(1, true, 0.82, 0.99);
    assert_eq!(aggregator.classify(&below), FrameClass::Spoof);
    let at = FrameEvaluation::new(1, true, 0.85, 0.99);
    assert_eq!(aggregator.classify(&at), FrameClass::Passing);
}

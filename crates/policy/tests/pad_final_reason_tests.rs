//! Final-reason priority of the PAD consensus (review finding PAD-16 — GitHub #262).
//!
//! A spoof detected earlier in a request must remain the reported reason even when the
//! attacker withdraws the device and every later capture has no face, several faces or
//! a non-matching face: audit logs and PAM feedback must keep the spoof signal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contract test suite uses direct assertions"
)]

use proptest::prelude::*;
use soos_policy::{ConsensusDecision, FrameEvaluation, PadAggregator, ThresholdConfig};
use soos_protocol::{ReasonClass, Verdict};

fn aggregator() -> PadAggregator {
    PadAggregator::with_defaults(ThresholdConfig::default())
}

#[test]
fn test_262_spoof_then_no_face_frames_keeps_pad_failed_reason() {
    let mut agg = aggregator();
    agg.record(&FrameEvaluation::spoof(0.05));
    for _ in 0..16 {
        agg.record(&FrameEvaluation::no_face());
    }
    assert_eq!(agg.decision(), ConsensusDecision::SpoofVetoed);
    assert_eq!(
        agg.decision().verdict(),
        (Verdict::Deny, ReasonClass::PadFailed)
    );
}

#[test]
fn test_262_no_face_without_spoof_still_reports_no_face() {
    let mut agg = aggregator();
    agg.record(&FrameEvaluation::live(0.97, 0.90));
    agg.record(&FrameEvaluation::no_face());
    assert_eq!(
        agg.decision().verdict(),
        (Verdict::Deny, ReasonClass::NoFace)
    );
}

fn non_spoof_frame() -> impl Strategy<Value = FrameEvaluation> {
    prop_oneof![
        Just(FrameEvaluation::no_face()),
        (2u8..=8).prop_map(FrameEvaluation::multiple_faces),
        Just(FrameEvaluation::live(0.97, 0.10)),
        Just(FrameEvaluation::live(0.97, 0.90)),
    ]
}

proptest! {
    /// Whatever follows a spoof (no face, several faces, non-matching or even passing
    /// captures), the final reason is `PadFailed` and the verdict never authorizes.
    #[test]
    fn prop_262_spoof_reason_outranks_every_later_capture(
        before in proptest::collection::vec(non_spoof_frame(), 0..4),
        after in proptest::collection::vec(non_spoof_frame(), 0..40),
    ) {
        let mut agg = aggregator();
        for frame in &before {
            agg.record(frame);
        }
        agg.record(&FrameEvaluation::spoof(0.05));
        for frame in &after {
            agg.record(frame);
        }
        prop_assert_eq!(agg.decision().verdict(), (Verdict::Deny, ReasonClass::PadFailed));
    }
}

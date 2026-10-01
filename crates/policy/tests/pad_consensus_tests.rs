//! Contractual tests for multi-frame PAD consensus aggregation (GitHub #147 / PAD-02).
//!
//! The dispatcher used to authorize on the first frame that passed PAD and matching,
//! giving an attacker one independent liveness trial per evaluated frame. These tests
//! encode the replacement contract: `Allow` requires `k` consecutive passing frames
//! inside a bounded window of `n` frames, and any spoof-classified frame in the request
//! vetoes `Allow` for the whole request (fail closed).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses direct assertions, unwrap, indexing and proptest"
)]

use proptest::prelude::*;
use soos_policy::{
    ConsensusDecision, FrameClass, FrameEvaluation, PadAggregator, PadConsensusConfig, PolicyError,
    ThresholdConfig, DEFAULT_PAD_CONSENSUS_REQUIRED, DEFAULT_PAD_CONSENSUS_WINDOW,
    MAX_PAD_CONSENSUS_WINDOW,
};
use soos_protocol::{ReasonClass, Verdict};

/// A frame that is live above the default PAD threshold and matches above the default
/// cosine threshold (`0.85` PAD, `0.50` match).
fn passing() -> FrameEvaluation {
    FrameEvaluation::live(0.97, 0.90)
}

/// A frame classified as a presentation attack by the PAD model.
fn spoof() -> FrameEvaluation {
    FrameEvaluation::spoof(0.05)
}

fn aggregator() -> PadAggregator {
    PadAggregator::with_defaults(ThresholdConfig::default())
}

// ---------------------------------------------------------------------------
// Configuration constants and bounds
// ---------------------------------------------------------------------------

#[test]
fn test_pad_consensus_config_defaults_k3_within_n5() {
    assert_eq!(DEFAULT_PAD_CONSENSUS_REQUIRED, 3);
    assert_eq!(DEFAULT_PAD_CONSENSUS_WINDOW, 5);
    const { assert!(MAX_PAD_CONSENSUS_WINDOW >= DEFAULT_PAD_CONSENSUS_WINDOW) };

    let cfg = PadConsensusConfig::default();
    assert_eq!(cfg.required(), DEFAULT_PAD_CONSENSUS_REQUIRED);
    assert_eq!(cfg.window(), DEFAULT_PAD_CONSENSUS_WINDOW);
}

#[test]
fn test_pad_consensus_config_rejects_invalid_bounds() {
    assert!(matches!(
        PadConsensusConfig::new(5, 0),
        Err(PolicyError::InvalidConsensus { .. })
    ));
    assert!(matches!(
        PadConsensusConfig::new(5, 6),
        Err(PolicyError::InvalidConsensus { .. })
    ));
    assert!(matches!(
        PadConsensusConfig::new(0, 0),
        Err(PolicyError::InvalidConsensus { .. })
    ));
    assert!(matches!(
        PadConsensusConfig::new(MAX_PAD_CONSENSUS_WINDOW + 1, 1),
        Err(PolicyError::InvalidConsensus { .. })
    ));

    let ok = PadConsensusConfig::new(5, 5).expect("k == n is valid");
    assert_eq!(ok.required(), 5);
    let max = PadConsensusConfig::new(MAX_PAD_CONSENSUS_WINDOW, 1).expect("n == MAX is valid");
    assert_eq!(max.window(), MAX_PAD_CONSENSUS_WINDOW);
}

// ---------------------------------------------------------------------------
// Nominal consensus
// ---------------------------------------------------------------------------

#[test]
fn test_pad_aggregator_three_consecutive_live_frames_allow() {
    let mut agg = aggregator();

    assert_eq!(agg.record(&passing()), FrameClass::Passing);
    assert_eq!(
        agg.decision(),
        ConsensusDecision::Pending(Some(FrameClass::Passing))
    );
    assert_eq!(agg.record(&passing()), FrameClass::Passing);
    assert_eq!(
        agg.decision(),
        ConsensusDecision::Pending(Some(FrameClass::Passing))
    );
    assert_eq!(agg.record(&passing()), FrameClass::Passing);

    assert_eq!(agg.decision(), ConsensusDecision::Allow);
    assert_eq!(
        agg.decision().verdict(),
        (Verdict::Allow, ReasonClass::FaceMatch)
    );
    assert_eq!(agg.consecutive_passing(), 3);
    assert_eq!(agg.frames_evaluated(), 3);
    assert_eq!(agg.live_frames(), 3);
    assert_eq!(agg.spoof_frames(), 0);
}

#[test]
fn test_pad_aggregator_fewer_than_k_live_frames_is_pending_timeout() {
    let mut agg = aggregator();
    agg.record(&passing());
    agg.record(&passing());

    let decision = agg.decision();
    assert_ne!(decision, ConsensusDecision::Allow);
    let (verdict, reason) = decision.verdict();
    assert!(
        verdict.should_ignore(),
        "k-1 passing frames must not authorize"
    );
    assert_eq!(verdict, Verdict::Unavailable);
    assert_eq!(reason, ReasonClass::Timeout);
}

#[test]
fn test_pad_aggregator_no_frames_is_unavailable_timeout() {
    let agg = aggregator();
    assert_eq!(agg.decision(), ConsensusDecision::Pending(None));
    assert_eq!(
        agg.decision().verdict(),
        (Verdict::Unavailable, ReasonClass::Timeout)
    );
    assert_eq!(agg.frames_evaluated(), 0);
}

#[test]
fn test_pad_aggregator_first_passing_frame_does_not_allow() {
    // Regression guard for the "first passing frame wins" defect (PAD-02).
    let mut agg = aggregator();
    agg.record(&passing());
    assert_ne!(agg.decision(), ConsensusDecision::Allow);
    assert!(agg.decision().verdict().0.should_ignore());
}

// ---------------------------------------------------------------------------
// Spoof veto (fail closed)
// ---------------------------------------------------------------------------

#[test]
fn test_pad_aggregator_alternating_spoof_live_never_allows() {
    let mut agg = aggregator();
    for i in 0..20 {
        let frame = if i % 2 == 0 { passing() } else { spoof() };
        agg.record(&frame);
        assert_ne!(
            agg.decision(),
            ConsensusDecision::Allow,
            "alternating spoof/live must never reach Allow (frame {i})"
        );
    }
    assert_eq!(agg.decision(), ConsensusDecision::SpoofVetoed);
    assert_eq!(
        agg.decision().verdict(),
        (Verdict::Deny, ReasonClass::PadFailed)
    );
    assert_eq!(agg.spoof_frames(), 10);
    assert_eq!(agg.live_frames(), 10);
}

#[test]
fn test_pad_aggregator_spoof_veto_is_sticky_for_the_request() {
    let mut agg = aggregator();
    assert_eq!(agg.record(&spoof()), FrameClass::Spoof);
    assert!(agg.spoof_seen());

    // Far more than k consecutive passing frames afterwards: still vetoed.
    for _ in 0..(DEFAULT_PAD_CONSENSUS_WINDOW * 4) {
        agg.record(&passing());
        assert_eq!(agg.decision(), ConsensusDecision::SpoofVetoed);
    }
    assert_eq!(
        agg.decision().verdict(),
        (Verdict::Deny, ReasonClass::PadFailed)
    );
}

#[test]
fn test_pad_aggregator_spoof_after_consensus_reached_revokes_allow() {
    // A caller that keeps recording after Allow must observe the veto.
    let mut agg = aggregator();
    for _ in 0..DEFAULT_PAD_CONSENSUS_REQUIRED {
        agg.record(&passing());
    }
    assert_eq!(agg.decision(), ConsensusDecision::Allow);
    agg.record(&spoof());
    assert_eq!(agg.decision(), ConsensusDecision::SpoofVetoed);
}

#[test]
fn test_pad_aggregator_pad_score_below_threshold_is_spoof_even_if_classified_live() {
    let thresholds = ThresholdConfig::default(); // pad: 0.85
    let mut agg = PadAggregator::with_defaults(thresholds);
    let weak_live = FrameEvaluation::live(0.84, 0.95);
    assert_eq!(agg.record(&weak_live), FrameClass::Spoof);
    assert_eq!(agg.decision(), ConsensusDecision::SpoofVetoed);

    // Exactly at the threshold is accepted.
    let mut agg2 = PadAggregator::with_defaults(thresholds);
    assert_eq!(
        agg2.record(&FrameEvaluation::live(thresholds.pad_threshold(), 0.95)),
        FrameClass::Passing
    );
}

#[test]
fn test_pad_aggregator_non_finite_scores_fail_closed() {
    let mut agg = aggregator();
    assert_eq!(
        agg.record(&FrameEvaluation::live(f32::NAN, 0.95)),
        FrameClass::Spoof
    );
    let mut agg = aggregator();
    assert_eq!(
        agg.record(&FrameEvaluation::live(f32::INFINITY, 0.95)),
        FrameClass::Spoof
    );
    let mut agg = aggregator();
    assert_eq!(
        agg.record(&FrameEvaluation::live(0.99, f32::NAN)),
        FrameClass::NoMatch
    );
    let mut agg = aggregator();
    assert_eq!(
        agg.record(&FrameEvaluation::live(0.99, f32::INFINITY)),
        FrameClass::NoMatch
    );
    assert_ne!(agg.decision(), ConsensusDecision::Allow);
}

// ---------------------------------------------------------------------------
// Run resets and window bounds
// ---------------------------------------------------------------------------

#[test]
fn test_pad_aggregator_no_face_frame_resets_consecutive_run() {
    let mut agg = aggregator();
    agg.record(&passing());
    agg.record(&passing());
    assert_eq!(agg.record(&FrameEvaluation::no_face()), FrameClass::NoFace);
    assert_eq!(agg.consecutive_passing(), 0);
    assert_eq!(
        agg.decision().verdict(),
        (Verdict::Deny, ReasonClass::NoFace)
    );

    agg.record(&passing());
    agg.record(&passing());
    assert_ne!(agg.decision(), ConsensusDecision::Allow);
    agg.record(&passing());
    assert_eq!(agg.decision(), ConsensusDecision::Allow);
}

#[test]
fn test_pad_aggregator_multiple_faces_frame_resets_consecutive_run() {
    let mut agg = aggregator();
    agg.record(&passing());
    agg.record(&passing());
    assert_eq!(
        agg.record(&FrameEvaluation::multiple_faces(2)),
        FrameClass::MultipleFaces
    );
    assert_eq!(agg.consecutive_passing(), 0);
    assert_eq!(
        agg.decision().verdict(),
        (Verdict::Deny, ReasonClass::MultipleFaces)
    );
}

#[test]
fn test_pad_aggregator_low_match_score_breaks_run_without_veto() {
    let mut agg = aggregator();
    agg.record(&passing());
    agg.record(&passing());
    let live_but_stranger = FrameEvaluation::live(0.97, 0.40);
    assert_eq!(agg.record(&live_but_stranger), FrameClass::NoMatch);
    assert!(
        !agg.spoof_seen(),
        "a non-matching live frame is not a spoof"
    );
    assert_eq!(
        agg.decision().verdict(),
        (Verdict::Deny, ReasonClass::ScoreBelowThreshold)
    );

    for _ in 0..DEFAULT_PAD_CONSENSUS_REQUIRED {
        agg.record(&passing());
    }
    assert_eq!(agg.decision(), ConsensusDecision::Allow);
}

#[test]
fn test_pad_aggregator_history_is_bounded_by_window() {
    let cfg = PadConsensusConfig::new(4, 2).expect("valid config");
    let mut agg = PadAggregator::new(cfg, ThresholdConfig::default());
    for _ in 0..50 {
        agg.record(&FrameEvaluation::no_face());
    }
    assert_eq!(agg.history().count(), 4);
    assert_eq!(agg.frames_evaluated(), 50);
}

#[test]
fn test_pad_aggregator_custom_required_count_is_honored() {
    let cfg = PadConsensusConfig::new(8, 5).expect("valid config");
    let mut agg = PadAggregator::new(cfg, ThresholdConfig::default());
    for i in 1..=4 {
        agg.record(&passing());
        assert_ne!(agg.decision(), ConsensusDecision::Allow, "frame {i}");
    }
    agg.record(&passing());
    assert_eq!(agg.decision(), ConsensusDecision::Allow);
}

// ---------------------------------------------------------------------------
// Property-based invariants
// ---------------------------------------------------------------------------

fn arb_non_spoof_frame() -> impl Strategy<Value = FrameEvaluation> {
    prop_oneof![
        Just(FrameEvaluation::no_face()),
        (2u8..=8).prop_map(FrameEvaluation::multiple_faces),
        // Live above the PAD threshold, arbitrary finite match score.
        (0.85_f32..=1.0_f32, -1.0_f32..=1.0_f32).prop_map(|(pad, m)| FrameEvaluation::live(pad, m)),
    ]
}

fn arb_spoof_frame() -> impl Strategy<Value = FrameEvaluation> {
    prop_oneof![
        (0.0_f32..0.85_f32).prop_map(FrameEvaluation::spoof),
        // Classified live by the model but below the PAD threshold.
        (0.0_f32..0.85_f32, 0.7_f32..=1.0_f32).prop_map(|(pad, m)| FrameEvaluation::live(pad, m)),
        Just(FrameEvaluation::live(f32::NAN, 0.99)),
        (0.0_f32..=1.0_f32).prop_map(|m| FrameEvaluation::new(1, false, 0.99, m)),
    ]
}

proptest! {
    /// Any request containing at least one spoof-classified frame never authorizes,
    /// regardless of how many passing frames surround it and of the moment it is observed.
    #[test]
    fn prop_any_spoof_frame_prevents_allow(
        prefix in prop::collection::vec(arb_non_spoof_frame(), 0..12),
        spoof in arb_spoof_frame(),
        suffix in prop::collection::vec(arb_non_spoof_frame(), 0..12),
    ) {
        let mut agg = PadAggregator::with_defaults(ThresholdConfig::default());
        for frame in prefix.iter().chain(std::iter::once(&spoof)).chain(suffix.iter()) {
            agg.record(frame);
        }
        prop_assert_eq!(agg.decision(), ConsensusDecision::SpoofVetoed);
        let (verdict, reason) = agg.decision().verdict();
        prop_assert_eq!(verdict, Verdict::Deny);
        prop_assert_eq!(reason, ReasonClass::PadFailed);
        prop_assert!(verdict.should_ignore());
        prop_assert!(agg.spoof_frames() >= 1);
    }

    /// Without any spoof frame, `Allow` holds exactly when the last `k` frames all pass.
    #[test]
    fn prop_allow_iff_trailing_k_frames_pass_without_spoof(
        frames in prop::collection::vec(arb_non_spoof_frame(), 0..16),
    ) {
        let thresholds = ThresholdConfig::default();
        let mut agg = PadAggregator::with_defaults(thresholds);
        for frame in &frames {
            agg.record(frame);
        }
        let trailing_passing = frames
            .iter()
            .rev()
            .take_while(|f| {
                f.face_count == 1
                    && f.pad_live
                    && f.pad_score.is_finite()
                    && f.pad_score >= thresholds.pad_threshold()
                    && f.match_score.is_finite()
                    && f.match_score >= thresholds.match_threshold()
            })
            .count();
        let expected_allow = trailing_passing >= DEFAULT_PAD_CONSENSUS_REQUIRED;
        prop_assert_eq!(agg.decision() == ConsensusDecision::Allow, expected_allow);
        prop_assert_eq!(agg.decision().verdict().0 == Verdict::Allow, expected_allow);
    }
}

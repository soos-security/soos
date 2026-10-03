//! Contract tests of GitHub #329 for the evaluation lower bound of the shared consensus
//! (`RequestDeadline::with_not_before`, read by `consensus::run_face_consensus`) and the
//! presence settle constant (spec `AI/architect_spec_install_presence_warmup.md` §5.2,
//! matrix IWP11, IWP12).
//!
//! A `compute`d deadline has no lower bound (`not_before_ns = 0`): the PAM consensus is
//! unchanged; a capture stamped before the bound is never evaluated; a bound past the
//! deadline fails closed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{build_pipeline, PipelineOptions, PipelineParts, MS_NS};
use soos_daemon::consensus::{run_face_consensus, ConsensusContext, ConsensusRun};
use soos_daemon::inference::{
    InferenceGate, InferencePriority, RequestDeadline, RESPONSE_WRITE_MARGIN_MS,
};
use soos_daemon::pipeline::{current_monotonic_nanos, DECISION_BUDGET_MS};
use soos_daemon::presence::config::MIN_SCAN_INTERVAL_MS;
use soos_daemon::presence::PRESENCE_WAKE_SETTLE_MS;
use soos_policy::{ConsensusDecision, ThresholdConfig};

const BOTH: [InferencePriority; 2] = [
    InferencePriority::Interactive,
    InferencePriority::Background,
];

fn describe(run: &ConsensusRun) -> String {
    match run {
        ConsensusRun::Decided {
            decision,
            frames_evaluated,
            ..
        } => format!("Decided({decision:?}, frames {frames_evaluated})"),
        ConsensusRun::Aborted {
            verdict, reason, ..
        } => format!("Aborted({verdict:?}, {reason:?})"),
        ConsensusRun::Preempted => "Preempted".to_string(),
    }
}

/// The deadline of a run whose decision budget starts at `start_ns` (`extra_ms` later than
/// now for the outer bound).
fn deadline_from(start_ns: u64, extra_ms: u64) -> RequestDeadline {
    RequestDeadline::compute(
        start_ns,
        0,
        Instant::now(),
        Duration::from_millis(DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS + extra_ms),
    )
}

async fn run(
    parts: &PipelineParts,
    gate: &InferenceGate,
    priority: InferencePriority,
    deadline: RequestDeadline,
    not_before_ns: u64,
) -> ConsensusRun {
    let ctx = ConsensusContext {
        camera: &parts.components.camera,
        vision: &parts.components.vision,
        inference: gate,
        thresholds: ThresholdConfig::default(),
        clock: current_monotonic_nanos,
        priority,
        uid: 1000,
    };
    let deadline = if not_before_ns == 0 {
        deadline
    } else {
        deadline.with_not_before(not_before_ns)
    };
    run_face_consensus(&ctx, &parts.frame_embedding, deadline).await
}

/// IWP11: the settle is the presence constant of 1000 ms, never longer than one scan
/// interval.
#[test]
fn test_iwp_settle_constant_value() {
    assert_eq!(PRESENCE_WAKE_SETTLE_MS, 1000);
    const { assert!(PRESENCE_WAKE_SETTLE_MS > 0 && PRESENCE_WAKE_SETTLE_MS <= MIN_SCAN_INTERVAL_MS) };
}

/// IWP11: a `compute`d deadline carries no lower bound, and the consensus over it is the
/// unchanged PAM consensus: `Allow` after exactly `k = 3` passing captures, for both
/// priorities; an explicit bound of 0 is the same window.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_iwp_not_before_zero_is_the_pam_consensus() {
    for priority in BOTH {
        let parts = build_pipeline(PipelineOptions::new(current_monotonic_nanos)).await;
        let gate = InferenceGate::new(1, Duration::from_millis(80));
        let now = current_monotonic_nanos().unwrap();
        let deadline = deadline_from(now, 0);
        assert_eq!(deadline.not_before_ns(), 0, "compute sets no lower bound");
        assert_eq!(
            deadline.with_not_before(0),
            deadline,
            "a bound of 0 is no bound"
        );
        let result = run(&parts, &gate, priority, deadline, 0).await;
        match &result {
            ConsensusRun::Decided {
                decision: ConsensusDecision::Allow,
                frames_evaluated,
                ..
            } => assert_eq!(*frames_evaluated, 3, "{priority:?}: k = 3"),
            other => panic!("{priority:?}: expected Allow, got {}", describe(other)),
        }
        assert_eq!(
            parts.inferences(),
            3,
            "{priority:?}: no inference after Allow"
        );
    }
}

/// IWP12: captures stamped before `not_before_ns` are never evaluated (no PAD call before the
/// bound), later captures decide with the unchanged rules.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_iwp_not_before_skips_earlier_captures() {
    const SKIP_MS: u64 = 400;
    let parts = build_pipeline(PipelineOptions::new(current_monotonic_nanos)).await;
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let start = Instant::now();
    let now = current_monotonic_nanos().unwrap();
    let not_before = now + SKIP_MS * MS_NS;

    let first_seen = Arc::new(AtomicU64::new(u64::MAX));
    // `build_pipeline` evaluates the mock frame once to derive the enrolled embedding.
    let baseline = parts.pad.call_count();
    let observer = {
        let pad = Arc::clone(&parts.pad);
        let first_seen = Arc::clone(&first_seen);
        std::thread::spawn(move || {
            while start.elapsed() < Duration::from_secs(5) {
                if pad.call_count() > baseline {
                    first_seen.store(
                        u64::try_from(start.elapsed().as_millis()).unwrap(),
                        Ordering::SeqCst,
                    );
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        })
    };

    let result = run(
        &parts,
        &gate,
        InferencePriority::Background,
        deadline_from(not_before, SKIP_MS),
        not_before,
    )
    .await;
    observer.join().unwrap();
    let first_pad_ms = first_seen.load(Ordering::SeqCst);
    match &result {
        ConsensusRun::Decided {
            decision: ConsensusDecision::Allow,
            frames_evaluated,
            ..
        } => assert_eq!(*frames_evaluated, 3),
        other => panic!("expected Allow after the bound, got {}", describe(other)),
    }
    assert!(
        first_pad_ms != u64::MAX && first_pad_ms >= SKIP_MS,
        "no capture stamped before not_before_ns may be evaluated (first PAD call {first_pad_ms} ms)"
    );
}

/// IWP12: a bound past the deadline evaluates nothing and fails closed (`Pending`, no
/// inference, no PAD call), for both priorities.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_iwp_not_before_past_the_deadline_fails_closed() {
    for priority in BOTH {
        let parts = build_pipeline(PipelineOptions::new(current_monotonic_nanos)).await;
        let gate = InferenceGate::new(1, Duration::from_millis(80));
        let now = current_monotonic_nanos().unwrap();
        let far = now + 10_000 * MS_NS;
        let baseline = parts.pad.call_count();
        let result = run(&parts, &gate, priority, deadline_from(now, 0), far).await;
        match &result {
            ConsensusRun::Decided {
                decision: ConsensusDecision::Pending(_),
                frames_evaluated,
                ..
            } => assert_eq!(*frames_evaluated, 0, "{priority:?}: nothing evaluated"),
            other => panic!("{priority:?}: expected Pending, got {}", describe(other)),
        }
        assert_eq!(parts.inferences(), 0, "{priority:?}: no inference");
        assert_eq!(
            parts.pad.call_count(),
            baseline,
            "{priority:?}: no PAD evaluation"
        );
    }
}

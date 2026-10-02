//! Contract tests of GitHub #323 for the shared multi-frame PAD consensus extracted from the
//! dispatcher (`consensus::run_face_consensus`, matrix PAU10) and its background-priority
//! preemption (PAU12).
//!
//! The same function serves PAM (`Interactive`) and presence (`Background`), so the pipeline
//! can never diverge: `k = 3` consecutive passing captures, any spoof capture vetoes, and the
//! thresholds are the ones passed from the shared `AuthorizationEngine`.

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

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{
    build_pipeline, failing_clock, template_with_similarity, PipelineOptions, PipelineParts,
};
use soos_daemon::consensus::{
    run_face_consensus, wake_camera, ConsensusContext, ConsensusRun, CAMERA_WAKE_POLL_MS,
    DEFAULT_CAMERA_WAKE_WAIT_MS, MAX_CAMERA_WAKE_WAIT_MS,
};
use soos_daemon::inference::{
    InferenceGate, InferencePriority, InteractiveDemandGuard, RequestDeadline,
    RESPONSE_WRITE_MARGIN_MS,
};
use soos_daemon::pipeline::{current_monotonic_nanos, DECISION_BUDGET_MS};
use soos_daemon::DaemonError;
use soos_inference_ort::{AttackType, PadResult};
use soos_policy::{ConsensusDecision, ThresholdConfig};
use soos_protocol::types::{ReasonClass, Verdict};

fn describe(run: &ConsensusRun) -> String {
    match run {
        ConsensusRun::Decided {
            decision,
            frames_evaluated,
            consecutive_passing,
            ..
        } => format!(
            "Decided({decision:?}, frames {frames_evaluated}, consecutive {consecutive_passing})"
        ),
        ConsensusRun::Aborted {
            verdict, reason, ..
        } => format!("Aborted({verdict:?}, {reason:?})"),
        ConsensusRun::Preempted => "Preempted".to_string(),
    }
}

fn scan_deadline() -> RequestDeadline {
    RequestDeadline::compute(
        current_monotonic_nanos().unwrap(),
        0,
        Instant::now(),
        Duration::from_millis(DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS),
    )
}

async fn consensus(
    parts: &PipelineParts,
    gate: &InferenceGate,
    priority: InferencePriority,
    thresholds: ThresholdConfig,
    clock: fn() -> Result<u64, DaemonError>,
    template: &[f32],
) -> ConsensusRun {
    let ctx = ConsensusContext {
        camera: &parts.components.camera,
        vision: &parts.components.vision,
        inference: gate,
        thresholds,
        clock,
        priority,
        uid: 1000,
    };
    run_face_consensus(&ctx, template, scan_deadline()).await
}

async fn parts() -> PipelineParts {
    build_pipeline(PipelineOptions::new(current_monotonic_nanos)).await
}

const BOTH: [InferencePriority; 2] = [
    InferencePriority::Interactive,
    InferencePriority::Background,
];

/// The extraction keeps the former dispatcher literals as named constants.
#[test]
fn test_pau_camera_wake_constants_keep_the_dispatcher_values() {
    assert_eq!(MAX_CAMERA_WAKE_WAIT_MS, 1200);
    assert_eq!(DEFAULT_CAMERA_WAKE_WAIT_MS, 1000);
    assert_eq!(CAMERA_WAKE_POLL_MS, 15);
}

/// PAU10: `Allow` only after exactly `k = 3` consecutive passing captures, for both callers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_consensus_allows_after_exactly_three_passing_captures() {
    for priority in BOTH {
        let parts = parts().await;
        let gate = InferenceGate::new(1, Duration::from_millis(80));
        let template = parts.frame_embedding.clone();
        let run = consensus(
            &parts,
            &gate,
            priority,
            ThresholdConfig::default(),
            current_monotonic_nanos,
            &template,
        )
        .await;
        match &run {
            ConsensusRun::Decided {
                decision: ConsensusDecision::Allow,
                frames_evaluated,
                consecutive_passing,
                spoof_capture,
                ..
            } => {
                assert_eq!(*frames_evaluated, 3, "{priority:?}: k = 3 captures");
                assert_eq!(*consecutive_passing, 3, "{priority:?}");
                assert!(spoof_capture.is_none());
            }
            other => panic!("{priority:?}: expected Allow, got {}", describe(other)),
        }
        assert_eq!(
            parts.inferences(),
            3,
            "{priority:?}: no inference after Allow"
        );
    }
}

/// PAU10: one spoof capture vetoes the whole run (no Allow), for both callers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_consensus_one_spoof_capture_vetoes() {
    for priority in BOTH {
        let parts = parts().await;
        parts.pad.set_result_sequence(vec![
            PadResult::live(0.98),
            PadResult::live(0.98),
            PadResult::spoof(0.99, AttackType::ScreenReplay),
            PadResult::live(0.98),
            PadResult::live(0.98),
        ]);
        let gate = InferenceGate::new(1, Duration::from_millis(80));
        let template = parts.frame_embedding.clone();
        let run = consensus(
            &parts,
            &gate,
            priority,
            ThresholdConfig::default(),
            current_monotonic_nanos,
            &template,
        )
        .await;
        match &run {
            ConsensusRun::Decided {
                decision: ConsensusDecision::SpoofVetoed,
                spoof_capture,
                ..
            } => assert!(spoof_capture.is_some(), "the vetoing capture is reported"),
            other => panic!(
                "{priority:?}: expected SpoofVetoed, got {}",
                describe(other)
            ),
        }
    }
}

/// PAU10: the thresholds passed by the caller apply: a template at cosine 0.60 is allowed at
/// the default 0.50 and refused (no Allow) with `match_threshold = 0.70`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_consensus_applies_the_caller_thresholds() {
    for priority in BOTH {
        let parts = parts().await;
        let gate = InferenceGate::new(1, Duration::from_millis(80));
        let template = template_with_similarity(&parts.frame_embedding, 0.60);
        let allowed = consensus(
            &parts,
            &gate,
            priority,
            ThresholdConfig::default(),
            current_monotonic_nanos,
            &template,
        )
        .await;
        assert!(
            matches!(
                allowed,
                ConsensusRun::Decided {
                    decision: ConsensusDecision::Allow,
                    ..
                }
            ),
            "{priority:?}: cosine 0.60 passes the default 0.50, got {}",
            describe(&allowed)
        );

        let raised = ThresholdConfig::builder()
            .match_threshold(0.70)
            .build()
            .unwrap();
        let refused = consensus(
            &parts,
            &gate,
            priority,
            raised,
            current_monotonic_nanos,
            &template,
        )
        .await;
        assert!(
            matches!(
                refused,
                ConsensusRun::Decided {
                    decision: ConsensusDecision::Pending(_),
                    ..
                }
            ),
            "{priority:?}: a raised match_threshold must make the same score fail, got {}",
            describe(&refused)
        );
    }
}

/// PAU12: a background run never starts an inference while an interactive request is live.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_background_consensus_is_preempted_before_its_first_inference() {
    let parts = parts().await;
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let _demand = gate.register_interactive();
    let template = parts.frame_embedding.clone();
    let run = consensus(
        &parts,
        &gate,
        InferencePriority::Background,
        ThresholdConfig::default(),
        current_monotonic_nanos,
        &template,
    )
    .await;
    assert!(
        matches!(run, ConsensusRun::Preempted),
        "expected Preempted, got {}",
        describe(&run)
    );
    assert_eq!(parts.inferences(), 0);
}

/// PAU12: an interactive request appearing during a background run preempts it before its
/// next inference.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_background_consensus_is_preempted_between_captures() {
    let parts = parts().await;
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let guards: Arc<Mutex<Vec<InteractiveDemandGuard>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let gate = gate.clone();
        let guards = Arc::clone(&guards);
        parts.set_extractor_hook(move || {
            let mut held = guards.lock().unwrap();
            if held.is_empty() {
                held.push(gate.register_interactive());
            }
        });
    }
    let template = parts.frame_embedding.clone();
    let run = consensus(
        &parts,
        &gate,
        InferencePriority::Background,
        ThresholdConfig::default(),
        current_monotonic_nanos,
        &template,
    )
    .await;
    assert!(
        matches!(run, ConsensusRun::Preempted),
        "expected Preempted, got {}",
        describe(&run)
    );
    assert_eq!(
        parts.inferences(),
        1,
        "no inference may start once an interactive request is registered"
    );
}

/// PAU12: an interactive run is never preempted, whatever the demand counter says.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_interactive_consensus_is_never_preempted() {
    let parts = parts().await;
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let _demand_a = gate.register_interactive();
    let _demand_b = gate.register_interactive();
    let template = parts.frame_embedding.clone();
    let run = consensus(
        &parts,
        &gate,
        InferencePriority::Interactive,
        ThresholdConfig::default(),
        current_monotonic_nanos,
        &template,
    )
    .await;
    assert!(
        matches!(
            run,
            ConsensusRun::Decided {
                decision: ConsensusDecision::Allow,
                ..
            }
        ),
        "expected Allow, got {}",
        describe(&run)
    );
}

/// PAU12: a busy slot makes a background run poll (never queue); it decides once free.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_background_consensus_polls_a_busy_slot_without_queueing() {
    let parts = parts().await;
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let held = gate.acquire_within(Duration::ZERO).await.unwrap();
    let releaser = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(held);
    });
    let template = parts.frame_embedding.clone();
    let run = consensus(
        &parts,
        &gate,
        InferencePriority::Background,
        ThresholdConfig::default(),
        current_monotonic_nanos,
        &template,
    )
    .await;
    releaser.await.unwrap();
    assert!(
        matches!(
            run,
            ConsensusRun::Decided {
                decision: ConsensusDecision::Allow,
                ..
            }
        ),
        "expected Allow once the slot is free, got {}",
        describe(&run)
    );
}

/// `VisionError::Inference` aborts with the dispatcher verdict (`Unavailable` /
/// `ModelUnavailable`), for both callers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_consensus_aborts_on_vision_inference_error() {
    for priority in BOTH {
        let parts = parts().await;
        parts.detector.set_fail_next(true);
        let gate = InferenceGate::new(1, Duration::from_millis(80));
        let template = parts.frame_embedding.clone();
        let run = consensus(
            &parts,
            &gate,
            priority,
            ThresholdConfig::default(),
            current_monotonic_nanos,
            &template,
        )
        .await;
        match run {
            ConsensusRun::Aborted {
                verdict, reason, ..
            } => {
                assert_eq!(verdict, Verdict::Unavailable);
                assert_eq!(reason, ReasonClass::ModelUnavailable);
            }
            other => panic!("{priority:?}: expected Aborted, got {}", describe(&other)),
        }
    }
}

/// A panicking inference job aborts (`Unavailable` / `InternalError`), never allows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_consensus_aborts_on_inference_job_panic() {
    let parts = parts().await;
    parts.set_extractor_hook(|| panic!("injected inference job panic"));
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let template = parts.frame_embedding.clone();
    let run = consensus(
        &parts,
        &gate,
        InferencePriority::Background,
        ThresholdConfig::default(),
        current_monotonic_nanos,
        &template,
    )
    .await;
    match run {
        ConsensusRun::Aborted {
            verdict, reason, ..
        } => {
            assert_eq!(verdict, Verdict::Unavailable);
            assert_eq!(reason, ReasonClass::InternalError);
        }
        other => panic!("expected Aborted, got {}", describe(&other)),
    }
}

/// A failing clock aborts (`Unavailable` / `InternalError`) and reports the error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_consensus_aborts_on_clock_failure() {
    for priority in BOTH {
        let parts = parts().await;
        let gate = InferenceGate::new(1, Duration::from_millis(80));
        let template = parts.frame_embedding.clone();
        let run = consensus(
            &parts,
            &gate,
            priority,
            ThresholdConfig::default(),
            failing_clock,
            &template,
        )
        .await;
        match run {
            ConsensusRun::Aborted {
                verdict,
                reason,
                completion_error,
            } => {
                assert_eq!(verdict, Verdict::Unavailable);
                assert_eq!(reason, ReasonClass::InternalError);
                assert!(completion_error.is_some(), "the clock error is reported");
            }
            other => panic!("{priority:?}: expected Aborted, got {}", describe(&other)),
        }
        assert_eq!(parts.inferences(), 0);
    }
}

/// `wake_camera` returns at once for a ready camera and gives up after the bound for a
/// camera that never becomes ready.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_wake_camera_is_bounded() {
    let parts = parts().await;
    let started = Instant::now();
    assert!(
        wake_camera(
            &parts.components.camera,
            Duration::from_millis(MAX_CAMERA_WAKE_WAIT_MS)
        )
        .await
    );
    assert!(started.elapsed() < Duration::from_millis(MAX_CAMERA_WAKE_WAIT_MS));

    parts
        .camera
        .never_ready
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let started = Instant::now();
    assert!(!wake_camera(&parts.components.camera, Duration::from_millis(200)).await);
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(150),
        "the wait honours its bound before giving up, took {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(1200),
        "the wait never exceeds its bound by much, took {elapsed:?}"
    );
}

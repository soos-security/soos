//! Shared camera wake and multi-frame PAD consensus (GitHub #147 / PAD-02, GitHub #323).
//!
//! Extracted verbatim from the dispatcher Step 8d-2 / 8e so that the PAM `Auth` path and the
//! presence auto-unlock scanner run exactly the same pipeline: capture freshness, inference
//! admission against the request deadline, `process_frame`, cosine match against the
//! enrolled template and the [`PadAggregator`] decision (`k` consecutive passing captures,
//! any spoof capture vetoes). The only difference between the callers is
//! [`InferencePriority`]: a `Background` run never waits for the inference slot and yields to
//! any interactive (PAM) request before each inference.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{debug, warn};

use crate::error::DaemonError;
use crate::inference::{InferenceGate, InferencePriority, RequestDeadline};
use crate::pipeline::{FRAME_POLL_INTERVAL_MS, MAX_FRAME_AGE_NS};
use soos_camera_v4l::{CameraManager, Frame};
use soos_policy::{ConsensusDecision, FrameClass, FrameEvaluation, PadAggregator, ThresholdConfig};
use soos_protocol::types::{ReasonClass, Verdict};
use soos_vision::VisionPipeline;

/// Upper bound of the camera wake wait for one decision (formerly the literal 1200 ms of
/// dispatcher Step 8d-2).
pub const MAX_CAMERA_WAKE_WAIT_MS: u64 = 1200;

/// Camera wake wait used when the client sent no usable deadline (formerly the literal
/// 1000 ms of dispatcher Step 8d-2).
pub const DEFAULT_CAMERA_WAKE_WAIT_MS: u64 = 1000;

/// Poll interval of the camera wake wait (formerly the literal 15 ms).
pub const CAMERA_WAKE_POLL_MS: u64 = 15;

/// Everything one consensus run reads; borrowed, nothing owned.
pub struct ConsensusContext<'a> {
    /// Shared camera manager (only `latest_frame` is used here).
    pub camera: &'a Arc<dyn CameraManager>,
    /// Shared vision pipeline.
    pub vision: &'a Arc<VisionPipeline>,
    /// Shared inference gate (single slot).
    pub inference: &'a InferenceGate,
    /// Thresholds copied from the shared `AuthorizationEngine`.
    pub thresholds: ThresholdConfig,
    /// Monotonic clock of the caller.
    pub clock: fn() -> Result<u64, DaemonError>,
    /// Who runs the consensus.
    pub priority: InferencePriority,
    /// Target UID, for log lines only.
    pub uid: u32,
}

impl std::fmt::Debug for ConsensusContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsensusContext")
            .field("camera", &"<dyn CameraManager>")
            .field("vision", &"<VisionPipeline>")
            .field("priority", &self.priority)
            .field("uid", &self.uid)
            .finish_non_exhaustive()
    }
}

/// Outcome of one consensus run.
#[derive(Debug)]
pub enum ConsensusRun {
    /// The loop ended normally (`Allow`, `SpoofVetoed`, or `Pending` at the deadline).
    Decided {
        /// Aggregate decision.
        decision: ConsensusDecision,
        /// Captures evaluated by the aggregator.
        frames_evaluated: usize,
        /// Consecutive passing captures at the end of the run.
        consecutive_passing: usize,
        /// Whether the last capture seen was stale.
        last_capture_stale: bool,
        /// First spoof-classified capture (only used by the `Auth` evidence path).
        spoof_capture: Option<Arc<Frame>>,
    },
    /// A failure the dispatcher answers with this verdict (clock error, inference job
    /// panic/cancel, `VisionError::Inference`, other `VisionError`).
    Aborted {
        /// Verdict to render.
        verdict: Verdict,
        /// Reason to render.
        reason: ReasonClass,
        /// Error reported after the response, if any.
        completion_error: Option<DaemonError>,
    },
    /// Background priority only: an interactive request appeared; nothing was decided.
    Preempted,
}

/// Whether a capture taken at `timestamp_ns` is still fresh at `now_ns` (`MAX_FRAME_AGE_NS`).
///
/// Captures without a timestamp, or stamped in the future, are accepted as before.
fn is_frame_fresh(timestamp_ns: u64, now_ns: u64) -> bool {
    if timestamp_ns > 0 && now_ns > timestamp_ns {
        now_ns.saturating_sub(timestamp_ns) <= MAX_FRAME_AGE_NS
    } else {
        true
    }
}

/// Waits at most `max_wake` for the camera to become ready (call `notify_activity` first).
///
/// Returns `camera.is_ready()` at the end of the wait; returns at once for a ready camera.
pub async fn wake_camera(camera: &Arc<dyn CameraManager>, max_wake: Duration) -> bool {
    if !camera.is_ready() {
        let wake_start = Instant::now();
        while !camera.is_ready() && wake_start.elapsed() < max_wake {
            tokio::time::sleep(Duration::from_millis(CAMERA_WAKE_POLL_MS)).await;
        }
    }
    camera.is_ready()
}

/// Builds the `Decided` outcome from the aggregator state.
fn decided(
    aggregator: &PadAggregator,
    last_capture_stale: bool,
    spoof_capture: Option<Arc<Frame>>,
) -> ConsensusRun {
    ConsensusRun::Decided {
        decision: aggregator.decision(),
        frames_evaluated: usize::try_from(aggregator.frames_evaluated()).unwrap_or(usize::MAX),
        consecutive_passing: aggregator.consecutive_passing(),
        last_capture_stale,
        spoof_capture,
    }
}

/// `Aborted` with no completion error.
const fn aborted(verdict: Verdict, reason: ReasonClass) -> ConsensusRun {
    ConsensusRun::Aborted {
        verdict,
        reason,
        completion_error: None,
    }
}

/// Runs the multi-frame PAD consensus against `template` until `deadline`.
///
/// `Interactive`: identical to the former dispatcher Step 8e (`acquire_within`, never
/// `Preempted`). `Background`: before every inference it returns `Preempted` if an
/// interactive request is registered, and acquires the slot only through
/// [`InferenceGate::try_acquire_background`] (a busy slot is polled again after
/// `FRAME_POLL_INTERVAL_MS`, never waited for).
#[allow(
    clippy::too_many_lines,
    reason = "verbatim extraction of the dispatcher consensus loop (GitHub #323)"
)]
pub async fn run_face_consensus(
    ctx: &ConsensusContext<'_>,
    template: &[f32],
    deadline: RequestDeadline,
) -> ConsensusRun {
    // Multi-frame PAD consensus loop (GitHub #147 / PAD-02).
    // Allow requires k consecutive passing captures (live at or above the PAD
    // threshold and matching at or above the cosine threshold) inside a bounded
    // window; any spoof-classified capture vetoes the whole request (fail closed).
    let mut aggregator = PadAggregator::with_defaults(ctx.thresholds);
    let mut last_sequence: Option<u64> = None;
    let mut last_capture_stale = false;
    // First capture classified as a presentation attack (GitHub #261 / PAD-14),
    // kept only to seal it as opt-in evidence once the verdict is rendered.
    let mut spoof_capture: Option<Arc<Frame>> = None;

    loop {
        let cur_ns = match (ctx.clock)() {
            Ok(ns) => ns,
            Err(err) => {
                warn!(error = %err, "Monotonic clock query failed checking loop deadline");
                return ConsensusRun::Aborted {
                    verdict: Verdict::Unavailable,
                    reason: ReasonClass::InternalError,
                    completion_error: Some(err),
                };
            }
        };

        if deadline.remaining(cur_ns).is_zero() {
            break;
        }

        if let Some(frame) = ctx.camera.latest_frame() {
            let is_new = match last_sequence {
                Some(seq) => frame.sequence != seq,
                None => true,
            };

            if is_new {
                last_sequence = Some(frame.sequence);

                if is_frame_fresh(frame.timestamp_mono_ns, cur_ns) {
                    last_capture_stale = false;

                    // Background priority (GitHub #323): never start an inference while a
                    // PAM request is registered.
                    if ctx.priority == InferencePriority::Background
                        && ctx.inference.interactive_demand() > 0
                    {
                        debug!("Interactive request registered; background consensus preempted");
                        return ConsensusRun::Preempted;
                    }

                    // Inference admission (GitHub #158 / #159). Never start an
                    // inference whose estimated duration exceeds the remaining budget,
                    // and never queue behind a busy inference slot past the last
                    // feasible start time: finalize with the current consensus instead.
                    let estimate = ctx.inference.estimate();
                    if !deadline.can_start(cur_ns, estimate) {
                        debug!(
                            estimate_ms = estimate.as_millis(),
                            "Remaining budget below the inference estimate; finalizing"
                        );
                        // GitHub #315 (DMN-NEW-2): with no capture evaluated, no
                        // measurement would ever lower an estimate above every
                        // client budget; decay it so face auth recovers. This
                        // request still fails closed.
                        if aggregator.frames_evaluated() == 0 {
                            ctx.inference.decay_estimate();
                        }
                        break;
                    }
                    let permit = match ctx.priority {
                        InferencePriority::Interactive => {
                            let max_wait = deadline.remaining(cur_ns).saturating_sub(estimate);
                            let Some(permit) = ctx.inference.acquire_within(max_wait).await else {
                                debug!("Inference slot still busy at the last feasible start; finalizing");
                                break;
                            };
                            Some(permit)
                        }
                        InferencePriority::Background => ctx.inference.try_acquire_background(),
                    };
                    if let Some(permit) = permit {
                        let start_ns = (ctx.clock)().unwrap_or(u64::MAX);
                        if !deadline.can_start(start_ns, ctx.inference.estimate()) {
                            drop(permit);
                            break;
                        }
                        if !is_frame_fresh(frame.timestamp_mono_ns, start_ns) {
                            // Aged while waiting for the slot: never evaluate it.
                            drop(permit);
                            continue;
                        }

                        // CPU inference on the bounded blocking pool; the async
                        // worker stays free for the accept loop and Status requests.
                        let vision = Arc::clone(ctx.vision);
                        let job_frame = Arc::clone(&frame);
                        let result = match ctx
                            .inference
                            .run(permit, move || vision.process_frame(&job_frame))
                            .await
                        {
                            Ok(result) => result,
                            Err(err) => {
                                warn!(error = %err, "Vision inference job failed; failing closed");
                                return aborted(Verdict::Unavailable, ReasonClass::InternalError);
                            }
                        };

                        let evaluation = match result {
                            Ok(output) => {
                                let sim = match soos_vision::cosine_similarity(
                                    template,
                                    output.embedding.as_slice(),
                                ) {
                                    Ok(s) => s,
                                    Err(e) => {
                                        warn!(error = %e, "Cosine similarity calculation error");
                                        0.0
                                    }
                                };
                                FrameEvaluation::new(
                                    1,
                                    output.pad_result.is_live,
                                    output.pad_result.score,
                                    sim,
                                )
                            }
                            Err(soos_vision::VisionError::PadFailed { score, threshold }) => {
                                debug!(
                                    score = score,
                                    threshold = threshold,
                                    "Presentation attack detected (PAD failed)"
                                );
                                FrameEvaluation::spoof(score)
                            }
                            Err(soos_vision::VisionError::IrLivenessGateFailed { reason }) => {
                                debug!(
                                    reason = %reason,
                                    "Presentation attack detected (IR liveness gate)"
                                );
                                FrameEvaluation::spoof(0.0)
                            }
                            Err(soos_vision::VisionError::NoFaceDetected) => {
                                debug!("Zero faces detected in capture");
                                FrameEvaluation::no_face()
                            }
                            Err(soos_vision::VisionError::MultipleFacesDetected { count }) => {
                                let count_u8 = u8::try_from(count).unwrap_or(u8::MAX);
                                debug!(count = count, "Multiple faces detected in capture");
                                FrameEvaluation::multiple_faces(count_u8)
                            }
                            Err(soos_vision::VisionError::FaceBelowConfidence { .. }) => {
                                debug!("Face detected below confidence threshold");
                                FrameEvaluation::no_face()
                            }
                            // Pre-PAD quality gate (GitHub #218): an unusable capture,
                            // never a pass and never an internal error.
                            Err(
                                soos_vision::VisionError::FaceTooSmall { .. }
                                | soos_vision::VisionError::FaceBlurred { .. },
                            ) => {
                                debug!("Face rejected by the pre-PAD quality gate");
                                FrameEvaluation::no_face()
                            }
                            Err(soos_vision::VisionError::Inference(err)) => {
                                warn!(error = %err, "Vision neural inference failure");
                                return aborted(
                                    Verdict::Unavailable,
                                    ReasonClass::ModelUnavailable,
                                );
                            }
                            Err(err) => {
                                warn!(error = %err, "Vision pipeline processing error");
                                return aborted(Verdict::Unavailable, ReasonClass::InternalError);
                            }
                        };

                        let class = aggregator.record(&evaluation);
                        if class == FrameClass::Spoof && spoof_capture.is_none() {
                            spoof_capture = Some(Arc::clone(&frame));
                        }
                        match aggregator.decision() {
                            ConsensusDecision::Allow => break,
                            ConsensusDecision::SpoofVetoed => {
                                warn!(
                                    uid = ctx.uid,
                                    captures_evaluated = aggregator.frames_evaluated(),
                                    "Presentation attack detected; vetoing request"
                                );
                                break;
                            }
                            ConsensusDecision::Pending(_) => {
                                debug!(
                                    class = ?class,
                                    consecutive_live = aggregator.consecutive_passing(),
                                    required = aggregator.config().required(),
                                    "Capture recorded; consensus pending"
                                );
                            }
                        }
                    } else {
                        // Background only: the slot is busy; poll again, never queue.
                        debug!("Inference slot busy; background consensus polls again");
                    }
                } else {
                    last_capture_stale = true;
                }
            }
        }

        // Deadline-aware poll: never sleep past the decision budget so the response
        // is always rendered before the connection timeout.
        let remaining = deadline.remaining((ctx.clock)().unwrap_or(u64::MAX));
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(FRAME_POLL_INTERVAL_MS).min(remaining)).await;
    }

    decided(&aggregator, last_capture_stale, spoof_capture)
}

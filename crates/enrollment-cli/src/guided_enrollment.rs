//! Guided multi-step biometric enrollment state machine and multi-sample template fusion.
//!
//! Guides the user through Frontal, Turn Left, Turn Right, and Tilt Up poses
//! while ensuring liveness and face stability. Fuses collected multi-angle embedding samples
//! into a single normalized composite vector for robust real-time PAM unlock performance.
//!
//! # Sample admission (GitHub #183 / STO-10)
//!
//! A sample is recorded only if all of the following hold:
//! - the head pose is finite and inside the documented range of the current step
//!   ([`EnrollmentStepFeedback::PoseOutOfRange`] beyond the upper bound);
//! - the embedding is non-empty, at most [`MAX_GUIDED_EMBEDDING_DIM`] components, has the
//!   same dimension as every previous sample, contains only finite values and has a
//!   non-zero L2 norm ([`EnrollmentStepFeedback::InvalidEmbedding`] otherwise);
//! - the embedding is identity-consistent: its cosine similarity is at least
//!   [`MIN_SAMPLE_CONSISTENCY_COSINE`] with every accepted frontal sample (pairwise
//!   consistency of the frontal anchor set) and, for off-axis steps, with the frontal
//!   anchor mean direction ([`EnrollmentStepFeedback::IdentityMismatch`] otherwise).
//!
//! Each step records at most [`MAX_SAMPLES_PER_STEP`] samples.
//!
//! # Session-level liveness (GitHub #217 / PAD-12)
//!
//! Liveness is tracked over the session, not per frame, according to a [`LivenessPolicy`]:
//! - a sample is recorded only after `min_consecutive_live_frames` consecutive live frames
//!   (the streak is broken by a spoof frame or by
//!   [`GuidedEnrollmentSession::interrupt_liveness_streak`]); a live frame that is not yet
//!   sampled yields [`EnrollmentStepFeedback::PromptHoldStill`];
//! - a spoof frame discards every sample of the current step and resets the streak
//!   ([`EnrollmentStepFeedback::SpoofDetected`]);
//! - the `max_spoof_events`-th spoof frame aborts the session: every sample of every step
//!   is discarded, [`EnrollmentStepFeedback::SessionAborted`] is returned from then on and
//!   the composite template can no longer be computed.
//!
//! [`GuidedEnrollmentSession::new`] keeps a single-frame gate
//! ([`LivenessPolicy::single_frame`]) for API compatibility; production callers (the GUI)
//! use [`LivenessPolicy::strict`] through [`GuidedEnrollmentSession::with_liveness_policy`].
//!
//! # Fusion
//!
//! The composite template is the mean direction of all accepted samples:
//! `t = m / ||m||` with `m = sum_i s_i / ||s_i||`. Fusion re-validates every sample
//! and fails closed on any non-finite value, dimension mismatch or identity inconsistency.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    reason = "Numerical vector normalization, progress percentage calculation, and bounded indexing"
)]

use soos_vision::pose::HeadPose;

/// Minimum cosine similarity between a candidate sample and the frontal identity anchor
/// (every accepted frontal sample and, for off-axis steps, their mean direction).
///
/// Conservative lower bound: same-identity pairs of the embedding model within +/-25 degrees of pose
/// stay well above it, while a different identity is typically near 0.
pub const MIN_SAMPLE_CONSISTENCY_COSINE: f32 = 0.5;

/// Maximum number of samples recorded per enrollment step.
pub const MAX_SAMPLES_PER_STEP: usize = 20;

/// Maximum accepted embedding dimension for a guided enrollment sample.
pub const MAX_GUIDED_EMBEDDING_DIM: usize = 2048;

/// Default number of consecutive live frames required before a sample is recorded.
///
/// Mirrors the daemon PAD consensus (`DEFAULT_PAD_CONSENSUS_REQUIRED` = 3).
pub const DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES: usize = 3;

/// Default number of spoof frames that aborts a guided enrollment session.
pub const DEFAULT_MAX_SPOOF_EVENTS: usize = 3;

/// Upper bound applied to both [`LivenessPolicy`] values (they are clamped to `1..=32`).
pub const MAX_LIVENESS_POLICY_BOUND: usize = 32;

/// Session-level liveness policy of a guided enrollment session (GitHub #217).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LivenessPolicy {
    /// Consecutive live frames required before a sample is recorded (clamped to `1..=32`).
    pub min_consecutive_live_frames: usize,
    /// Spoof frames that abort the session (clamped to `1..=32`).
    pub max_spoof_events: usize,
}

impl LivenessPolicy {
    /// Production policy: [`DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES`] consecutive live frames
    /// per sample, abort after [`DEFAULT_MAX_SPOOF_EVENTS`] spoof frames.
    pub const fn strict() -> Self {
        Self {
            min_consecutive_live_frames: DEFAULT_MIN_CONSECUTIVE_LIVE_FRAMES,
            max_spoof_events: DEFAULT_MAX_SPOOF_EVENTS,
        }
    }

    /// Legacy single-frame gate (used by [`GuidedEnrollmentSession::new`]); spoof frames
    /// still reset the current step and abort after [`DEFAULT_MAX_SPOOF_EVENTS`].
    pub const fn single_frame() -> Self {
        Self {
            min_consecutive_live_frames: 1,
            max_spoof_events: DEFAULT_MAX_SPOOF_EVENTS,
        }
    }

    /// Returns the policy with both values clamped to `1..=MAX_LIVENESS_POLICY_BOUND`.
    fn clamped(self) -> Self {
        Self {
            min_consecutive_live_frames: self
                .min_consecutive_live_frames
                .clamp(1, MAX_LIVENESS_POLICY_BOUND),
            max_spoof_events: self.max_spoof_events.clamp(1, MAX_LIVENESS_POLICY_BOUND),
        }
    }
}

impl Default for LivenessPolicy {
    fn default() -> Self {
        Self::strict()
    }
}

/// Minimum L2 norm below which an embedding is considered degenerate.
const MIN_EMBEDDING_NORM: f32 = 1e-6;

/// Maximum absolute roll (degrees) accepted in any step.
const MAX_ROLL_DEG: f32 = 10.0;

/// Frontal step: maximum absolute yaw (degrees).
const FRONTAL_MAX_YAW_DEG: f32 = 8.0;

/// Frontal step: maximum absolute pitch (degrees).
const FRONTAL_MAX_PITCH_DEG: f32 = 10.0;

/// Turn steps: minimum absolute yaw (degrees).
const TURN_MIN_YAW_DEG: f32 = 10.0;

/// Turn steps: maximum absolute yaw (degrees).
const TURN_MAX_YAW_DEG: f32 = 25.0;

/// Tilt-up step: lowest accepted pitch (degrees, negative is upward).
const TILT_MIN_PITCH_DEG: f32 = -25.0;

/// Tilt-up step: highest accepted pitch (degrees, negative is upward).
const TILT_MAX_PITCH_DEG: f32 = -8.0;

/// Off-axis steps: maximum absolute value (degrees) of the rotation that is not the
/// target of the step (pitch while turning, yaw while tilting).
const OFF_AXIS_MAX_SECONDARY_DEG: f32 = 15.0;

/// Steps in the multi-angle guided enrollment lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentStep {
    /// Frontal head orientation (looking straight into camera).
    Frontal,
    /// Head turned slightly to the left (yaw in [-25°, -10°]).
    TurnLeft,
    /// Head turned slightly to the right (yaw in [+10°, +25°]).
    TurnRight,
    /// Head tilted slightly upward (pitch in [-25°, -8°]).
    TiltUp,
    /// All poses successfully captured and validated.
    Completed,
}

/// Instantaneous feedback returned during frame processing in guided enrollment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrollmentStepFeedback {
    /// Presentation attack detection rejected frame (spoof suspected).
    SpoofDetected,
    /// Face not centered in target bounds.
    PromptCenterFace,
    /// Waiting for head to turn slightly left.
    PromptTurnLeft,
    /// Waiting for head to turn slightly right.
    PromptTurnRight,
    /// Waiting for head to tilt slightly up.
    PromptTiltUp,
    /// Waiting for head to hold still / level.
    PromptHoldStill,
    /// Valid sample accepted for current step.
    SampleAccepted {
        step: EnrollmentStep,
        collected: usize,
        target: usize,
    },
    /// Step target reached, transitioning to next step.
    StepCompleted { next_step: EnrollmentStep },
    /// All enrollment steps finished.
    AllStepsCompleted,
    /// Embedding rejected: non-finite, zero-norm, oversized or dimension-inconsistent.
    InvalidEmbedding,
    /// Sample rejected: not identity-consistent with the frontal anchor samples.
    IdentityMismatch,
    /// Head rotation exceeds the documented range of the current step.
    PoseOutOfRange,
    /// Session aborted after repeated spoof frames; every sample was discarded and a new
    /// session must be started (GitHub #217).
    SessionAborted,
    /// Frame rejected by the pre-PAD face quality gate (face too small or blurred,
    /// GitHub #218); reported by the caller, never produced by `process_sample`.
    FaceQualityTooLow,
}

/// State machine coordinating interactive multi-angle enrollment.
#[derive(Debug, Clone)]
pub struct GuidedEnrollmentSession {
    target_samples_per_step: usize,
    current_step: EnrollmentStep,
    frontal_samples: Vec<Vec<f32>>,
    left_samples: Vec<Vec<f32>>,
    right_samples: Vec<Vec<f32>>,
    tilt_samples: Vec<Vec<f32>>,
    liveness: LivenessPolicy,
    live_streak: usize,
    spoof_events: usize,
    aborted: bool,
}

impl GuidedEnrollmentSession {
    /// Creates a new guided enrollment session with target valid samples per pose step.
    ///
    /// Uses [`LivenessPolicy::single_frame`]; production callers should use
    /// [`Self::with_liveness_policy`] with [`LivenessPolicy::strict`].
    pub fn new(target_samples_per_step: usize) -> Self {
        Self::with_liveness_policy(target_samples_per_step, LivenessPolicy::single_frame())
    }

    /// Creates a new guided enrollment session with an explicit session-level liveness
    /// policy (values clamped to `1..=MAX_LIVENESS_POLICY_BOUND`).
    pub fn with_liveness_policy(target_samples_per_step: usize, policy: LivenessPolicy) -> Self {
        let capacity = target_samples_per_step.clamp(1, MAX_SAMPLES_PER_STEP);
        Self {
            target_samples_per_step: capacity,
            current_step: EnrollmentStep::Frontal,
            frontal_samples: Vec::with_capacity(capacity),
            left_samples: Vec::with_capacity(capacity),
            right_samples: Vec::with_capacity(capacity),
            tilt_samples: Vec::with_capacity(capacity),
            liveness: policy.clamped(),
            live_streak: 0,
            spoof_events: 0,
            aborted: false,
        }
    }

    /// Returns `true` once repeated spoof frames aborted the session.
    pub fn is_aborted(&self) -> bool {
        self.aborted
    }

    /// Number of spoof frames observed in this session.
    pub fn spoof_events(&self) -> usize {
        self.spoof_events
    }

    /// Breaks the consecutive-live streak without counting a spoof event.
    ///
    /// Callers invoke it for frames that carry no PAD verdict (no face, failed crop,
    /// quality-gate rejection), so a sample always follows an uninterrupted live run.
    pub fn interrupt_liveness_streak(&mut self) {
        self.live_streak = 0;
    }

    /// Counts a spoof frame that carries no usable sample (no pose or no embedding, e.g. an
    /// alignment failure after the PAD verdict), exactly as [`Self::process_sample`] counts a
    /// non-live sample: an aborted or completed session is left unchanged (GitHub #285).
    pub fn record_presentation_attack(&mut self) -> EnrollmentStepFeedback {
        if self.aborted {
            return EnrollmentStepFeedback::SessionAborted;
        }
        if self.current_step == EnrollmentStep::Completed {
            return EnrollmentStepFeedback::AllStepsCompleted;
        }
        self.record_spoof()
    }

    /// Handles a spoof frame: resets the streak and the current step, aborts the session
    /// once `max_spoof_events` is reached.
    fn record_spoof(&mut self) -> EnrollmentStepFeedback {
        self.live_streak = 0;
        self.spoof_events = self.spoof_events.saturating_add(1);
        if self.spoof_events >= self.liveness.max_spoof_events {
            self.aborted = true;
            self.frontal_samples.clear();
            self.left_samples.clear();
            self.right_samples.clear();
            self.tilt_samples.clear();
            return EnrollmentStepFeedback::SessionAborted;
        }
        match self.current_step {
            EnrollmentStep::Frontal => self.frontal_samples.clear(),
            EnrollmentStep::TurnLeft => self.left_samples.clear(),
            EnrollmentStep::TurnRight => self.right_samples.clear(),
            EnrollmentStep::TiltUp => self.tilt_samples.clear(),
            EnrollmentStep::Completed => {}
        }
        EnrollmentStepFeedback::SpoofDetected
    }

    /// Returns the currently active step.
    pub fn current_step(&self) -> EnrollmentStep {
        self.current_step
    }

    /// Returns overall completion progress as a percentage in [0.0, 100.0].
    pub fn progress_percent(&self) -> f32 {
        if self.aborted {
            return 0.0;
        }
        let total_target = (self.target_samples_per_step * 4) as f32;
        if total_target == 0.0 {
            return 0.0;
        }

        let total_collected = (self.frontal_samples.len()
            + self.left_samples.len()
            + self.right_samples.len()
            + self.tilt_samples.len()) as f32;

        (total_collected / total_target * 100.0).clamp(0.0, 100.0)
    }

    /// Evaluates a frame analysis and records valid embedding samples.
    ///
    /// See the module documentation for the admission rules. Rejected samples are never
    /// recorded and never contribute to the fused template.
    pub fn process_sample(
        &mut self,
        pose: &HeadPose,
        embedding: &[f32],
        is_live: bool,
        is_centered: bool,
    ) -> EnrollmentStepFeedback {
        if self.aborted {
            return EnrollmentStepFeedback::SessionAborted;
        }

        if self.current_step == EnrollmentStep::Completed {
            return EnrollmentStepFeedback::AllStepsCompleted;
        }

        if !is_live {
            return self.record_spoof();
        }
        self.live_streak = self
            .live_streak
            .saturating_add(1)
            .min(MAX_LIVENESS_POLICY_BOUND);

        if !is_centered {
            return EnrollmentStepFeedback::PromptCenterFace;
        }

        if embedding.is_empty() {
            return EnrollmentStepFeedback::PromptHoldStill;
        }

        if !(pose.yaw.is_finite() && pose.pitch.is_finite() && pose.roll.is_finite()) {
            return EnrollmentStepFeedback::PromptHoldStill;
        }

        if let Some(feedback) = Self::pose_feedback(self.current_step, pose) {
            return feedback;
        }

        if !self.is_valid_embedding(embedding) {
            return EnrollmentStepFeedback::InvalidEmbedding;
        }

        if !self.is_identity_consistent(embedding) {
            return EnrollmentStepFeedback::IdentityMismatch;
        }

        let target = self.target_samples_per_step;
        let step = self.current_step;
        let (samples, next_step) = match step {
            EnrollmentStep::Frontal => (&mut self.frontal_samples, EnrollmentStep::TurnLeft),
            EnrollmentStep::TurnLeft => (&mut self.left_samples, EnrollmentStep::TurnRight),
            EnrollmentStep::TurnRight => (&mut self.right_samples, EnrollmentStep::TiltUp),
            EnrollmentStep::TiltUp => (&mut self.tilt_samples, EnrollmentStep::Completed),
            EnrollmentStep::Completed => return EnrollmentStepFeedback::AllStepsCompleted,
        };
        if samples.len() >= target || self.live_streak < self.liveness.min_consecutive_live_frames {
            return EnrollmentStepFeedback::PromptHoldStill;
        }

        samples.push(embedding.to_vec());
        let collected = samples.len();
        if collected >= target {
            self.current_step = next_step;
            if next_step == EnrollmentStep::Completed {
                EnrollmentStepFeedback::AllStepsCompleted
            } else {
                EnrollmentStepFeedback::StepCompleted { next_step }
            }
        } else {
            EnrollmentStepFeedback::SampleAccepted {
                step,
                collected,
                target,
            }
        }
    }

    /// Returns the pose feedback for `step`, or `None` when the pose is in range.
    fn pose_feedback(step: EnrollmentStep, pose: &HeadPose) -> Option<EnrollmentStepFeedback> {
        if pose.roll.abs() > MAX_ROLL_DEG {
            return Some(EnrollmentStepFeedback::PromptHoldStill);
        }
        match step {
            EnrollmentStep::Frontal => {
                if pose.yaw.abs() > FRONTAL_MAX_YAW_DEG {
                    Some(EnrollmentStepFeedback::PromptCenterFace)
                } else if pose.pitch.abs() > FRONTAL_MAX_PITCH_DEG {
                    Some(EnrollmentStepFeedback::PromptHoldStill)
                } else {
                    None
                }
            }
            EnrollmentStep::TurnLeft => {
                if pose.yaw > -TURN_MIN_YAW_DEG {
                    Some(EnrollmentStepFeedback::PromptTurnLeft)
                } else if pose.yaw < -TURN_MAX_YAW_DEG
                    || pose.pitch.abs() > OFF_AXIS_MAX_SECONDARY_DEG
                {
                    Some(EnrollmentStepFeedback::PoseOutOfRange)
                } else {
                    None
                }
            }
            EnrollmentStep::TurnRight => {
                if pose.yaw < TURN_MIN_YAW_DEG {
                    Some(EnrollmentStepFeedback::PromptTurnRight)
                } else if pose.yaw > TURN_MAX_YAW_DEG
                    || pose.pitch.abs() > OFF_AXIS_MAX_SECONDARY_DEG
                {
                    Some(EnrollmentStepFeedback::PoseOutOfRange)
                } else {
                    None
                }
            }
            EnrollmentStep::TiltUp => {
                if pose.pitch > TILT_MAX_PITCH_DEG {
                    Some(EnrollmentStepFeedback::PromptTiltUp)
                } else if pose.pitch < TILT_MIN_PITCH_DEG
                    || pose.yaw.abs() > OFF_AXIS_MAX_SECONDARY_DEG
                {
                    Some(EnrollmentStepFeedback::PoseOutOfRange)
                } else {
                    None
                }
            }
            EnrollmentStep::Completed => Some(EnrollmentStepFeedback::AllStepsCompleted),
        }
    }

    /// Iterates over every accepted sample in step order.
    fn all_samples(&self) -> impl Iterator<Item = &Vec<f32>> {
        self.frontal_samples
            .iter()
            .chain(self.left_samples.iter())
            .chain(self.right_samples.iter())
            .chain(self.tilt_samples.iter())
    }

    /// Checks bounds, dimension consistency, finiteness and non-degenerate norm.
    fn is_valid_embedding(&self, embedding: &[f32]) -> bool {
        if embedding.is_empty() || embedding.len() > MAX_GUIDED_EMBEDDING_DIM {
            return false;
        }
        if let Some(first) = self.all_samples().next() {
            if first.len() != embedding.len() {
                return false;
            }
        }
        if !embedding.iter().all(|x| x.is_finite()) {
            return false;
        }
        l2_norm(embedding).is_some_and(|n| n > MIN_EMBEDDING_NORM)
    }

    /// Checks identity consistency against the frontal anchor samples.
    fn is_identity_consistent(&self, embedding: &[f32]) -> bool {
        let pairwise_ok = self.frontal_samples.iter().all(|anchor| {
            cosine(anchor, embedding).is_some_and(|c| c >= MIN_SAMPLE_CONSISTENCY_COSINE)
        });
        if !pairwise_ok {
            return false;
        }
        if self.current_step == EnrollmentStep::Frontal {
            return true;
        }
        match mean_direction(self.frontal_samples.iter()) {
            Some(anchor) => {
                cosine(&anchor, embedding).is_some_and(|c| c >= MIN_SAMPLE_CONSISTENCY_COSINE)
            }
            None => false,
        }
    }

    /// Fuses all multi-angle collected embedding vectors into a normalized composite unit vector.
    ///
    /// The composite is the mean direction of the L2-normalized samples. Every sample is
    /// re-validated (dimension, finiteness, identity consistency with the frontal anchor);
    /// any violation fails closed with an error.
    pub fn compute_composite_embedding(&self) -> Result<Vec<f32>, String> {
        if self.aborted {
            return Err("Enrollment session aborted after repeated spoof detections".to_string());
        }
        let dim = match self.all_samples().next() {
            Some(first) => first.len(),
            None => return Err("No samples collected to compute template".to_string()),
        };
        if dim == 0 || dim > MAX_GUIDED_EMBEDDING_DIM {
            return Err("Collected embedding dimension is out of bounds".to_string());
        }

        let anchor = mean_direction(self.frontal_samples.iter())
            .ok_or_else(|| "No valid frontal anchor sample collected".to_string())?;

        for sample in self.all_samples() {
            if sample.len() != dim {
                return Err("Dimension mismatch among collected embedding samples".to_string());
            }
            if !sample.iter().all(|x| x.is_finite()) {
                return Err("Non-finite value in collected embedding samples".to_string());
            }
            if !cosine(&anchor, sample).is_some_and(|c| c >= MIN_SAMPLE_CONSISTENCY_COSINE) {
                return Err("Collected samples are not identity-consistent".to_string());
            }
        }

        let composite = mean_direction(self.all_samples())
            .ok_or_else(|| "Composite embedding norm is zero".to_string())?;
        if !composite.iter().all(|x| x.is_finite()) {
            return Err("Composite embedding is not finite".to_string());
        }
        Ok(composite)
    }
}

/// Returns the L2 norm of `v`, or `None` when it is not finite.
fn l2_norm(v: &[f32]) -> Option<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    norm.is_finite().then_some(norm)
}

/// Cosine similarity of two equal-length vectors, or `None` when undefined.
fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() {
        return None;
    }
    let na = l2_norm(a)?;
    let nb = l2_norm(b)?;
    if na <= MIN_EMBEDDING_NORM || nb <= MIN_EMBEDDING_NORM {
        return None;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let c = dot / (na * nb);
    c.is_finite().then_some(c.clamp(-1.0, 1.0))
}

/// Mean direction (normalized sum of normalized vectors), or `None` when degenerate.
fn mean_direction<'a>(samples: impl Iterator<Item = &'a Vec<f32>>) -> Option<Vec<f32>> {
    let mut sum: Option<Vec<f32>> = None;
    for sample in samples {
        let norm = l2_norm(sample)?;
        if norm <= MIN_EMBEDDING_NORM {
            return None;
        }
        let acc = sum.get_or_insert_with(|| vec![0.0f32; sample.len()]);
        if acc.len() != sample.len() {
            return None;
        }
        for (a, x) in acc.iter_mut().zip(sample.iter()) {
            *a += *x / norm;
        }
    }
    let mut sum = sum?;
    let norm = l2_norm(&sum)?;
    if norm <= MIN_EMBEDDING_NORM {
        return None;
    }
    for x in &mut sum {
        *x /= norm;
    }
    Some(sum)
}

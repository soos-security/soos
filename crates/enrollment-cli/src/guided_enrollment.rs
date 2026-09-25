//! Guided multi-step biometric enrollment state machine and multi-sample template fusion.
//!
//! Guides the user through Frontal, Turn Left, Turn Right, and Tilt Up poses
//! while ensuring liveness and face stability. Fuses collected multi-angle embedding samples
//! into a single normalized composite vector for robust real-time PAM unlock performance.

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
}

impl GuidedEnrollmentSession {
    /// Creates a new guided enrollment session with target valid samples per pose step.
    pub fn new(target_samples_per_step: usize) -> Self {
        let capacity = target_samples_per_step.clamp(1, 20);
        Self {
            target_samples_per_step: capacity,
            current_step: EnrollmentStep::Frontal,
            frontal_samples: Vec::with_capacity(capacity),
            left_samples: Vec::with_capacity(capacity),
            right_samples: Vec::with_capacity(capacity),
            tilt_samples: Vec::with_capacity(capacity),
        }
    }

    /// Returns the currently active step.
    pub fn current_step(&self) -> EnrollmentStep {
        self.current_step
    }

    /// Returns overall completion progress as a percentage in [0.0, 100.0].
    pub fn progress_percent(&self) -> f32 {
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
    pub fn process_sample(
        &mut self,
        pose: &HeadPose,
        embedding: &[f32],
        is_live: bool,
        is_centered: bool,
    ) -> EnrollmentStepFeedback {
        if self.current_step == EnrollmentStep::Completed {
            return EnrollmentStepFeedback::AllStepsCompleted;
        }

        if !is_live {
            return EnrollmentStepFeedback::SpoofDetected;
        }

        if !is_centered {
            return EnrollmentStepFeedback::PromptCenterFace;
        }

        if embedding.is_empty() {
            return EnrollmentStepFeedback::PromptHoldStill;
        }

        match self.current_step {
            EnrollmentStep::Frontal => {
                if pose.roll.abs() > 10.0 {
                    return EnrollmentStepFeedback::PromptHoldStill;
                }
                if pose.yaw.abs() > 8.0 {
                    return EnrollmentStepFeedback::PromptCenterFace;
                }
                if pose.pitch.abs() > 10.0 {
                    return EnrollmentStepFeedback::PromptHoldStill;
                }

                self.frontal_samples.push(embedding.to_vec());
                let collected = self.frontal_samples.len();
                if collected >= self.target_samples_per_step {
                    self.current_step = EnrollmentStep::TurnLeft;
                    EnrollmentStepFeedback::StepCompleted {
                        next_step: EnrollmentStep::TurnLeft,
                    }
                } else {
                    EnrollmentStepFeedback::SampleAccepted {
                        step: EnrollmentStep::Frontal,
                        collected,
                        target: self.target_samples_per_step,
                    }
                }
            }

            EnrollmentStep::TurnLeft => {
                if pose.yaw > -10.0 {
                    return EnrollmentStepFeedback::PromptTurnLeft;
                }

                self.left_samples.push(embedding.to_vec());
                let collected = self.left_samples.len();
                if collected >= self.target_samples_per_step {
                    self.current_step = EnrollmentStep::TurnRight;
                    EnrollmentStepFeedback::StepCompleted {
                        next_step: EnrollmentStep::TurnRight,
                    }
                } else {
                    EnrollmentStepFeedback::SampleAccepted {
                        step: EnrollmentStep::TurnLeft,
                        collected,
                        target: self.target_samples_per_step,
                    }
                }
            }

            EnrollmentStep::TurnRight => {
                if pose.yaw < 10.0 {
                    return EnrollmentStepFeedback::PromptTurnRight;
                }

                self.right_samples.push(embedding.to_vec());
                let collected = self.right_samples.len();
                if collected >= self.target_samples_per_step {
                    self.current_step = EnrollmentStep::TiltUp;
                    EnrollmentStepFeedback::StepCompleted {
                        next_step: EnrollmentStep::TiltUp,
                    }
                } else {
                    EnrollmentStepFeedback::SampleAccepted {
                        step: EnrollmentStep::TurnRight,
                        collected,
                        target: self.target_samples_per_step,
                    }
                }
            }

            EnrollmentStep::TiltUp => {
                if pose.pitch > -5.0 {
                    return EnrollmentStepFeedback::PromptTiltUp;
                }

                self.tilt_samples.push(embedding.to_vec());
                let collected = self.tilt_samples.len();
                if collected >= self.target_samples_per_step {
                    self.current_step = EnrollmentStep::Completed;
                    EnrollmentStepFeedback::AllStepsCompleted
                } else {
                    EnrollmentStepFeedback::SampleAccepted {
                        step: EnrollmentStep::TiltUp,
                        collected,
                        target: self.target_samples_per_step,
                    }
                }
            }

            EnrollmentStep::Completed => EnrollmentStepFeedback::AllStepsCompleted,
        }
    }

    /// Fuses all multi-angle collected embedding vectors into a normalized composite unit vector.
    pub fn compute_composite_embedding(&self) -> Result<Vec<f32>, String> {
        let total_samples = self.frontal_samples.len()
            + self.left_samples.len()
            + self.right_samples.len()
            + self.tilt_samples.len();

        if total_samples == 0 {
            return Err("No samples collected to compute template".to_string());
        }

        let dim = self.frontal_samples.first().map(|v| v.len()).unwrap_or(512);

        let mut sum = vec![0.0f32; dim];

        let all_samples = self
            .frontal_samples
            .iter()
            .chain(self.left_samples.iter())
            .chain(self.right_samples.iter())
            .chain(self.tilt_samples.iter());

        for sample in all_samples {
            if sample.len() != dim {
                return Err("Dimension mismatch among collected embedding samples".to_string());
            }
            for (s_elem, sample_val) in sum.iter_mut().zip(sample.iter()) {
                *s_elem += *sample_val;
            }
        }

        // L2 normalize composite vector to unit length
        let norm = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm <= 1e-6 {
            return Err("Composite embedding norm is zero".to_string());
        }

        for x in &mut sum {
            *x /= norm;
        }

        Ok(sum)
    }
}

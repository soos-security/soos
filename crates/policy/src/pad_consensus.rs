//! Multi-frame Presentation Attack Detection (PAD) consensus aggregation.
//!
//! The daemon evaluates several camera frames per authentication request. Authorizing on
//! the first frame that passes PAD and matching gives an attacker one independent liveness
//! trial per evaluated frame (GitHub #147 / PAD-02). This module replaces that rule with a
//! zero-I/O aggregator:
//!
//! - `Allow` requires `k` **consecutive** passing frames (live above the PAD threshold and
//!   matching above the cosine threshold) inside a bounded window of the last `n` frames.
//! - Any frame classified as a spoof anywhere in the request **vetoes** `Allow` for the
//!   whole request (sticky, fail closed).
//! - Frames without a face, with several faces, or live-but-not-matching reset the
//!   consecutive run without vetoing.
//!
//! The aggregator is clockless and holds at most [`MAX_PAD_CONSENSUS_WINDOW`] classifications.

use std::collections::VecDeque;

use crate::error::PolicyError;
use crate::threshold::ThresholdConfig;
use soos_protocol::{ReasonClass, Verdict};

/// Number of most recent evaluated frames retained by the consensus window (`n`).
pub const DEFAULT_PAD_CONSENSUS_WINDOW: usize = 5;

/// Number of consecutive passing frames required to authorize (`k`).
pub const DEFAULT_PAD_CONSENSUS_REQUIRED: usize = 3;

/// Upper bound on the consensus window, bounding aggregator memory.
pub const MAX_PAD_CONSENSUS_WINDOW: usize = 32;

/// Consensus window parameters: `1 <= required <= window <= MAX_PAD_CONSENSUS_WINDOW`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadConsensusConfig {
    window: usize,
    required: usize,
}

impl PadConsensusConfig {
    /// Builds a validated consensus configuration.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::InvalidConsensus`] if `required == 0`, `required > window`,
    /// or `window > MAX_PAD_CONSENSUS_WINDOW`.
    pub fn new(window: usize, required: usize) -> Result<Self, PolicyError> {
        if required == 0 {
            return Err(PolicyError::InvalidConsensus {
                window,
                required,
                reason: "required consecutive frame count must be at least 1",
            });
        }
        if window > MAX_PAD_CONSENSUS_WINDOW {
            return Err(PolicyError::InvalidConsensus {
                window,
                required,
                reason: "window exceeds MAX_PAD_CONSENSUS_WINDOW",
            });
        }
        if required > window {
            return Err(PolicyError::InvalidConsensus {
                window,
                required,
                reason: "required consecutive frame count exceeds the window",
            });
        }
        Ok(Self { window, required })
    }

    /// Window size `n` (number of retained frame classifications).
    #[must_use]
    pub const fn window(&self) -> usize {
        self.window
    }

    /// Required consecutive passing frames `k`.
    #[must_use]
    pub const fn required(&self) -> usize {
        self.required
    }
}

impl Default for PadConsensusConfig {
    fn default() -> Self {
        Self {
            window: DEFAULT_PAD_CONSENSUS_WINDOW,
            required: DEFAULT_PAD_CONSENSUS_REQUIRED,
        }
    }
}

/// One evaluated camera frame as observed by the daemon.
///
/// `pad_live`, `pad_score` and `match_score` are only meaningful when `face_count == 1`;
/// they are ignored otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameEvaluation {
    /// Number of detected faces.
    pub face_count: u8,
    /// PAD classifier live decision.
    pub pad_live: bool,
    /// PAD liveness score in `[0.0, 1.0]`; non-finite values are treated as spoof.
    pub pad_score: f32,
    /// Cosine similarity against the enrolled template; non-finite values never match.
    pub match_score: f32,
}

impl FrameEvaluation {
    /// Constructs a frame evaluation.
    #[must_use]
    pub const fn new(face_count: u8, pad_live: bool, pad_score: f32, match_score: f32) -> Self {
        Self {
            face_count,
            pad_live,
            pad_score,
            match_score,
        }
    }

    /// Frame without any detected face.
    #[must_use]
    pub const fn no_face() -> Self {
        Self::new(0, false, 0.0, 0.0)
    }

    /// Frame with several detected faces.
    #[must_use]
    pub const fn multiple_faces(count: u8) -> Self {
        Self::new(count, false, 0.0, 0.0)
    }

    /// Single face classified as a presentation attack.
    #[must_use]
    pub const fn spoof(pad_score: f32) -> Self {
        Self::new(1, false, pad_score, 0.0)
    }

    /// Single face classified live with the given liveness and match scores.
    #[must_use]
    pub const fn live(pad_score: f32, match_score: f32) -> Self {
        Self::new(1, true, pad_score, match_score)
    }
}

/// Classification of one evaluated frame against the configured thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameClass {
    /// Single face, live at or above the PAD threshold, matching at or above the cosine threshold.
    Passing,
    /// Single face classified as a presentation attack, or with an invalid liveness score.
    Spoof,
    /// No face detected.
    NoFace,
    /// Several faces detected.
    MultipleFaces,
    /// Live single face below the match threshold (or non-finite match score).
    NoMatch,
}

/// Aggregate decision over the frames observed so far in one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusDecision {
    /// `k` consecutive passing frames observed and no spoof frame in the request.
    Allow,
    /// At least one spoof-classified frame was observed in this request (sticky).
    SpoofVetoed,
    /// Consensus not reached yet; carries the last frame classification, if any.
    Pending(Option<FrameClass>),
}

impl ConsensusDecision {
    /// Maps the aggregate decision to the protocol verdict and reason class.
    ///
    /// Every variant other than [`ConsensusDecision::Allow`] maps to a non-authorizing
    /// verdict (`PAM_IGNORE`). An unfinished consensus maps to `Unavailable`/`Timeout`
    /// because the daemon ran out of budget before collecting enough evidence.
    #[must_use]
    pub const fn verdict(self) -> (Verdict, ReasonClass) {
        match self {
            Self::Allow => (Verdict::Allow, ReasonClass::FaceMatch),
            Self::SpoofVetoed | Self::Pending(Some(FrameClass::Spoof)) => {
                (Verdict::Deny, ReasonClass::PadFailed)
            }
            Self::Pending(Some(FrameClass::NoFace)) => (Verdict::Deny, ReasonClass::NoFace),
            Self::Pending(Some(FrameClass::MultipleFaces)) => {
                (Verdict::Deny, ReasonClass::MultipleFaces)
            }
            Self::Pending(Some(FrameClass::NoMatch)) => {
                (Verdict::Deny, ReasonClass::ScoreBelowThreshold)
            }
            Self::Pending(None | Some(FrameClass::Passing)) => {
                (Verdict::Unavailable, ReasonClass::Timeout)
            }
        }
    }
}

/// Sliding-window PAD consensus aggregator for one authentication request.
///
/// Create one aggregator per request; there is deliberately no reset method so a spoof
/// veto cannot be cleared within a request.
#[derive(Debug, Clone)]
pub struct PadAggregator {
    config: PadConsensusConfig,
    thresholds: ThresholdConfig,
    history: VecDeque<FrameClass>,
    spoof_seen: bool,
    frames_evaluated: u32,
    live_frames: u32,
    spoof_frames: u32,
}

impl PadAggregator {
    /// Creates an aggregator with explicit window parameters.
    #[must_use]
    pub fn new(config: PadConsensusConfig, thresholds: ThresholdConfig) -> Self {
        Self {
            config,
            thresholds,
            history: VecDeque::with_capacity(config.window()),
            spoof_seen: false,
            frames_evaluated: 0,
            live_frames: 0,
            spoof_frames: 0,
        }
    }

    /// Creates an aggregator with the default `k = 3` within `n = 5` parameters.
    #[must_use]
    pub fn with_defaults(thresholds: ThresholdConfig) -> Self {
        Self::new(PadConsensusConfig::default(), thresholds)
    }

    /// Consensus parameters in use.
    #[must_use]
    pub const fn config(&self) -> &PadConsensusConfig {
        &self.config
    }

    /// Classifies a frame without recording it.
    #[must_use]
    pub fn classify(&self, frame: &FrameEvaluation) -> FrameClass {
        if frame.face_count == 0 {
            return FrameClass::NoFace;
        }
        if frame.face_count > 1 {
            return FrameClass::MultipleFaces;
        }
        if !frame.pad_live
            || !frame.pad_score.is_finite()
            || frame.pad_score < self.thresholds.pad_threshold()
        {
            return FrameClass::Spoof;
        }
        if !frame.match_score.is_finite() || frame.match_score < self.thresholds.match_threshold() {
            return FrameClass::NoMatch;
        }
        FrameClass::Passing
    }

    /// Classifies and records a frame, returning its classification.
    pub fn record(&mut self, frame: &FrameEvaluation) -> FrameClass {
        let class = self.classify(frame);
        self.frames_evaluated = self.frames_evaluated.saturating_add(1);
        match class {
            FrameClass::Spoof => {
                self.spoof_seen = true;
                self.spoof_frames = self.spoof_frames.saturating_add(1);
            }
            FrameClass::Passing => {
                self.live_frames = self.live_frames.saturating_add(1);
            }
            FrameClass::NoFace | FrameClass::MultipleFaces | FrameClass::NoMatch => {}
        }
        while self.history.len() >= self.config.window() {
            self.history.pop_front();
        }
        self.history.push_back(class);
        class
    }

    /// Aggregate decision over the frames recorded so far.
    #[must_use]
    pub fn decision(&self) -> ConsensusDecision {
        if self.spoof_seen {
            return ConsensusDecision::SpoofVetoed;
        }
        if self.consecutive_passing() >= self.config.required() {
            return ConsensusDecision::Allow;
        }
        ConsensusDecision::Pending(self.history.back().copied())
    }

    /// Length of the trailing run of passing frames inside the window.
    #[must_use]
    pub fn consecutive_passing(&self) -> usize {
        self.history
            .iter()
            .rev()
            .take_while(|class| **class == FrameClass::Passing)
            .count()
    }

    /// Whether a spoof-classified frame was observed in this request.
    #[must_use]
    pub const fn spoof_seen(&self) -> bool {
        self.spoof_seen
    }

    /// Total number of frames recorded (saturating).
    #[must_use]
    pub const fn frames_evaluated(&self) -> u32 {
        self.frames_evaluated
    }

    /// Number of passing (live and matching) frames recorded (saturating).
    #[must_use]
    pub const fn live_frames(&self) -> u32 {
        self.live_frames
    }

    /// Number of spoof-classified frames recorded (saturating).
    #[must_use]
    pub const fn spoof_frames(&self) -> u32 {
        self.spoof_frames
    }

    /// Retained classification history, oldest first (at most `window` entries).
    pub fn history(&self) -> impl Iterator<Item = FrameClass> + '_ {
        self.history.iter().copied()
    }
}

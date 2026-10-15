//! Strobe classification of IR frame streams (GitHub #378, PR 1 of #353).
//!
//! Many Windows Hello modules light only every other frame, some keep the emitter steady, some
//! only light it on command, and an IR node without a working emitter is dark. The capture side
//! classifies the stream from the mean luma of its frames:
//!
//! | Class | Rule (first match wins) |
//! |---|---|
//! | [`StrobeClass::Controllable`] | both states commanded (≥ 2 each), pattern not periodic, `min(on) − max(off) >= STROBE_MIN_LUMA_DELTA` |
//! | [`StrobeClass::NativeAlternating`] | each sequence parity ≥ 2 samples, one parity's `min − other's max >= DELTA` |
//! | [`StrobeClass::Absent`] | every sample `< STROBE_DARK_LUMA` |
//! | [`StrobeClass::Steady`] | `max − min < DELTA` |
//!
//! Anything else fails closed ([`StrobeError`]). A commanded pattern that is a function of
//! `sequence % p` for `p` in `2..=4` is refused as [`StrobeError::AmbiguousPattern`]: a
//! natively strobing module would satisfy it too. The thresholds are provisional and
//! uncalibrated (an IR hardware row of the verification matrix). Only mean luma values are kept:
//! no pixel is copied.

use crate::ir_emitter::EmitterState;
use soos_frame::{Frame, PixelFormat, SensorType};

/// Fewest samples a classification accepts.
pub const STROBE_MIN_SAMPLES: usize = 8;
/// Most samples a classification accepts (also the monitor's memory bound).
pub const STROBE_MAX_SAMPLES: usize = 64;
/// Fewest samples per group (on/off or even/odd).
pub const STROBE_MIN_PER_GROUP: usize = 2;
/// Smallest gap between the lit and the unlit group, in 8-bit luma levels.
pub const STROBE_MIN_LUMA_DELTA: f32 = 12.0;
/// A stream whose every sample is darker than this has no illumination at all.
pub const STROBE_DARK_LUMA: f32 = 20.0;
/// Longest command period checked by the ambiguity rule.
const MAX_AMBIGUOUS_PERIOD: u64 = 4;
/// Phase slots of the longest checked period.
const PERIOD_SLOTS: usize = 4;

/// Mean luma of one frame and the emitter state commanded for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LumaSample {
    /// Frame sequence number.
    pub sequence: u64,
    /// Mean 8-bit luma (`0.0..=255.0`).
    pub mean_luma: f32,
    /// Emitter state commanded for this frame (`None`: no command).
    pub commanded: Option<EmitterState>,
}

impl LumaSample {
    /// Sample of an Infrared frame (`None` for any other sensor or an unreadable buffer).
    #[must_use]
    pub fn from_ir_frame(frame: &Frame, commanded: Option<EmitterState>) -> Option<Self> {
        if frame.sensor_type != SensorType::Infrared {
            return None;
        }
        Some(Self {
            sequence: frame.sequence,
            mean_luma: frame_mean_luma(frame)?,
            commanded,
        })
    }
}

/// Mean of `bytes` selected by `step` from `start`, as `f32`.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "sums of at most usize::MAX bytes; the mean is within 0..=255"
)]
fn mean_of(bytes: impl Iterator<Item = u8>) -> Option<f32> {
    let (sum, count) = bytes.fold((0u64, 0u64), |(sum, count), b| {
        (sum.saturating_add(u64::from(b)), count.saturating_add(1))
    });
    if count == 0 {
        return None;
    }
    Some((sum as f64 / count as f64) as f32)
}

/// Mean 8-bit luma of a frame: Grey (all bytes), Yuyv (even bytes), Nv12 (Y plane), Rgb24 (all
/// bytes). `None` for Mjpeg, an empty frame, or a buffer whose length differs from the
/// expected size of its geometry.
#[must_use]
pub fn frame_mean_luma(frame: &Frame) -> Option<f32> {
    let expected = frame
        .format
        .expected_buffer_size(frame.width, frame.height)?;
    if expected == 0 || frame.data.len() != expected {
        return None;
    }
    match frame.format {
        PixelFormat::Grey | PixelFormat::Rgb24 => mean_of(frame.data.iter().copied()),
        PixelFormat::Yuyv => mean_of(frame.data.iter().step_by(2).copied()),
        PixelFormat::Nv12 => {
            let pixels = (frame.width as usize).saturating_mul(frame.height as usize);
            mean_of(frame.data.get(..pixels)?.iter().copied())
        }
        PixelFormat::Mjpeg => None,
    }
}

/// The class of an IR stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrobeClass {
    /// Lit and stable; an emitter, if any, cannot be shown to respond.
    Steady,
    /// Frames alternate lit/unlit by sequence parity on their own.
    NativeAlternating,
    /// The scene follows the commanded emitter states.
    Controllable,
    /// No illumination at all.
    Absent,
}

/// Why a sequence was not classified (fail closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StrobeError {
    /// Fewer than [`STROBE_MIN_SAMPLES`].
    #[error("{got} samples, at least {min} needed")]
    TooFewSamples {
        /// Samples given.
        got: usize,
        /// The bound.
        min: usize,
    },
    /// More than [`STROBE_MAX_SAMPLES`].
    #[error("{got} samples, at most {max} accepted")]
    TooManySamples {
        /// Samples given.
        got: usize,
        /// The bound.
        max: usize,
    },
    /// NaN, infinite, negative or above 255.
    #[error("invalid mean luma")]
    InvalidLuma,
    /// Sequence numbers not strictly increasing.
    #[error("sequence numbers are not strictly increasing")]
    NonMonotonicSequence,
    /// The commanded pattern is periodic (p = 2..=4), so it proves nothing.
    #[error("the commanded pattern is periodic and proves nothing")]
    AmbiguousPattern,
    /// No class fits.
    #[error("irregular illumination")]
    Irregular,
}

/// Minimum and maximum luma of an iterator (`None` when empty).
fn min_max(values: impl Iterator<Item = f32>) -> Option<(f32, f32)> {
    values.fold(None, |acc, v| match acc {
        None => Some((v, v)),
        Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
    })
}

/// `true` when `lit` is separated from `dark` by at least the gap, each with enough samples.
fn separated(lit: &[f32], dark: &[f32]) -> bool {
    if lit.len() < STROBE_MIN_PER_GROUP || dark.len() < STROBE_MIN_PER_GROUP {
        return false;
    }
    match (min_max(lit.iter().copied()), min_max(dark.iter().copied())) {
        (Some((lit_min, _)), Some((_, dark_max))) => lit_min - dark_max >= STROBE_MIN_LUMA_DELTA,
        _ => false,
    }
}

/// `true` when the commands of the commanded samples are a function of `sequence % p` for some
/// `p` in `2..=MAX_AMBIGUOUS_PERIOD`.
fn is_periodic_pattern(commanded: &[(u64, EmitterState)]) -> bool {
    (2..=MAX_AMBIGUOUS_PERIOD).any(|p| {
        let mut phase: [Option<EmitterState>; PERIOD_SLOTS] = [None; PERIOD_SLOTS];
        commanded.iter().all(|(seq, state)| {
            let Some(slot) = seq
                .checked_rem(p)
                .and_then(|r| usize::try_from(r).ok())
                .and_then(|i| phase.get_mut(i))
            else {
                return false;
            };
            match slot {
                None => {
                    *slot = Some(*state);
                    true
                }
                Some(previous) => previous == state,
            }
        })
    })
}

/// Classifies a sequence of samples (see the module documentation for the rules).
///
/// # Errors
///
/// A [`StrobeError`] for invalid input, a periodic command pattern, or irregular light.
pub fn classify_strobe(samples: &[LumaSample]) -> Result<StrobeClass, StrobeError> {
    if samples.len() < STROBE_MIN_SAMPLES {
        return Err(StrobeError::TooFewSamples {
            got: samples.len(),
            min: STROBE_MIN_SAMPLES,
        });
    }
    if samples.len() > STROBE_MAX_SAMPLES {
        return Err(StrobeError::TooManySamples {
            got: samples.len(),
            max: STROBE_MAX_SAMPLES,
        });
    }
    if samples
        .iter()
        .any(|s| !s.mean_luma.is_finite() || !(0.0..=255.0).contains(&s.mean_luma))
    {
        return Err(StrobeError::InvalidLuma);
    }
    if samples.windows(2).any(|w| match w {
        [a, b] => b.sequence <= a.sequence,
        _ => false,
    }) {
        return Err(StrobeError::NonMonotonicSequence);
    }

    let commanded: Vec<(u64, EmitterState)> = samples
        .iter()
        .filter_map(|s| s.commanded.map(|c| (s.sequence, c)))
        .collect();
    let on: Vec<f32> = samples
        .iter()
        .filter(|s| s.commanded == Some(EmitterState::On))
        .map(|s| s.mean_luma)
        .collect();
    let off: Vec<f32> = samples
        .iter()
        .filter(|s| s.commanded == Some(EmitterState::Off))
        .map(|s| s.mean_luma)
        .collect();
    if on.len() >= STROBE_MIN_PER_GROUP && off.len() >= STROBE_MIN_PER_GROUP {
        if is_periodic_pattern(&commanded) {
            return Err(StrobeError::AmbiguousPattern);
        }
        if separated(&on, &off) {
            return Ok(StrobeClass::Controllable);
        }
    }

    let even: Vec<f32> = samples
        .iter()
        .filter(|s| s.sequence % 2 == 0)
        .map(|s| s.mean_luma)
        .collect();
    let odd: Vec<f32> = samples
        .iter()
        .filter(|s| s.sequence % 2 == 1)
        .map(|s| s.mean_luma)
        .collect();
    if separated(&even, &odd) || separated(&odd, &even) {
        return Ok(StrobeClass::NativeAlternating);
    }

    let (lo, hi) = min_max(samples.iter().map(|s| s.mean_luma)).ok_or(StrobeError::Irregular)?;
    if hi < STROBE_DARK_LUMA {
        return Ok(StrobeClass::Absent);
    }
    if hi - lo < STROBE_MIN_LUMA_DELTA {
        return Ok(StrobeClass::Steady);
    }
    Err(StrobeError::Irregular)
}

/// State of a [`StrobeMonitor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrobeStatus {
    /// The first block is not complete yet.
    Learning,
    /// The first block failed classification (sticky).
    Unclassified(StrobeError),
    /// Every block so far had this class.
    Stable(StrobeClass),
    /// A later block had another class or failed (sticky).
    Changed {
        /// The established class.
        from: StrobeClass,
        /// The new class, `None` when the block failed classification.
        to: Option<StrobeClass>,
    },
}

impl StrobeStatus {
    /// `true` for everything but [`StrobeStatus::Stable`]: only a stable stream may ever
    /// contribute to an `Allow`.
    #[must_use]
    pub const fn vetoes_allow(self) -> bool {
        !matches!(self, Self::Stable(_))
    }
}

/// Classifies consecutive, non-overlapping blocks of samples and remembers any change.
#[derive(Debug, Clone)]
pub struct StrobeMonitor {
    block_len: usize,
    buffer: Vec<LumaSample>,
    status: StrobeStatus,
}

impl StrobeMonitor {
    /// A monitor with blocks of `block_len` samples, clamped to
    /// `STROBE_MIN_SAMPLES..=STROBE_MAX_SAMPLES`.
    #[must_use]
    pub fn new(block_len: usize) -> Self {
        let block_len = block_len.clamp(STROBE_MIN_SAMPLES, STROBE_MAX_SAMPLES);
        Self {
            block_len,
            buffer: Vec::with_capacity(block_len),
            status: StrobeStatus::Learning,
        }
    }

    /// Adds one sample; classifies and clears the block when it is full.
    pub fn push(&mut self, sample: LumaSample) -> StrobeStatus {
        if matches!(
            self.status,
            StrobeStatus::Unclassified(_) | StrobeStatus::Changed { .. }
        ) {
            return self.status;
        }
        self.buffer.push(sample);
        if self.buffer.len() < self.block_len {
            return self.status;
        }
        let result = classify_strobe(&self.buffer);
        self.buffer.clear();
        self.status = match (self.status, result) {
            (StrobeStatus::Learning, Ok(class)) => StrobeStatus::Stable(class),
            (StrobeStatus::Learning, Err(error)) => StrobeStatus::Unclassified(error),
            (StrobeStatus::Stable(from), Ok(class)) if class == from => StrobeStatus::Stable(from),
            (StrobeStatus::Stable(from), Ok(class)) => StrobeStatus::Changed {
                from,
                to: Some(class),
            },
            (StrobeStatus::Stable(from), Err(_)) => StrobeStatus::Changed { from, to: None },
            (sticky, _) => sticky,
        };
        self.status
    }

    /// The current status.
    #[must_use]
    pub fn status(&self) -> StrobeStatus {
        self.status
    }
}

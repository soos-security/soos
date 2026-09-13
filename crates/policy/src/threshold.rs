//! Cosine similarity and Presentation Attack Detection (PAD) threshold configuration.

use crate::error::PolicyError;

/// Sane biometric thresholds derived from academic literature and NIST benchmarks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThresholdConfig {
    pub(crate) match_threshold: f32,
    pub(crate) pad_threshold: f32,
}

impl ThresholdConfig {
    /// Default cosine similarity threshold for `MobileFaceNet` feature embeddings.
    ///
    /// Sourced from Chen et al. (2018), *`MobileFaceNets`: Efficient CNNs for
    /// Accurate Real-Time Face Verification on Mobile Devices* (arXiv:1804.07573).
    /// Calibrated for False Accept Rate (FAR) <= 0.1% on standard LFW evaluation.
    pub const DEFAULT_MATCH_THRESHOLD: f32 = 0.70;

    /// Default presentation attack detection (PAD) confidence threshold.
    ///
    /// Sourced from NIST SP 800-63B and NIST IR 8491 guidance for biometric
    /// presentation attack mitigation on local sensor inputs.
    pub const DEFAULT_PAD_THRESHOLD: f32 = 0.85;

    /// Creates a new [`ThresholdConfig`] directly without validation (internal/testing).
    #[must_use]
    pub const fn new_raw(match_threshold: f32, pad_threshold: f32) -> Self {
        Self {
            match_threshold,
            pad_threshold,
        }
    }

    /// Returns a new builder for constructing a custom [`ThresholdConfig`].
    #[must_use]
    pub fn builder() -> ThresholdConfigBuilder {
        ThresholdConfigBuilder::default()
    }

    /// Access the configured cosine match threshold.
    #[must_use]
    pub const fn match_threshold(&self) -> f32 {
        self.match_threshold
    }

    /// Access the configured PAD liveness threshold.
    #[must_use]
    pub const fn pad_threshold(&self) -> f32 {
        self.pad_threshold
    }
}

impl Default for ThresholdConfig {
    fn default() -> Self {
        Self {
            match_threshold: Self::DEFAULT_MATCH_THRESHOLD,
            pad_threshold: Self::DEFAULT_PAD_THRESHOLD,
        }
    }
}

/// Builder pattern for [`ThresholdConfig`] with invariant validation.
#[derive(Debug, Clone, Copy)]
pub struct ThresholdConfigBuilder {
    pub(crate) match_threshold: f32,
    pub(crate) pad_threshold: f32,
}

impl Default for ThresholdConfigBuilder {
    fn default() -> Self {
        Self {
            match_threshold: ThresholdConfig::DEFAULT_MATCH_THRESHOLD,
            pad_threshold: ThresholdConfig::DEFAULT_PAD_THRESHOLD,
        }
    }
}

impl ThresholdConfigBuilder {
    /// Create a new builder with default thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the cosine similarity match threshold.
    #[must_use]
    pub fn match_threshold(mut self, threshold: f32) -> Self {
        self.match_threshold = threshold;
        self
    }

    /// Set the Presentation Attack Detection (PAD) confidence threshold.
    #[must_use]
    pub fn pad_threshold(mut self, threshold: f32) -> Self {
        self.pad_threshold = threshold;
        self
    }

    /// Validates thresholds and builds a [`ThresholdConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::InvalidThreshold`] if either threshold is NaN,
    /// infinite, or outside the closed interval `[0.0, 1.0]`.
    pub fn build(self) -> Result<ThresholdConfig, PolicyError> {
        validate_threshold("match_threshold", self.match_threshold)?;
        validate_threshold("pad_threshold", self.pad_threshold)?;
        Ok(ThresholdConfig {
            match_threshold: self.match_threshold,
            pad_threshold: self.pad_threshold,
        })
    }
}

fn validate_threshold(name: &'static str, val: f32) -> Result<(), PolicyError> {
    if val.is_nan() {
        return Err(PolicyError::invalid_threshold(
            name,
            val,
            "threshold must not be NaN",
        ));
    }
    if val.is_infinite() {
        return Err(PolicyError::invalid_threshold(
            name,
            val,
            "threshold must not be infinite",
        ));
    }
    if !(0.0..=1.0).contains(&val) {
        return Err(PolicyError::invalid_threshold(
            name,
            val,
            "threshold must be between 0.0 and 1.0 inclusive",
        ));
    }
    Ok(())
}

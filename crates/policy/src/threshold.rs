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

    /// Security floor for the cosine match threshold of operator-supplied configuration.
    ///
    /// Below this value unrelated faces are routinely accepted; configuration loaded from
    /// `daemon.toml` below this floor is refused at start-up (GitHub #170, PAD-04).
    pub const MIN_MATCH_THRESHOLD: f32 = 0.40;

    /// Security floor for the PAD liveness threshold of operator-supplied configuration.
    ///
    /// `pad_threshold = 0.0` (or any negative value) classifies every frame as live and
    /// silently disables anti-spoofing; configuration loaded from `daemon.toml` below this
    /// floor is refused at start-up (GitHub #170, PAD-04).
    pub const MIN_PAD_THRESHOLD: f32 = 0.50;

    /// Creates a new [`ThresholdConfig`] directly without validation (internal/testing).
    ///
    /// Production code outside `soos-policy` must never call this (repository invariant
    /// `test_thresholds_never_built_unvalidated_outside_policy`); operator configuration
    /// goes through [`ThresholdConfigBuilder::build_with_security_floor`].
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

    /// Validates thresholds against the domain `[0.0, 1.0]` **and** the security floors
    /// [`ThresholdConfig::MIN_MATCH_THRESHOLD`] / [`ThresholdConfig::MIN_PAD_THRESHOLD`].
    ///
    /// This is the constructor required for operator-supplied configuration
    /// (`[pipeline.thresholds]` in `daemon.toml`), GitHub #170 (PAD-04).
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::InvalidThreshold`] naming the offending setting if either
    /// threshold is NaN, infinite, outside `[0.0, 1.0]`, or below its security floor.
    pub fn build_with_security_floor(self) -> Result<ThresholdConfig, PolicyError> {
        let config = self.build()?;
        if config.match_threshold < ThresholdConfig::MIN_MATCH_THRESHOLD {
            return Err(PolicyError::invalid_threshold(
                "match_threshold",
                config.match_threshold,
                "threshold is below the security floor of 0.4 (would accept unrelated faces)",
            ));
        }
        if config.pad_threshold < ThresholdConfig::MIN_PAD_THRESHOLD {
            return Err(PolicyError::invalid_threshold(
                "pad_threshold",
                config.pad_threshold,
                "threshold is below the security floor of 0.5 (would weaken or disable anti-spoofing)",
            ));
        }
        Ok(config)
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

//! Per-UID sliding window rate limiter (Zero I/O, no system clock).

use crate::error::PolicyError;
use std::collections::{BTreeMap, VecDeque};

/// Configuration for per-UID rate limiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// Maximum number of allowed attempts within the sliding window.
    pub max_attempts: u32,
    /// Sliding window duration in nanoseconds.
    pub window_duration_ns: u64,
}

impl RateLimitConfig {
    /// Default maximum attempts: 5 attempts.
    pub const DEFAULT_MAX_ATTEMPTS: u32 = 5;

    /// Default window duration: 60 seconds (60,000,000,000 ns).
    pub const DEFAULT_WINDOW_DURATION_NS: u64 = 60_000_000_000;

    /// Construct a new [`RateLimitConfig`].
    #[must_use]
    pub const fn new(max_attempts: u32, window_duration_ns: u64) -> Self {
        Self {
            max_attempts,
            window_duration_ns,
        }
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
            window_duration_ns: Self::DEFAULT_WINDOW_DURATION_NS,
        }
    }
}

/// Sliding-window rate limiter per user identity (UID).
///
/// Operates without any system clock or filesystem I/O: all evaluation methods
/// require a monotonic timestamp `now_monotonic_ns` passed by the caller.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    config: RateLimitConfig,
    history: BTreeMap<u32, VecDeque<u64>>,
}

impl RateLimiter {
    /// Construct a new rate limiter with the specified configuration.
    #[must_use]
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            history: BTreeMap::new(),
        }
    }

    /// Access the current configuration.
    #[must_use]
    pub const fn config(&self) -> &RateLimitConfig {
        &self.config
    }

    /// Check whether a request from `uid` is permitted and record the attempt if allowed.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::RateLimitExceeded`] if the UID has reached the maximum allowed attempts.
    pub fn check_and_record(&mut self, uid: u32, now_monotonic_ns: u64) -> Result<(), PolicyError> {
        if self.config.max_attempts == 0 {
            return Err(PolicyError::RateLimitExceeded {
                uid,
                retry_after_ns: self.config.window_duration_ns,
            });
        }

        let cutoff = now_monotonic_ns.saturating_sub(self.config.window_duration_ns);
        let attempts = self.history.entry(uid).or_default();

        // Evict timestamps older than or equal to cutoff
        while let Some(&front) = attempts.front() {
            if front <= cutoff {
                attempts.pop_front();
            } else {
                break;
            }
        }

        let max_usize = usize::try_from(self.config.max_attempts).unwrap_or(usize::MAX);
        if attempts.len() >= max_usize {
            let retry_after_ns =
                attempts
                    .front()
                    .map_or(self.config.window_duration_ns, |&oldest| {
                        oldest
                            .saturating_add(self.config.window_duration_ns)
                            .saturating_sub(now_monotonic_ns)
                    });
            return Err(PolicyError::RateLimitExceeded {
                uid,
                retry_after_ns,
            });
        }

        attempts.push_back(now_monotonic_ns);
        Ok(())
    }

    /// Check rate limit without mutating state.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::RateLimitExceeded`] if currently blocked.
    pub fn check_only(&self, uid: u32, now_monotonic_ns: u64) -> Result<(), PolicyError> {
        self.check_allowed(uid, now_monotonic_ns)
    }

    /// Check whether a request from `uid` is allowed without mutating state.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::RateLimitExceeded`] if currently blocked.
    pub fn check_allowed(&self, uid: u32, now_monotonic_ns: u64) -> Result<(), PolicyError> {
        if self.config.max_attempts == 0 {
            return Err(PolicyError::RateLimitExceeded {
                uid,
                retry_after_ns: self.config.window_duration_ns,
            });
        }

        let Some(attempts) = self.history.get(&uid) else {
            return Ok(());
        };

        let cutoff = now_monotonic_ns.saturating_sub(self.config.window_duration_ns);
        let active_count = attempts.iter().filter(|&&ts| ts > cutoff).count();
        let max_usize = usize::try_from(self.config.max_attempts).unwrap_or(usize::MAX);

        if active_count >= max_usize {
            let oldest_active = attempts.iter().copied().find(|&ts| ts > cutoff);
            let retry_after_ns = oldest_active.map_or(self.config.window_duration_ns, |oldest| {
                oldest
                    .saturating_add(self.config.window_duration_ns)
                    .saturating_sub(now_monotonic_ns)
            });
            return Err(PolicyError::RateLimitExceeded {
                uid,
                retry_after_ns,
            });
        }

        Ok(())
    }

    /// Reset recorded attempts for a specific UID.
    pub fn reset_uid(&mut self, uid: u32) {
        self.history.remove(&uid);
    }

    /// Prune stale entries across all UIDs to maintain memory bounds.
    pub fn prune_stale(&mut self, now_monotonic_ns: u64) {
        let cutoff = now_monotonic_ns.saturating_sub(self.config.window_duration_ns);
        self.history.retain(|_, attempts| {
            while let Some(&front) = attempts.front() {
                if front <= cutoff {
                    attempts.pop_front();
                } else {
                    break;
                }
            }
            !attempts.is_empty()
        });
    }

    /// Remaining allowed attempts in the current window for `uid`.
    #[must_use]
    pub fn remaining_attempts(&self, uid: u32, now_monotonic_ns: u64) -> u32 {
        if self.config.max_attempts == 0 {
            return 0;
        }

        let Some(attempts) = self.history.get(&uid) else {
            return self.config.max_attempts;
        };

        let cutoff = now_monotonic_ns.saturating_sub(self.config.window_duration_ns);
        let active_count = attempts.iter().filter(|&&ts| ts > cutoff).count();
        let active_u32 = u32::try_from(active_count).unwrap_or(u32::MAX);

        self.config.max_attempts.saturating_sub(active_u32)
    }
}

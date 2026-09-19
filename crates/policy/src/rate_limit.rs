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
    /// Maximum number of unique UIDs tracked simultaneously in memory.
    pub max_tracked_uids: usize,
}

impl RateLimitConfig {
    /// Default maximum attempts: 5 attempts.
    pub const DEFAULT_MAX_ATTEMPTS: u32 = 5;

    /// Default window duration: 60 seconds (60,000,000,000 ns).
    pub const DEFAULT_WINDOW_DURATION_NS: u64 = 60_000_000_000;

    /// Default maximum tracked UIDs: 1024 unique UIDs.
    pub const DEFAULT_MAX_TRACKED_UIDS: usize = 1024;

    /// Construct a new [`RateLimitConfig`] with default capacity.
    #[must_use]
    pub const fn new(max_attempts: u32, window_duration_ns: u64) -> Self {
        Self {
            max_attempts,
            window_duration_ns,
            max_tracked_uids: Self::DEFAULT_MAX_TRACKED_UIDS,
        }
    }

    /// Construct a new [`RateLimitConfig`] with explicit capacity.
    #[must_use]
    pub const fn new_with_capacity(
        max_attempts: u32,
        window_duration_ns: u64,
        max_tracked_uids: usize,
    ) -> Self {
        Self {
            max_attempts,
            window_duration_ns,
            max_tracked_uids,
        }
    }

    /// Set the maximum number of tracked UIDs.
    #[must_use]
    pub const fn with_max_tracked_uids(mut self, max_tracked_uids: usize) -> Self {
        self.max_tracked_uids = max_tracked_uids;
        self
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
            window_duration_ns: Self::DEFAULT_WINDOW_DURATION_NS,
            max_tracked_uids: Self::DEFAULT_MAX_TRACKED_UIDS,
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

    /// Number of distinct UIDs currently tracked in memory.
    #[must_use]
    pub fn tracked_uids(&self) -> usize {
        self.history.len()
    }

    /// Maximum capacity of distinct UIDs allowed in memory.
    #[must_use]
    pub const fn max_tracked_uids(&self) -> usize {
        self.config.max_tracked_uids
    }

    /// Check whether a request from `uid` is permitted and record the attempt if allowed.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::RateLimitExceeded`] if the UID has reached the maximum allowed attempts
    /// or if capacity cannot accommodate new UIDs.
    pub fn check_and_record(&mut self, uid: u32, now_monotonic_ns: u64) -> Result<(), PolicyError> {
        if self.config.max_attempts == 0 || self.config.max_tracked_uids == 0 {
            return Err(PolicyError::RateLimitExceeded {
                uid,
                retry_after_ns: self.config.window_duration_ns,
            });
        }

        let cutoff = now_monotonic_ns.saturating_sub(self.config.window_duration_ns);

        // If UID is already tracked, clean its expired timestamps and evaluate
        if let Some(attempts) = self.history.get_mut(&uid) {
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
            return Ok(());
        }

        // New UID: ensure capacity bounds are maintained before insertion
        if self.history.len() >= self.config.max_tracked_uids {
            // First evict any stale UIDs across history
            self.prune_stale(now_monotonic_ns);

            // If still at or over capacity, evict the least recently used UID
            while self.history.len() >= self.config.max_tracked_uids {
                let lru_uid = self
                    .history
                    .iter()
                    .min_by_key(|(_, attempts)| attempts.back().copied().unwrap_or(0))
                    .map(|(&u, _)| u);

                if let Some(evict_uid) = lru_uid {
                    self.history.remove(&evict_uid);
                } else {
                    break;
                }
            }
        }

        if self.history.len() >= self.config.max_tracked_uids {
            return Err(PolicyError::RateLimitExceeded {
                uid,
                retry_after_ns: self.config.window_duration_ns,
            });
        }

        let mut attempts = VecDeque::new();
        attempts.push_back(now_monotonic_ns);
        self.history.insert(uid, attempts);
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
        if self.config.max_attempts == 0 || self.config.max_tracked_uids == 0 {
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
        if self.config.max_attempts == 0 || self.config.max_tracked_uids == 0 {
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

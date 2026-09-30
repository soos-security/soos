//! Per-peer connection and event limits (GitHub #157 DMN-04, GitHub #175 DMN-03 hardening).
//!
//! Every limit is keyed on the kernel `SO_PEERCRED` UID of the connecting process, never on
//! payload data. The PAM module runs inside root processes (gdm-session-worker, sudo, su,
//! login, polkit-agent-helper), so root peers get a reserved share of the global capacity
//! that unprivileged peers (GUI, screen lockers, `soos-admin`, any `soos`-group process)
//! can never consume. Each unprivileged UID additionally holds at most
//! `max_connections_per_uid` connections, so a single account cannot starve other users.
//!
//! The limiter is pure (no I/O, no clock) and its tracking table holds at most one entry per
//! connection currently in use, so its memory is bounded by the global capacity.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use soos_policy::RateLimitConfig;
use thiserror::Error;

use crate::error::DaemonError;

/// Default maximum concurrent connections per unprivileged peer UID.
pub const DEFAULT_MAX_CONNECTIONS_PER_UID: usize = 2;

/// Default number of connection permits reserved for root peers (the PAM module).
pub const DEFAULT_RESERVED_ROOT_CONNECTIONS: usize = 2;

/// Default maximum requests served on one connection before the daemon closes it.
///
/// At the ~30 requests per second of the GUI preview poll this forces a reconnect about
/// every 34 seconds; the lifetime cap below usually triggers first.
pub const DEFAULT_MAX_REQUESTS_PER_CONNECTION: usize = 1024;

/// Default maximum lifetime of one connection, in milliseconds.
pub const DEFAULT_MAX_CONNECTION_LIFETIME_MS: u64 = 30_000;

/// Default `PasswordFailed` events accepted per peer UID within [`DEFAULT_EVENT_WINDOW_MS`].
pub const DEFAULT_MAX_EVENTS_PER_WINDOW: u32 = 5;

/// Default sliding window of the per-peer event quota, in milliseconds.
pub const DEFAULT_EVENT_WINDOW_MS: u64 = 10_000;

/// Maximum distinct peer UIDs tracked by the event rate limiter (bounded memory).
pub const EVENT_RATE_MAX_TRACKED_UIDS: usize = 256;

/// Configuration of the `[peer_limits]` section of `daemon.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerLimitsConfig {
    /// Maximum concurrent connections of one unprivileged peer UID (root is exempt).
    pub max_connections_per_uid: usize,
    /// Permits of the global capacity usable only by root peers. Clamped at runtime so that
    /// at least one permit remains for unprivileged peers; rejected by [`Self::validate`]
    /// when it is not strictly below the capacity.
    pub reserved_root_connections: usize,
    /// Requests served on one connection before it is closed.
    pub max_requests_per_connection: usize,
    /// Maximum lifetime of one connection; checked before every request read.
    pub max_connection_lifetime: Duration,
    /// `PasswordFailed` events accepted per peer UID within `event_window` (root included).
    /// `0` drops every event.
    pub max_events_per_window: u32,
    /// Sliding window of the per-peer event quota.
    pub event_window: Duration,
}

impl Default for PeerLimitsConfig {
    fn default() -> Self {
        Self {
            max_connections_per_uid: DEFAULT_MAX_CONNECTIONS_PER_UID,
            reserved_root_connections: DEFAULT_RESERVED_ROOT_CONNECTIONS,
            max_requests_per_connection: DEFAULT_MAX_REQUESTS_PER_CONNECTION,
            max_connection_lifetime: Duration::from_millis(DEFAULT_MAX_CONNECTION_LIFETIME_MS),
            max_events_per_window: DEFAULT_MAX_EVENTS_PER_WINDOW,
            event_window: Duration::from_millis(DEFAULT_EVENT_WINDOW_MS),
        }
    }
}

impl PeerLimitsConfig {
    /// Validates the limits against the dispatcher capacity (`max_concurrent_connections`).
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::Config`] when the capacity or any per-connection bound is zero,
    /// or when `reserved_root_connections` leaves no permit for unprivileged peers.
    pub fn validate(&self, max_concurrent_connections: usize) -> Result<(), DaemonError> {
        if max_concurrent_connections == 0 {
            return Err(DaemonError::Config(
                "[dispatcher] max_concurrent_connections must be at least 1".into(),
            ));
        }
        if self.reserved_root_connections >= max_concurrent_connections {
            return Err(DaemonError::Config(format!(
                "[peer_limits] reserved_root_connections ({}) must be lower than \
                 [dispatcher] max_concurrent_connections ({})",
                self.reserved_root_connections, max_concurrent_connections
            )));
        }
        if self.max_connections_per_uid == 0 {
            return Err(DaemonError::Config(
                "[peer_limits] max_connections_per_uid must be at least 1".into(),
            ));
        }
        if self.max_requests_per_connection == 0 {
            return Err(DaemonError::Config(
                "[peer_limits] max_requests_per_connection must be at least 1".into(),
            ));
        }
        if self.max_connection_lifetime.is_zero() {
            return Err(DaemonError::Config(
                "[peer_limits] max_connection_lifetime_ms must be at least 1".into(),
            ));
        }
        if self.event_window.is_zero() {
            return Err(DaemonError::Config(
                "[peer_limits] event_window_ms must be at least 1".into(),
            ));
        }
        Ok(())
    }

    /// Rate limiter parameters of the per-peer `PasswordFailed` event quota.
    #[must_use]
    pub fn event_rate_limit_config(&self) -> RateLimitConfig {
        let window_ns = u64::try_from(self.event_window.as_nanos()).unwrap_or(u64::MAX);
        RateLimitConfig::new_with_capacity(
            self.max_events_per_window,
            window_ns,
            EVENT_RATE_MAX_TRACKED_UIDS,
        )
    }
}

/// Reason a connection was refused by [`PeerConnectionLimiter::try_acquire`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ConnectionRejection {
    /// Every permit of the global capacity is in use.
    #[error("global connection capacity exhausted")]
    GlobalCapacity,
    /// Only permits reserved for root peers remain.
    #[error("remaining connection permits are reserved for root peers")]
    ReservedForPrivileged,
    /// The unprivileged peer UID already holds its maximum number of connections.
    #[error("per-UID connection cap reached")]
    PerUidCap,
}

#[derive(Debug, Default)]
struct LimiterState {
    total: usize,
    unprivileged: usize,
    per_uid: HashMap<u32, usize>,
}

#[derive(Debug)]
struct LimiterShared {
    capacity: usize,
    unprivileged_capacity: usize,
    per_uid_cap: usize,
    state: Mutex<LimiterState>,
}

impl LimiterShared {
    fn lock(&self) -> MutexGuard<'_, LimiterState> {
        // The state holds plain counters; a poisoned lock never leaves them inconsistent
        // because every update below is completed without intermediate panics.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Bounded connection admission keyed on the kernel peer UID.
#[derive(Debug, Clone)]
pub struct PeerConnectionLimiter {
    shared: Arc<LimiterShared>,
}

impl PeerConnectionLimiter {
    /// Builds a limiter for `capacity` concurrent connections.
    ///
    /// `reserved_root_connections` is clamped to `capacity - 1` (and to zero for a capacity
    /// of one) so that unprivileged peers keep at least one permit.
    #[must_use]
    pub fn new(capacity: usize, limits: &PeerLimitsConfig) -> Self {
        let max_reserved = capacity.saturating_sub(1);
        let reserved = limits.reserved_root_connections.min(max_reserved);
        Self {
            shared: Arc::new(LimiterShared {
                capacity,
                unprivileged_capacity: capacity.saturating_sub(reserved),
                per_uid_cap: limits.max_connections_per_uid,
                state: Mutex::new(LimiterState::default()),
            }),
        }
    }

    /// Tries to admit one connection of `peer_uid`; the permit is released on drop.
    ///
    /// # Errors
    ///
    /// Returns the [`ConnectionRejection`] reason when the peer must be refused.
    pub fn try_acquire(&self, peer_uid: u32) -> Result<PeerConnectionPermit, ConnectionRejection> {
        let shared = &self.shared;
        let mut state = shared.lock();
        if state.total >= shared.capacity {
            return Err(ConnectionRejection::GlobalCapacity);
        }
        if peer_uid != 0 {
            if state.unprivileged >= shared.unprivileged_capacity {
                return Err(ConnectionRejection::ReservedForPrivileged);
            }
            let held = state.per_uid.get(&peer_uid).copied().unwrap_or(0);
            if held >= shared.per_uid_cap {
                return Err(ConnectionRejection::PerUidCap);
            }
            state.per_uid.insert(peer_uid, held.saturating_add(1));
            state.unprivileged = state.unprivileged.saturating_add(1);
        }
        state.total = state.total.saturating_add(1);
        drop(state);
        Ok(PeerConnectionPermit {
            shared: Arc::clone(&self.shared),
            peer_uid,
        })
    }

    /// Number of permits currently in use.
    #[must_use]
    pub fn in_use(&self) -> usize {
        self.shared.lock().total
    }

    /// Number of permits currently free (root view: reserved permits included).
    #[must_use]
    pub fn available(&self) -> usize {
        self.shared.capacity.saturating_sub(self.in_use())
    }

    /// Number of distinct unprivileged UIDs currently holding at least one permit.
    #[must_use]
    pub fn tracked_uids(&self) -> usize {
        self.shared.lock().per_uid.len()
    }
}

/// RAII connection permit returned by [`PeerConnectionLimiter::try_acquire`].
#[derive(Debug)]
pub struct PeerConnectionPermit {
    shared: Arc<LimiterShared>,
    peer_uid: u32,
}

impl PeerConnectionPermit {
    /// Kernel peer UID this permit was granted to.
    #[must_use]
    pub const fn peer_uid(&self) -> u32 {
        self.peer_uid
    }
}

impl Drop for PeerConnectionPermit {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.total = state.total.saturating_sub(1);
        if self.peer_uid != 0 {
            state.unprivileged = state.unprivileged.saturating_sub(1);
            let remaining = state
                .per_uid
                .get(&self.peer_uid)
                .copied()
                .unwrap_or(0)
                .saturating_sub(1);
            if remaining == 0 {
                state.per_uid.remove(&self.peer_uid);
            } else {
                state.per_uid.insert(self.peer_uid, remaining);
            }
        }
    }
}

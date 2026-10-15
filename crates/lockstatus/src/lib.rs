//! # soos-lockstatus
//!
//! User-level bridge between the soos daemon's read-only lock-screen state channel
//! (`RequestKind::LockState`, ADR 2026-10-13 "Read-Only Lock-Screen State Channel") and the
//! screen-locker adapters (ADR 2026-10-15 "Lock-Screen Adapters for swaylock-plugin and hyprlock",
//! GitHub #401, `Docs/LOCKSCREEN_WIDGET.md`).
//!
//! - **Bounded blocking IPC only**: one short `std::os::unix::net::UnixStream` exchange per poll
//!   under one cumulative deadline; no async runtime, no D-Bus, no network.
//! - **One exchange in flight per user**: every client of one user (hyprlock labels, `--watch`,
//!   `soos-lock-widget`) coordinates through a lock file in `$XDG_RUNTIME_DIR` that also caches
//!   the last answer for [`shared::CACHE_FRESH_MS`], so these clients never hold more than one
//!   daemon connection of the user and never reach the daemon quota.
//! - **Three visible states**: a locked screen only ever shows Idle, Scanning or Not recognized
//!   (owner decision of 2026-10-13); a transport error renders exactly like Idle.
//! - Nothing biometric is ever read: the daemon answers a coarse state and a counter only.

#![forbid(unsafe_code)]

pub mod args;
pub mod client;
pub mod format;
pub mod motion;
pub mod poll;
pub mod shared;
pub mod view;

/// Default path of the daemon socket (`0660 root:soos`, never widened).
pub const DEFAULT_SOCKET_PATH: &str = "/run/soos/daemon.sock";

/// `Request::service` of every lock-state request (at most `MAX_SERVICE_LEN` bytes).
pub const CLIENT_SERVICE: &str = "soos-lockstatus";

/// Cumulative deadline of one poll: lock wait, connect, write and read together.
pub const EXCHANGE_BUDGET_MS: u64 = 250;

/// Documented upper bound of one `--once` run (the exchange budget plus start-up and output).
pub const ONCE_EXIT_BUDGET_MS: u64 = 300;

/// Default poll interval of `--watch` and of the widget.
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 500;

/// Shortest poll interval (4 Hz); smaller requests, `0` included, are clamped up.
pub const MIN_POLL_INTERVAL_MS: u64 = 250;

/// Longest poll interval; larger requests are clamped down.
pub const MAX_POLL_INTERVAL_MS: u64 = 2000;

/// Delay after the first, second and every further consecutive failed poll (never shorter than
/// the poll interval); a successful poll resets it.
pub const ERROR_BACKOFF_MS: [u64; 3] = [1000, 2000, 4000];

/// Upper bound of one `--watch` JSON line, newline included.
pub const MAX_JSON_LINE_BYTES: usize = 96;

/// Real UID of this process: the only UID an unprivileged client may ask about.
#[must_use]
pub fn current_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

//! Presence auto-unlock of locked local sessions (GitHub #323).
//!
//! While a local, seat-attached session is locked (logind `LockedHint`), the daemon runs the
//! unchanged face pipeline (camera wake, multi-frame PAD consensus, cosine match) for the
//! session owner and asks systemd-logind to unlock the session after a fresh, single-use
//! `Allow`, a fresh logind re-check and a fresh account check (`pam_faillock`, shadow
//! expiry). Every error, unknown state or timeout leaves the session locked; the locker and
//! its PAM password path are never touched.
//!
//! - [`config`]: `[presence]` configuration and bounds;
//! - [`logind`]: the logind access trait and its zbus implementation (pinned system bus);
//! - [`display`]: lid/screen gating input from sysfs DRM connectors;
//! - [`switch`]: the kill-switch flag files;
//! - [`tracker`]: the pure lock-period tracker and candidate selection;
//! - [`account`]: the account guard (`pam_faillock` tally and `/etc/shadow` expiry);
//! - [`worker`]: the presence worker (one tick = one decision).

#![forbid(unsafe_code)]

use std::time::Duration;

pub mod account;
pub mod config;
pub mod display;
pub mod logind;
pub mod switch;
pub mod tracker;
pub mod worker;

/// Tick period of the worker (one logind snapshot per tick).
pub const LOCK_POLL_INTERVAL_MS: u64 = 1000;

/// Attempts of the shared per-UID rate-limit window that presence never consumes, so PAM
/// always keeps at least the pre-#323 budget.
pub const PRESENCE_RESERVED_ATTEMPTS: u32 = 5;

/// Bound of each logind step of the worker on an established connection; expiry is
/// `PresenceLogindError::Timeout`. A step is the whole `seat_sessions` snapshot taken as one
/// unit (one `ListSessions` plus one `GetAll` per seat session, at most
/// 1 + `MAX_PRESENCE_SEAT_SESSIONS` round trips), `session_state` (`GetSession` + `GetAll`),
/// `lid_closed` or `unlock_session`. Every single D-Bus round trip inside a step is bounded
/// by the same value (zbus `method_timeout` and the per-call timeout of `ZbusLogind`), so the
/// step bound is the binding one. It never covers opening the connection
/// ([`DBUS_CONNECT_TIMEOUT_MS`]).
pub const DBUS_CALL_TIMEOUT_MS: u64 = 500;

/// Bound of one system-bus connection attempt, `PresenceLogind::connect()` (authentication
/// handshake and `Hello`). The worker applies it before the snapshot, outside the
/// [`DBUS_CALL_TIMEOUT_MS`] bound; only `connect()` opens a connection.
pub const DBUS_CONNECT_TIMEOUT_MS: u64 = 1000;

/// First reconnect backoff after a logind failure.
pub const DBUS_RECONNECT_BACKOFF_MIN_MS: u64 = 1000;

/// Upper bound of the reconnect backoff.
pub const DBUS_RECONNECT_BACKOFF_MAX_MS: u64 = 30_000;

/// Seat-attached sessions examined per tick; more skips the tick.
pub const MAX_PRESENCE_SEAT_SESSIONS: usize = 16;

/// Capacity of the lock tracker; overflow clears it and skips the tick.
pub const MAX_TRACKED_LOCKED_SESSIONS: usize = 16;

/// Maximum time between the consensus `Allow` and the `UnlockSession` call.
pub const MAX_ALLOW_TO_UNLOCK_MS: u64 = 1000;

/// A session still locked this long after a successful unlock call is `locker_ignored`.
pub const UNLOCK_CONFIRM_TIMEOUT_MS: u64 = 5000;

/// Bytes of a D-Bus error name/message kept in `PresenceLogindError::Call`.
pub const MAX_LOGIND_ERROR_LEN: usize = 256;

/// Directory entries of the DRM sysfs class directory examined; more is `Unknown`.
pub const MAX_DRM_CONNECTORS: usize = 64;

/// Bytes read from one DRM connector `status` / `dpms` attribute; longer is ignored.
pub const MAX_SYSFS_ATTR_BYTES: usize = 64;

/// Bound of one account check on the blocking pool; expiry is `Undeterminable`.
pub const ACCOUNT_CHECK_TIMEOUT_MS: u64 = 500;

/// Maximum length in bytes of a logind user name (`UserName::parse`).
pub const MAX_USER_NAME_LEN: usize = 256;

/// `pam_faillock` configuration file.
pub const DEFAULT_FAILLOCK_CONF: &str = "/etc/security/faillock.conf";

/// Vendor `pam_faillock` configuration file (read only when the `/etc` file is absent).
pub const VENDOR_FAILLOCK_CONF: &str = "/usr/etc/security/faillock.conf";

/// Default `pam_faillock` tally directory (`FAILLOCK_DEFAULT_TALLYDIR`).
pub const DEFAULT_FAILLOCK_DIR: &str = "/run/faillock";

/// `pam_faillock` default `deny`.
pub const DEFAULT_FAILLOCK_DENY: u16 = 3;

/// `pam_faillock` default `fail_interval` (seconds).
pub const DEFAULT_FAILLOCK_FAIL_INTERVAL_S: u32 = 900;

/// `pam_faillock` default `unlock_time` (seconds); `root_unlock_time` defaults to it.
pub const DEFAULT_FAILLOCK_UNLOCK_TIME_S: u32 = 600;

/// `pam_faillock` `MAX_TIME_INTERVAL` (7 days); larger values are `Undeterminable`.
pub const MAX_FAILLOCK_TIME_INTERVAL: u64 = 604_800;

/// Size of one `struct tally` record.
pub const TALLY_RECORD_BYTES: usize = 64;

/// `TALLY_STATUS_VALID` bit of a tally record.
pub const TALLY_STATUS_VALID: u16 = 0x1;

/// Largest tally file read (`MAX_RECORDS` 1024 plus one chunk: 1088 records).
pub const MAX_TALLY_BYTES: usize = 1088 * TALLY_RECORD_BYTES;

/// Largest `faillock.conf` read.
pub const MAX_FAILLOCK_CONF_BYTES: usize = 65_536;

/// Largest PAM stack file read.
pub const MAX_PAM_FILE_BYTES: usize = 65_536;

/// PAM stack directories scanned for `pam_faillock.so` policy options.
pub const DEFAULT_PAM_DIRS: [&str; 3] = ["/etc/pam.d", "/usr/lib/pam.d", "/usr/etc/pam.d"];

/// Single-file PAM configuration, read by libpam only when none of the PAM directories is a
/// directory (scanned like a stack file when one is; `Undeterminable` when none is).
pub const DEFAULT_PAM_CONF: &str = "/etc/pam.conf";

/// Directory entries examined per PAM directory; more is `Undeterminable`.
pub const MAX_PAM_DIR_ENTRIES: usize = 512;

/// Shadow password file.
pub const DEFAULT_SHADOW_PATH: &str = "/etc/shadow";

/// Largest shadow file read.
pub const MAX_SHADOW_BYTES: usize = 4 * 1024 * 1024;

/// Largest number of shadow lines examined.
pub const MAX_SHADOW_LINES: usize = 65_536;

/// Pinned system bus address (never `DBUS_SYSTEM_BUS_ADDRESS` from the environment).
pub const SYSTEM_BUS_ADDRESS: &str = "unix:path=/run/dbus/system_bus_socket";

/// DRM sysfs class directory.
pub const DEFAULT_DRM_SYSFS_DIR: &str = "/sys/class/drm";

/// Kill-switch directory; equals the PAM flag directory (`soos_pam::config::DEFAULT_FLAG_DIR`).
pub const DEFAULT_KILL_SWITCH_DIR: &str = "/etc/soos";

/// Global disable flag (face PAM and presence).
pub const GLOBAL_DISABLE_FLAG: &str = "disabled";

/// Presence-only disable flag.
pub const PRESENCE_DISABLE_FLAG: &str = "presence.disable";

/// Reconnect backoff after `consecutive_failures` logind failures: zero without a failure,
/// then 1 s doubling per failure, saturating at [`DBUS_RECONNECT_BACKOFF_MAX_MS`].
#[must_use]
pub fn reconnect_backoff(consecutive_failures: u32) -> Duration {
    if consecutive_failures == 0 {
        return Duration::ZERO;
    }
    let exponent = consecutive_failures.saturating_sub(1).min(16);
    let factor = 1u64.checked_shl(exponent).unwrap_or(u64::MAX);
    let ms = DBUS_RECONNECT_BACKOFF_MIN_MS
        .saturating_mul(factor)
        .min(DBUS_RECONNECT_BACKOFF_MAX_MS);
    Duration::from_millis(ms)
}

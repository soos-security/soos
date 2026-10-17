//! Offline history and back-online push notifications (GitHub #376, tier 1 of #359 Part B;
//! ADR 2026-10-13 "Offline History and Back-Online Push Notifications in `soos-remote`";
//! architect spec `AI/architect_spec_remote_offline_alerts.md`, §15 and §16 authoritative).
//!
//! When `offline_alerts = true` (which requires `push_notifications = true`), the service
//! records the intervals during which the PC was offline, at most `MAX_OFFLINE_INTERVALS`, each
//! with wall start and end times, a duration measured on `CLOCK_BOOTTIME` and a reason from a
//! fixed enum ([`OfflineReason`]). After a `network_lost`, `wifi_disabled` or
//! `shutdown_or_crash` interval, and once the PC is online again, it hands one
//! [`OfflineNotice`] (enums and times only) to the Web Push scheduler.
//!
//! The bus is only read, never subscribed. Every `OFFLINE_TICK_MS` the watcher asks the bus
//! driver whether NetworkManager owns its name (`NameHasOwner`, which never starts a stopped
//! service). When it does, the watcher reads three scalar properties of the manager object with
//! `Properties.Get`: `State`, `Connectivity` and `WirelessEnabled`. No match rule is installed,
//! no proxy or signal subscription is built, and no connection, access-point or device object is
//! ever read. A suspend shows as the gap between `CLOCK_BOOTTIME` and `CLOCK_MONOTONIC`
//! between two ticks. A clean stop reads logind's `PreparingForShutdown` once and writes the stop
//! marker of `remote-network.json`. A run that ended without a marker (crash or power loss)
//! gives one `shutdown_or_crash` interval at the next start.
//!
//! The module never logs through a `tracing` macro: its three audit lines are fixed-text events
//! of [`crate::audit`]. No `std::sync::Mutex` guard is held across an `.await`.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::{sleep, timeout, Instant, MissedTickBehavior};
use zbus::OwnedVariant;

use crate::audit;
use crate::auth::RandomSource;
use crate::credentials::{
    check_parent_dir, read_owned_file, try_lock_file, write_atomic, StoreError, STORE_LOCK_RETRY_MS,
};
use crate::journal::BoxFuture;
use crate::logind::{connect_system_bus, method_error_name, SourceError};
use crate::server::UnixClock;
use crate::{
    DBUS_CALL_TIMEOUT_MS, MAX_NETWORK_STORE_BYTES, MAX_OFFLINE_INTERVALS, NETWORK_READ_DEADLINE_MS,
    OFFLINE_HEARTBEAT_MS, OFFLINE_MIN_INTERVAL_MS, OFFLINE_SHUTDOWN_READ_MS, OFFLINE_TICK_MS,
    OFFLINE_UNMONITORED_RELEASE_TICKS, STORE_LOCK_TIMEOUT_MS,
};

/// Version of the store file.
const STORE_VERSION: u32 = 1;
/// One minute (duration labels).
const MINUTE_MS: u64 = 60_000;
/// One hour (duration labels).
const HOUR_MS: u64 = 3_600_000;
/// One day (duration labels).
const DAY_MS: u64 = 86_400_000;
/// Below this many hours a duration is shown in hours and minutes.
const HOURS_SHOWN_BELOW: u64 = 48;

/// The bus driver (never activates anything).
const BUS_DRIVER: &str = "org.freedesktop.DBus";
/// The bus driver object.
const BUS_DRIVER_PATH: &str = "/org/freedesktop/DBus";
/// NetworkManager's bus name and manager interface.
const NM_NAME: &str = "org.freedesktop.NetworkManager";
/// NetworkManager's manager object.
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
/// logind's bus name.
const LOGIND_NAME: &str = "org.freedesktop.login1";
/// logind's manager object.
const LOGIND_PATH: &str = "/org/freedesktop/login1";
/// logind's manager interface.
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";
/// The standard properties interface.
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

// ---------------------------------------------------------------------------------------
// Types (spec §5.1)
// ---------------------------------------------------------------------------------------

/// Why the PC was offline. No other value is ever recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineReason {
    /// NetworkManager reported no usable connection.
    NetworkLost,
    /// No usable connection while the Wi-Fi radio was switched off.
    WifiDisabled,
    /// The PC was suspended or hibernated.
    Sleep,
    /// The previous run ended in a system shutdown, a crash or a power loss.
    ShutdownOrCrash,
}

impl OfflineReason {
    /// Whether the interval is followed by a back-online push (`false` for `Sleep` only).
    #[must_use]
    pub const fn pushes(self) -> bool {
        !matches!(self, Self::Sleep)
    }
}

/// One recorded interval: wall times in Unix ms (`start <= end`), duration from
/// `CLOCK_BOOTTIME` when the interval lies in one boot, else from the wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineInterval {
    /// Why.
    pub reason: OfflineReason,
    /// Wall start (Unix ms).
    pub start_unix_ms: u64,
    /// Wall end (Unix ms).
    pub end_unix_ms: u64,
    /// Duration (ms).
    pub duration_ms: u64,
}

impl OfflineInterval {
    /// An interval whose end is never before its start (a wall clock stepped backwards yields
    /// an empty wall span, never an inverted one).
    fn new(reason: OfflineReason, start_unix_ms: u64, end_unix_ms: u64, duration_ms: u64) -> Self {
        Self {
            reason,
            start_unix_ms,
            end_unix_ms: end_unix_ms.max(start_unix_ms),
            duration_ms,
        }
    }
}

/// What a back-online push says: the whole input of the payload builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfflineNotice {
    /// Reason of the newest merged interval.
    pub reason: OfflineReason,
    /// Merged intervals (saturating, at least 1).
    pub intervals: u32,
    /// Earliest start of the merged intervals.
    pub first_start_unix_ms: u64,
    /// Latest end of the merged intervals.
    pub last_end_unix_ms: u64,
    /// Duration of the newest merged interval.
    pub last_duration_ms: u64,
}

impl OfflineNotice {
    /// The notice of one interval.
    #[must_use]
    pub fn of(interval: &OfflineInterval) -> Self {
        Self {
            reason: interval.reason,
            intervals: 1,
            first_start_unix_ms: interval.start_unix_ms,
            last_end_unix_ms: interval.end_unix_ms,
            last_duration_ms: interval.duration_ms,
        }
    }

    /// Pure fold: counts add (saturating), the span widens, reason and duration come from the
    /// operand with the greater `last_end_unix_ms` (ties: `newer`).
    #[must_use]
    pub fn merge(older: &Self, newer: &Self) -> Self {
        let newest = if older.last_end_unix_ms > newer.last_end_unix_ms {
            older
        } else {
            newer
        };
        Self {
            reason: newest.reason,
            intervals: older.intervals.saturating_add(newer.intervals),
            first_start_unix_ms: older.first_start_unix_ms.min(newer.first_start_unix_ms),
            last_end_unix_ms: older.last_end_unix_ms.max(newer.last_end_unix_ms),
            last_duration_ms: newest.last_duration_ms,
        }
    }
}

/// One clock sample: wall (Unix ms), `CLOCK_BOOTTIME` (ms) and `CLOCK_MONOTONIC` (ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    /// Wall clock (Unix ms).
    pub unix_ms: u64,
    /// `CLOCK_BOOTTIME` (ms): advances during suspend.
    pub boot_ms: u64,
    /// `CLOCK_MONOTONIC` (ms): stops during suspend.
    pub mono_ms: u64,
}

/// Raw NetworkManager reading: three scalar properties, never a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NmReading {
    /// `State` (`NMState`).
    pub state: u32,
    /// `Connectivity` (`NMConnectivityState`).
    pub connectivity: u32,
    /// `WirelessEnabled`; `None` when that one property could not be read.
    pub wireless_enabled: Option<bool>,
}

/// What one reading means for the offline history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkObservation {
    /// Connected (captive portal and limited connectivity included).
    Online,
    /// Not connected (`NetworkLost` or `WifiDisabled` only).
    Offline(OfflineReason),
    /// No information: no transition.
    Unknown,
}

/// D-4: states 10–40 offline; 50–70 offline only with connectivity `NONE` (1); any other
/// state `Unknown`. An offline reading with `WirelessEnabled = false` is `WifiDisabled`.
#[must_use]
pub fn classify(reading: &NmReading) -> NetworkObservation {
    let offline = || {
        if reading.wireless_enabled == Some(false) {
            NetworkObservation::Offline(OfflineReason::WifiDisabled)
        } else {
            NetworkObservation::Offline(OfflineReason::NetworkLost)
        }
    };
    match reading.state {
        10 | 20 | 30 | 40 => offline(),
        50 | 60 | 70 if reading.connectivity == 1 => offline(),
        50 | 60 | 70 => NetworkObservation::Online,
        _ => NetworkObservation::Unknown,
    }
}

/// Whether NetworkManager may be read: only while it owns its bus name (a call to an unowned
/// name could start a stopped NetworkManager through bus activation).
#[must_use]
pub fn nm_reads_allowed(has_owner: bool) -> bool {
    has_owner
}

/// `run` marker of the store (D-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMarker {
    /// The service runs (or ended without a clean stop).
    Running,
    /// Clean stop outside a system shutdown.
    Stopped,
    /// Clean stop during a system shutdown (or when that could not be read).
    Shutdown,
}

/// D-7: `Running` or `Shutdown` → one `ShutdownOrCrash` interval `[last_alive, now]` when it
/// lasted at least `OFFLINE_MIN_INTERVAL_MS`; `Stopped` → `None`.
#[must_use]
pub fn previous_run_interval(
    run: RunMarker,
    last_alive_unix_ms: u64,
    now_unix_ms: u64,
) -> Option<OfflineInterval> {
    if run == RunMarker::Stopped {
        return None;
    }
    let duration = now_unix_ms.saturating_sub(last_alive_unix_ms);
    (duration >= OFFLINE_MIN_INTERVAL_MS).then(|| {
        OfflineInterval::new(
            OfflineReason::ShutdownOrCrash,
            last_alive_unix_ms,
            now_unix_ms,
            duration,
        )
    })
}

/// The duration of a notification body: "less than a minute", "N min", "H h", "H h M min"
/// (below 48 h) or "D days".
#[must_use]
pub fn duration_label(ms: u64) -> String {
    if ms < MINUTE_MS {
        return "less than a minute".to_string();
    }
    if ms < HOUR_MS {
        return format!("{} min", ms / MINUTE_MS);
    }
    if ms < HOURS_SHOWN_BELOW.saturating_mul(HOUR_MS) {
        let hours = ms / HOUR_MS;
        let minutes = (ms % HOUR_MS) / MINUTE_MS;
        return if minutes == 0 {
            format!("{hours} h")
        } else {
            format!("{hours} h {minutes} min")
        };
    }
    format!("{} days", ms / DAY_MS)
}

// ---------------------------------------------------------------------------------------
// Tracker (spec §5.2, §15 F-2, §16 R2-3)
// ---------------------------------------------------------------------------------------

/// What one step produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickOutcome {
    /// Intervals recorded by this step.
    pub recorded: Vec<OfflineInterval>,
    /// A notice released by this step.
    pub notice: Option<OfflineNotice>,
}

/// An outage not closed yet.
#[derive(Debug, Clone, Copy)]
struct OpenInterval {
    reason: OfflineReason,
    start_unix_ms: u64,
    start_boot_ms: u64,
}

/// Pure tracker: no clock, no I/O.
#[derive(Debug, Default)]
pub struct OfflineTracker {
    history: VecDeque<OfflineInterval>,
    open: Option<OpenInterval>,
    last: Option<ClockSample>,
    pending: Option<OfflineNotice>,
    unknown_streak: u32,
}

impl OfflineTracker {
    /// History loaded from the store (oldest first); only the newest `MAX_OFFLINE_INTERVALS`
    /// are kept.
    #[must_use]
    pub fn new(history: Vec<OfflineInterval>) -> Self {
        let skip = history.len().saturating_sub(MAX_OFFLINE_INTERVALS);
        Self {
            history: history.into_iter().skip(skip).collect(),
            ..Self::default()
        }
    }

    /// Appends `interval` (evicting the oldest beyond the bound); with `notify`, a pushing
    /// reason joins the held notice.
    fn record(&mut self, interval: OfflineInterval, notify: bool, outcome: &mut TickOutcome) {
        while self.history.len() >= MAX_OFFLINE_INTERVALS {
            self.history.pop_front();
        }
        self.history.push_back(interval);
        outcome.recorded.push(interval);
        if notify && interval.reason.pushes() {
            let notice = OfflineNotice::of(&interval);
            self.pending = Some(match self.pending {
                Some(held) => OfflineNotice::merge(&held, &notice),
                None => notice,
            });
        }
    }

    /// Closes the open outage at `at` (recorded when at least the floor).
    fn close_open(&mut self, at: ClockSample, notify: bool, outcome: &mut TickOutcome) {
        if let Some(open) = self.open.take() {
            let duration = at.boot_ms.saturating_sub(open.start_boot_ms);
            if duration >= OFFLINE_MIN_INTERVAL_MS {
                let interval =
                    OfflineInterval::new(open.reason, open.start_unix_ms, at.unix_ms, duration);
                self.record(interval, notify, outcome);
            }
        }
    }

    /// Start step: records `previous`, whose notice is held until a release.
    pub fn start(&mut self, previous: Option<OfflineInterval>, now: ClockSample) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        if let Some(interval) = previous {
            self.record(interval, true, &mut outcome);
        }
        self.last = Some(now);
        outcome
    }

    /// One watcher step (spec §5.2 rules 1–4, release rule of §16 R2-3).
    pub fn tick(&mut self, now: ClockSample, observation: NetworkObservation) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        if let Some(previous) = self.last {
            let slept = now
                .boot_ms
                .saturating_sub(previous.boot_ms)
                .saturating_sub(now.mono_ms.saturating_sub(previous.mono_ms));
            if slept >= OFFLINE_MIN_INTERVAL_MS {
                self.close_open(previous, true, &mut outcome);
                let interval = OfflineInterval::new(
                    OfflineReason::Sleep,
                    now.unix_ms.saturating_sub(slept),
                    now.unix_ms,
                    slept,
                );
                self.record(interval, true, &mut outcome);
            }
        }
        match observation {
            NetworkObservation::Offline(reason) => {
                self.unknown_streak = 0;
                match self.open.as_mut() {
                    None => {
                        self.open = Some(OpenInterval {
                            reason,
                            start_unix_ms: now.unix_ms,
                            start_boot_ms: now.boot_ms,
                        });
                    }
                    Some(open) => {
                        if reason == OfflineReason::WifiDisabled {
                            open.reason = OfflineReason::WifiDisabled;
                        }
                    }
                }
            }
            NetworkObservation::Online => {
                self.unknown_streak = 0;
                self.close_open(now, true, &mut outcome);
                outcome.notice = self.pending.take();
            }
            NetworkObservation::Unknown => {
                self.unknown_streak = self.unknown_streak.saturating_add(1);
                if self.unknown_streak >= OFFLINE_UNMONITORED_RELEASE_TICKS {
                    outcome.notice = self.pending.take();
                }
            }
        }
        self.last = Some(now);
        outcome
    }

    /// Clean stop: closes an open outage at `now` without any notice.
    pub fn stop(&mut self, now: ClockSample) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        self.close_open(now, false, &mut outcome);
        outcome
    }

    /// Newest first.
    #[must_use]
    pub fn history_newest_first(&self) -> Vec<OfflineInterval> {
        self.history.iter().rev().copied().collect()
    }

    /// Oldest first (store order).
    #[must_use]
    pub fn history(&self) -> Vec<OfflineInterval> {
        self.history.iter().copied().collect()
    }
}

// ---------------------------------------------------------------------------------------
// Store (spec §5.3)
// ---------------------------------------------------------------------------------------

/// The parsed store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkStoreFile {
    /// Stop marker.
    pub run: RunMarker,
    /// Last sign of life (Unix ms).
    pub last_alive_unix_ms: u64,
    /// At most `MAX_OFFLINE_INTERVALS` intervals, oldest first.
    pub intervals: Vec<OfflineInterval>,
}

/// Store failure. Fixed texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NetworkStoreError {
    /// Insecure (symlink, mode, owner, not a regular file) or oversized.
    #[error("network history store refused")]
    Refused,
    /// Not a valid store (never repaired).
    #[error("network history store malformed")]
    Malformed,
    /// An I/O operation failed.
    #[error("network history store input/output failed")]
    Io,
    /// Another writer holds the store lock.
    #[error("network history store busy")]
    Locked,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreIn {
    version: u32,
    run: RunMarker,
    last_alive_unix_ms: u64,
    intervals: Vec<OfflineInterval>,
}

#[derive(Serialize)]
struct StoreOut<'a> {
    version: u32,
    run: RunMarker,
    last_alive_unix_ms: u64,
    intervals: &'a [OfflineInterval],
}

fn from_store_error(err: StoreError) -> NetworkStoreError {
    match err {
        StoreError::Insecure | StoreError::TooLarge => NetworkStoreError::Refused,
        StoreError::Malformed => NetworkStoreError::Malformed,
        StoreError::Busy => NetworkStoreError::Locked,
        _ => NetworkStoreError::Io,
    }
}

/// `remote-network.json` (`0600`, owner-checked, `O_NOFOLLOW`, at most
/// `MAX_NETWORK_STORE_BYTES`, written under `flock` by temp file + `fsync` + `rename`).
#[derive(Clone)]
pub struct NetworkStore {
    path: PathBuf,
    owner_uid: u32,
}

impl std::fmt::Debug for NetworkStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("network history store")
    }
}

impl NetworkStore {
    /// Store at `path`, owned by `owner_uid`.
    #[must_use]
    pub fn new(path: PathBuf, owner_uid: u32) -> Self {
        Self { path, owner_uid }
    }

    /// Absent → `Ok(None)`; present → strict checks, then strict JSON.
    ///
    /// # Errors
    /// [`NetworkStoreError::Refused`], [`NetworkStoreError::Malformed`],
    /// [`NetworkStoreError::Io`].
    pub fn load(&self) -> Result<Option<NetworkStoreFile>, NetworkStoreError> {
        let read = read_owned_file(&self.path, self.owner_uid, MAX_NETWORK_STORE_BYTES)
            .map_err(from_store_error)?;
        let Some((bytes, _)) = read else {
            return Ok(None);
        };
        let parsed: StoreIn =
            serde_json::from_slice(&bytes).map_err(|_| NetworkStoreError::Malformed)?;
        if parsed.version != STORE_VERSION
            || parsed.intervals.len() > MAX_OFFLINE_INTERVALS
            || parsed
                .intervals
                .iter()
                .any(|i| i.start_unix_ms > i.end_unix_ms)
        {
            return Err(NetworkStoreError::Malformed);
        }
        Ok(Some(NetworkStoreFile {
            run: parsed.run,
            last_alive_unix_ms: parsed.last_alive_unix_ms,
            intervals: parsed.intervals,
        }))
    }

    /// One locked write (`Locked` when another holder has the lock; nothing written then).
    ///
    /// # Errors
    /// [`NetworkStoreError`].
    pub fn save(
        &self,
        file: &NetworkStoreFile,
        random: &RandomSource,
    ) -> Result<(), NetworkStoreError> {
        let _lock = match try_lock_file(&self.path, self.owner_uid) {
            Ok(Some(lock)) => lock,
            Ok(None) => return Err(NetworkStoreError::Locked),
            Err(err) => return Err(from_store_error(err)),
        };
        let dir = check_parent_dir(&self.path, self.owner_uid).map_err(from_store_error)?;
        if file.intervals.len() > MAX_OFFLINE_INTERVALS {
            return Err(NetworkStoreError::Io);
        }
        let bytes = serde_json::to_vec(&StoreOut {
            version: STORE_VERSION,
            run: file.run,
            last_alive_unix_ms: file.last_alive_unix_ms,
            intervals: &file.intervals,
        })
        .map_err(|_| NetworkStoreError::Io)?;
        if bytes.len() > MAX_NETWORK_STORE_BYTES {
            return Err(NetworkStoreError::Io);
        }
        write_atomic(&self.path, &dir, &bytes, random).map_err(from_store_error)
    }
}

// ---------------------------------------------------------------------------------------
// Seams (spec §5.4, §15 F-1, F-3, F-5)
// ---------------------------------------------------------------------------------------

/// The bus seam.
pub trait NetworkSource: Send + Sync + 'static {
    /// One reading of NetworkManager (only while it owns its name).
    fn observe(&self) -> BoxFuture<'_, Result<NmReading, SourceError>>;
    /// logind `Manager.PreparingForShutdown`.
    fn preparing_for_shutdown(&self) -> BoxFuture<'_, Result<bool, SourceError>>;
}

/// The clock seam; `None` when a clock could not be read (the step is skipped).
pub trait OfflineClock: Send + Sync + 'static {
    /// One sample.
    fn sample(&self) -> Option<ClockSample>;
}

/// Production clock: the injected Unix clock plus `CLOCK_BOOTTIME` and `CLOCK_MONOTONIC`.
pub struct SystemOfflineClock {
    unix: UnixClock,
}

impl std::fmt::Debug for SystemOfflineClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("system offline clock")
    }
}

impl SystemOfflineClock {
    /// The clock over the injected Unix-ms source.
    #[must_use]
    pub fn new(unix: UnixClock) -> Self {
        Self { unix }
    }
}

/// One kernel clock in ms (`None` on failure or a negative value).
fn kernel_ms(id: nix::time::ClockId) -> Option<u64> {
    let spec = nix::time::clock_gettime(id).ok()?;
    let seconds = u64::try_from(spec.tv_sec()).ok()?;
    let nanos = u64::try_from(spec.tv_nsec()).ok()?;
    Some(
        seconds
            .saturating_mul(1000)
            .saturating_add(nanos / 1_000_000),
    )
}

impl OfflineClock for SystemOfflineClock {
    fn sample(&self) -> Option<ClockSample> {
        let boot_ms = kernel_ms(nix::time::ClockId::CLOCK_BOOTTIME)?;
        let mono_ms = kernel_ms(nix::time::ClockId::CLOCK_MONOTONIC)?;
        Some(ClockSample {
            unix_ms: (self.unix)(),
            boot_ms,
            mono_ms,
        })
    }
}

/// Production bus access: bounded reads over the pinned system bus, the connection opened
/// lazily through [`connect_system_bus`] and dropped after a transport failure, a timeout or a
/// malformed reply.
#[derive(Debug, Default)]
pub struct ZbusNetworkSource {
    slot: tokio::sync::Mutex<Option<zbus::Connection>>,
}

impl ZbusNetworkSource {
    /// Creates the client; performs no I/O.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    async fn connection(&self) -> Result<zbus::Connection, SourceError> {
        let mut slot = self.slot.lock().await;
        if let Some(connection) = slot.as_ref() {
            return Ok(connection.clone());
        }
        let connection = connect_system_bus().await?;
        *slot = Some(connection.clone());
        Ok(connection)
    }

    async fn reset(&self) {
        *self.slot.lock().await = None;
    }

    /// Maps one reply: a method error keeps the connection, anything else drops it.
    async fn reply(
        &self,
        result: Result<zbus::Result<zbus::Message>, tokio::time::error::Elapsed>,
    ) -> Result<zbus::Message, SourceError> {
        match result {
            Ok(Ok(message)) => Ok(message),
            Ok(Err(error)) if method_error_name(&error).is_some() => Err(SourceError::Call),
            Ok(Err(_)) => {
                self.reset().await;
                Err(SourceError::BusUnavailable)
            }
            Err(_) => {
                self.reset().await;
                Err(SourceError::Timeout)
            }
        }
    }

    /// `NameHasOwner(NetworkManager)` on the bus driver (never activates a service).
    async fn networkmanager_running(&self) -> Result<bool, SourceError> {
        let connection = self.connection().await?;
        let result = timeout(
            Duration::from_millis(DBUS_CALL_TIMEOUT_MS),
            connection.call_method(
                Some(BUS_DRIVER),
                BUS_DRIVER_PATH,
                Some(BUS_DRIVER),
                "NameHasOwner",
                &(NM_NAME,),
            ),
        )
        .await;
        let message = self.reply(result).await?;
        match message.body().deserialize::<bool>() {
            Ok(owned) => Ok(owned),
            Err(_) => {
                self.reset().await;
                Err(SourceError::Malformed)
            }
        }
    }

    /// One `Properties.Get(interface, property)` of `destination` at `object`.
    async fn property(
        &self,
        destination: &'static str,
        object: &'static str,
        interface: &'static str,
        property: &'static str,
    ) -> Result<OwnedVariant, SourceError> {
        let connection = self.connection().await?;
        let result = timeout(
            Duration::from_millis(DBUS_CALL_TIMEOUT_MS),
            connection.call_method(
                Some(destination),
                object,
                Some(PROPERTIES),
                "Get",
                &(interface, property),
            ),
        )
        .await;
        let message = self.reply(result).await?;
        match message.body().deserialize::<OwnedVariant>() {
            Ok(value) => Ok(value),
            Err(_) => {
                self.reset().await;
                Err(SourceError::Malformed)
            }
        }
    }

    async fn read_networkmanager(&self) -> Result<NmReading, SourceError> {
        let running = self.networkmanager_running().await?;
        if !nm_reads_allowed(running) {
            return Err(SourceError::Call);
        }
        let state = u32::try_from(self.property(NM_NAME, NM_PATH, NM_NAME, "State").await?)
            .map_err(|_| SourceError::Malformed)?;
        let connectivity = u32::try_from(
            self.property(NM_NAME, NM_PATH, NM_NAME, "Connectivity")
                .await?,
        )
        .map_err(|_| SourceError::Malformed)?;
        let wireless_enabled = match self
            .property(NM_NAME, NM_PATH, NM_NAME, "WirelessEnabled")
            .await
        {
            Ok(value) => bool::try_from(value).ok(),
            Err(_) => None,
        };
        Ok(NmReading {
            state,
            connectivity,
            wireless_enabled,
        })
    }

    async fn read_shutdown(&self) -> Result<bool, SourceError> {
        let value = self
            .property(
                LOGIND_NAME,
                LOGIND_PATH,
                LOGIND_MANAGER,
                "PreparingForShutdown",
            )
            .await?;
        bool::try_from(value).map_err(|_| SourceError::Malformed)
    }
}

impl NetworkSource for ZbusNetworkSource {
    fn observe(&self) -> BoxFuture<'_, Result<NmReading, SourceError>> {
        Box::pin(self.read_networkmanager())
    }

    fn preparing_for_shutdown(&self) -> BoxFuture<'_, Result<bool, SourceError>> {
        Box::pin(self.read_shutdown())
    }
}

// ---------------------------------------------------------------------------------------
// Runtime and view (spec §5.5)
// ---------------------------------------------------------------------------------------

/// What `main` resolves for the runtime.
#[derive(Debug, Clone)]
pub struct OfflineSettings {
    /// `remote-network.json` next to the credential store.
    pub store_path: PathBuf,
}

/// The notice sink handed over by the push runtime.
pub(crate) type OfflineNoticeSink = Arc<dyn Fn(OfflineNotice) + Send + Sync>;

/// `state` of `GET /api/offline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineState {
    /// `offline_alerts` is off.
    Disabled,
    /// Recording.
    Active,
    /// The store cannot be used.
    Unavailable,
}

/// `reason` of an unavailable view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineUnavailableReason {
    /// The store is invalid or insecure (never overwritten).
    StoreFailed,
}

/// Whether NetworkManager could be read at the last step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMonitor {
    /// Not active, or no step yet.
    Unknown,
    /// The last reading succeeded.
    Active,
    /// The last reading failed (NetworkManager absent, stopped or unreachable).
    Unavailable,
}

/// JSON of `GET /api/offline` (exactly four keys).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfflineView {
    /// State.
    pub state: OfflineState,
    /// `Some` iff `Unavailable`.
    pub reason: Option<OfflineUnavailableReason>,
    /// `Unknown` unless `Active`.
    pub network_monitor: NetworkMonitor,
    /// Newest first, empty unless `Active`.
    pub intervals: Vec<OfflineInterval>,
}

impl OfflineView {
    /// The view while `offline_alerts` is off.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            state: OfflineState::Disabled,
            reason: None,
            network_monitor: NetworkMonitor::Unknown,
            intervals: Vec::new(),
        }
    }

    fn store_failed() -> Self {
        Self {
            state: OfflineState::Unavailable,
            reason: Some(OfflineUnavailableReason::StoreFailed),
            network_monitor: NetworkMonitor::Unknown,
            intervals: Vec::new(),
        }
    }
}

const MONITOR_UNKNOWN: u8 = 0;
const MONITOR_ACTIVE: u8 = 1;
const MONITOR_UNAVAILABLE: u8 = 2;

/// The offline state shared by the watcher, the view and the stop step.
pub(crate) struct OfflineRuntime {
    store: NetworkStore,
    random: RandomSource,
    clock: Arc<dyn OfflineClock>,
    source: Arc<dyn NetworkSource>,
    sink: Option<OfflineNoticeSink>,
    tracker: Mutex<OfflineTracker>,
    active: AtomicBool,
    failed: AtomicBool,
    monitor: AtomicU8,
    persist_failing: AtomicBool,
    /// Serializes every save; the owned guard travels into the blocking task (R2-1).
    save_gate: Arc<tokio::sync::Mutex<()>>,
    last_save: Mutex<Option<Instant>>,
}

impl OfflineRuntime {
    /// The runtime before its store is read (no I/O).
    pub(crate) fn new(
        settings: &OfflineSettings,
        file_owner_uid: u32,
        random: RandomSource,
        clock: Arc<dyn OfflineClock>,
        source: Arc<dyn NetworkSource>,
        sink: Option<OfflineNoticeSink>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: NetworkStore::new(settings.store_path.clone(), file_owner_uid),
            random,
            clock,
            source,
            sink,
            tracker: Mutex::new(OfflineTracker::default()),
            active: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            monitor: AtomicU8::new(MONITOR_UNKNOWN),
            persist_failing: AtomicBool::new(false),
            save_gate: Arc::new(tokio::sync::Mutex::new(())),
            last_save: Mutex::new(None),
        })
    }

    /// `true` once the store was usable at start.
    pub(crate) fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Start: the store is read on the blocking pool; a refused or malformed store leaves the
    /// feature `unavailable` (no bus read, the file untouched). Otherwise the previous run is
    /// classified and the `running` marker written (the only place the file is created).
    pub(crate) async fn start(&self) {
        let store = self.store.clone();
        let loaded = tokio::task::spawn_blocking(move || store.load())
            .await
            .unwrap_or(Err(NetworkStoreError::Io));
        let Ok(previous) = loaded else {
            self.failed.store(true, Ordering::SeqCst);
            audit::offline_unavailable();
            return;
        };
        let now = self.clock.sample();
        let (history, classified) = match previous {
            Some(file) => {
                let classified = now.and_then(|now| {
                    previous_run_interval(file.run, file.last_alive_unix_ms, now.unix_ms)
                });
                (file.intervals, classified)
            }
            None => (Vec::new(), None),
        };
        if let Ok(mut tracker) = self.tracker.lock() {
            *tracker = OfflineTracker::new(history);
            if let Some(now) = now {
                tracker.start(classified, now);
            }
        }
        self.active.store(true, Ordering::SeqCst);
        audit::offline_active();
        if let Some(now) = now {
            self.persist(RunMarker::Running, now.unix_ms).await;
        }
    }

    /// The watcher: one step every `OFFLINE_TICK_MS` (the first at once); no error ends it.
    pub(crate) async fn run_watcher(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(Duration::from_millis(OFFLINE_TICK_MS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            self.step().await;
        }
    }

    async fn step(&self) {
        let Some(now) = self.clock.sample() else {
            return;
        };
        let observation = match timeout(
            Duration::from_millis(NETWORK_READ_DEADLINE_MS),
            self.source.observe(),
        )
        .await
        {
            Ok(Ok(reading)) => {
                self.monitor.store(MONITOR_ACTIVE, Ordering::SeqCst);
                classify(&reading)
            }
            Ok(Err(_)) | Err(_) => {
                self.monitor.store(MONITOR_UNAVAILABLE, Ordering::SeqCst);
                NetworkObservation::Unknown
            }
        };
        let outcome = match self.tracker.lock() {
            Ok(mut tracker) => tracker.tick(now, observation),
            Err(_) => return,
        };
        if let (Some(notice), Some(sink)) = (outcome.notice, self.sink.as_ref()) {
            sink(notice);
        }
        let heartbeat_due = self.last_save.lock().map_or(true, |last| {
            last.is_none_or(|at| {
                Instant::now().saturating_duration_since(at)
                    >= Duration::from_millis(OFFLINE_HEARTBEAT_MS)
            })
        });
        if !outcome.recorded.is_empty() || heartbeat_due {
            self.persist(RunMarker::Running, now.unix_ms).await;
        }
    }

    /// Clean stop (after the watcher was aborted): reads `PreparingForShutdown` under
    /// `OFFLINE_SHUTDOWN_READ_MS`, closes an open outage and writes the marker. The caller
    /// bounds the whole step by `OFFLINE_STOP_DEADLINE_MS`.
    pub(crate) async fn stop(&self) {
        if !self.is_active() {
            return;
        }
        let Some(now) = self.clock.sample() else {
            return;
        };
        let marker = match timeout(
            Duration::from_millis(OFFLINE_SHUTDOWN_READ_MS),
            self.source.preparing_for_shutdown(),
        )
        .await
        {
            Ok(Ok(false)) => RunMarker::Stopped,
            Ok(Ok(true) | Err(_)) | Err(_) => RunMarker::Shutdown,
        };
        if let Ok(mut tracker) = self.tracker.lock() {
            tracker.stop(now);
        }
        self.persist(marker, now.unix_ms).await;
    }

    /// Writes the current history with `run` and `last_alive_unix_ms`; `Locked` is retried
    /// every `STORE_LOCK_RETRY_MS` for at most `STORE_LOCK_TIMEOUT_MS`.
    async fn persist(&self, run: RunMarker, last_alive_unix_ms: u64) {
        let intervals = match self.tracker.lock() {
            Ok(tracker) => tracker.history(),
            Err(_) => return,
        };
        let file = Arc::new(NetworkStoreFile {
            run,
            last_alive_unix_ms,
            intervals,
        });
        let started = Instant::now();
        let result = loop {
            let guard = Arc::clone(&self.save_gate).lock_owned().await;
            let (store, random, file) = (
                self.store.clone(),
                Arc::clone(&self.random),
                Arc::clone(&file),
            );
            let attempt = tokio::task::spawn_blocking(move || {
                let _guard = guard;
                store.save(&file, &random)
            })
            .await
            .unwrap_or(Err(NetworkStoreError::Io));
            match attempt {
                Err(NetworkStoreError::Locked)
                    if started.elapsed() < Duration::from_millis(STORE_LOCK_TIMEOUT_MS) =>
                {
                    sleep(Duration::from_millis(STORE_LOCK_RETRY_MS)).await;
                }
                other => break other,
            }
        };
        if let Ok(mut last) = self.last_save.lock() {
            *last = Some(Instant::now());
        }
        match result {
            Ok(()) => self.persist_failing.store(false, Ordering::SeqCst),
            Err(_) => {
                if !self.persist_failing.swap(true, Ordering::SeqCst) {
                    audit::offline_not_persisted();
                }
            }
        }
    }

    /// `GET /api/offline`.
    pub(crate) fn view(&self) -> OfflineView {
        if self.failed.load(Ordering::SeqCst) {
            return OfflineView::store_failed();
        }
        if !self.is_active() {
            return OfflineView::disabled();
        }
        let intervals = self
            .tracker
            .lock()
            .map(|t| t.history_newest_first())
            .unwrap_or_default();
        let network_monitor = match self.monitor.load(Ordering::SeqCst) {
            MONITOR_ACTIVE => NetworkMonitor::Active,
            MONITOR_UNAVAILABLE => NetworkMonitor::Unavailable,
            _ => NetworkMonitor::Unknown,
        };
        OfflineView {
            state: OfflineState::Active,
            reason: None,
            network_monitor,
            intervals,
        }
    }
}

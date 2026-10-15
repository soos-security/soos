# Architect Spec — GitHub #376: Offline History and a Back-Online Push in `soos-remote`

- **Date**: 2026-10-13
- **Issue**: GitHub #376 = tier 1 of #359 Part B (planned split item 2); GitHub-only, no backlog id
- **Branch**: `feat/remote-offline-alerts` (not registered in `scripts/sync_issue.py`; the commit
  carries `Closes #376` and `Refs #359`)
- **Owner decisions used**: #359 (2026-10-12: back-online alert wanted, real-time alert comes
  from tier 2/3; 2026-10-13 Part A answers), #362 (2026-10-12: the install enables everything by
  default, each feature toggleable later)
- **ADR**: new entry "Offline History and Back-Online Push Notifications in `soos-remote`",
  amending item (1) of the 2026-10-06 Web Push ADR (§12 below)

> **Reading order:** §15 (Revision 2) and §16 (Revision 3) are authoritative. Any value, signature,
> test or text in §0–§14 that they change is superseded: §3 constants, §4 `effective` (removed),
> §5.4 `SystemTime` (removed), §5.5/§6 `PushDisabled` (removed), the §8 push-disabled page string
> (removed), and §10 tests 6, 17 and 23.

## 0. Decisions

| # | Decision | Why |
|---|---|---|
| D-1 | **No signal match, no `MessageStream`, no proxy.** NetworkManager is *read* on a fixed tick (`OFFLINE_TICK_MS` = 5 s) with three bounded `org.freedesktop.DBus.Properties.Get` round trips (`State`, `Connectivity`, `WirelessEnabled` of `org.freedesktop.NetworkManager`). Sleep is detected from the gap between `CLOCK_BOOTTIME` and `CLOCK_MONOTONIC` between two ticks (both clocks advance together while awake; only `CLOCK_BOOTTIME` advances during suspend or hibernation). A system shutdown is recognised at `SIGTERM` by one bounded `Properties.Get` of logind's `PreparingForShutdown`. | The issue words tier 1 as "subscribes to `StateChanged`/`Connectivity`/`PrepareForSleep`/`PrepareForShutdown`". The companion's existing bus rules (invariant RMC-S9, `test_rmc_s9_bus_rules_match_the_presence_worker`, R3-2 parity with the daemon's presence worker) forbid `MessageStream`, `SignalStream`, `receive_signal`, `#[proxy`, `zbus::proxy` and `object_server` in every `crates/remote/src` file, and `test_rmc_s9_zbus_manifest_line_and_pinned_system_bus` allows exactly one `Builder::address(` call. Those are immutable contracts: tier 1 is therefore designed so that **no match rule is ever installed** (strictly stronger than the issue's "while off no bus match is installed"), no unsolicited message is ever queued, and RMC-S9 is unchanged. The observable facts are the same: a property read every 5 s sees every outage longer than the recording floor (D-6), the clock gap sees every suspend (no signal can be missed while the process is frozen), and the shutdown read happens exactly when it matters. Tier 2 (watchdog, delay inhibitor) needs real signals and will amend RMC-S9 in its own ADR. |
| D-2 | **No method call on NetworkManager other than `Properties.Get`, no `Inhibit`, no `Properties.Set`.** Destinations are only `org.freedesktop.NetworkManager` (path `/org/freedesktop/NetworkManager`) and `org.freedesktop.login1` (path `/org/freedesktop/login1`). | Issue acceptance line 1. |
| D-3 | **Reason enum** `OfflineReason { NetworkLost, WifiDisabled, Sleep, ShutdownOrCrash }` (serde `network_lost`, `wifi_disabled`, `sleep`, `shutdown_or_crash`). No other value is ever recorded. | Issue. |
| D-4 | **Classification** (pure `classify`): NM `State` 10/20/30/40 (asleep, disconnected, disconnecting, connecting) → offline; 50/60/70 (connected local/site/global) → offline only when `Connectivity` = 1 (`NONE`), otherwise online (`PORTAL` = 2 and `LIMITED` = 3 are **online**, as are `UNKNOWN` = 0 and `FULL` = 4); `State` 0 or any undocumented value → `Unknown` (no transition). An offline reading is `WifiDisabled` when `WirelessEnabled` is `false`, else `NetworkLost`. | Issue risk list: "`Connectivity=LIMITED/PORTAL` is not treated as offline". |
| D-5 | **Sleep intervals are recorded but never pushed.** A back-online push follows `NetworkLost`, `WifiDisabled` and `ShutdownOrCrash` intervals only. | A push after every lid close is alert fatigue; #359 tier 2 defines `alert_on_sleep` (default `false`) for the sleep case; tier 1 follows the same default without a new key (a key is added with tier 2). |
| D-6 | **Recording floor**: an interval shorter than `OFFLINE_MIN_INTERVAL_MS` (30 s) is dropped (Wi-Fi roaming, DHCP renewal, the few seconds NetworkManager needs after a resume, a fast service restart). | Issue risk list: false alarms. |
| D-7 | **Clean-stop marker** in `remote-network.json`: `run` is `running` while the service runs (written at start and refreshed with `last_alive_unix_ms` every `OFFLINE_HEARTBEAT_MS` = 60 s), `stopped` after a clean stop outside a system shutdown, `shutdown` after a clean stop while logind reports `PreparingForShutdown = true` **or the read failed** (fail-safe: an extra alert, never a missed one). At the next start, `running` or `shutdown` → one `ShutdownOrCrash` interval from `last_alive_unix_ms` to now (floor D-6 applies); `stopped` → none; absent file → none. | Issue: "a previous run that has no clean-stop marker counts as `shutdown_or_crash`". A user-level stop (logout, `systemctl --user restart` after an upgrade) is not an outage. |
| D-8 | **Default of `offline_alerts`**: the configuration-file default is `false`, exactly like `push_notifications` and `password_alerts` today; the one-command setup of #362 (a later PR) writes `offline_alerts = true` together with `push_notifications = true`, as decided by the owner on 2026-10-12 ("everything enabled by default, toggleable"). `scripts/install_remote.sh` documents the key in its commented template. | Consistency with the existing push keys; the setup owns defaults. |
| D-9 | **`offline_alerts = true` requires `push_notifications = true`: exit 78 otherwise** (`ConfigError::OfflineRequiresPush`, *revised in round 2, plan finding F-4*). This follows the acceptance line and the existing precedent (`push_notifications` → `password_alerts`/`rp_id`). There is no failure loop: `EXIT_CONFIG` is never restarted (`RestartPreventExitStatus=78`). The #362 setup writes both keys together, and its GUI toggle must turn `offline_alerts` off together with push (requirement recorded in the ADR). The degrade-with-warning alternative was rejected because it would change an acceptance line without the owner's approval. | Acceptance line 4 of #359 Part B tier 1. |
| D-10 | **Coalescing and hourly limits are shared** with the failed-password alerts: the existing `PushScheduler` gains a second pending slot (the back-online notice) that draws from the same `last_sent_ms` and the same rolling-hour deque (`PUSH_MIN_INTERVAL_MS` 30 s, `PUSH_MAX_PER_HOUR` 20). When both are due, the alert summary is sent first. Notices noted before the send merge (count, earliest start, latest end, newest reason and duration). | Issue acceptance line 3. |
| D-11 | **Fixed vocabulary, durations instead of clock times.** The notification body names the reason and a duration ("Offline for 12 min (network lost)"); clock times stay in the encrypted `soos` member and the web app renders them in the phone's time zone. | The PC has no knowledge of the phone's time zone and `std` has no time-zone database; a duration is unambiguous. The issue's "HH:MM" is an example text, not a contract. |
| D-12 | **Topic** `soosonline` (ASCII letters and digits, distinct from `soosalerts`/`soostest`): a back-online notice never replaces an undelivered failed-password alert on the push service. The service worker shows it with tag `soos-online`. | Web Push ADR item (8) topic rule. |
| D-13 | **"Network changed" push: not in this PR** (recorded as later). It needs `PrimaryConnection` → connection UUID reads, i.e. a second object class of NetworkManager whose properties include the connection id (the SSID by default); keeping the read set at three scalar properties is what makes the "no SSID anywhere" guarantee checkable by a static invariant. | Issue "optional". |
| D-14 | **Store failure is fail-closed for the feature, never for the service**: an insecure (symlink, owner, mode, not a regular file, oversize) or malformed `remote-network.json` at start → state `unavailable`, reason `store_failed`, one audit line, no bus call, no task, the file never rewritten (same rule as `remote-push.json`); the owner deletes it to recover. A write failure while running keeps the in-memory history (one `offline history not persisted` audit line per failure streak). | Same store rules as `remote-push.json`. |
| D-15 | **NetworkManager absent or stopped** (iwd-only or networkd hosts, NM stopped): before the NetworkManager reads of each tick, one `org.freedesktop.DBus.NameHasOwner("org.freedesktop.NetworkManager")` call goes to the bus driver, which never activates a service. `false` → reading `Unknown`, `network_monitor = "unavailable"`, and no call reaches NetworkManager, so a stopped-but-enabled NetworkManager is never started by bus activation (*revised in round 2, F-3*). Sleep and shutdown/crash tracking keep working. | Read-only must not mutate the host. |

## 1. Scope & blast radius

### 1.1 Files

| File | Change |
|---|---|
| `crates/remote/src/lib.rs` | `pub mod offline;`, constants §3, module doc paragraph |
| `crates/remote/src/offline.rs` (new) | types, pure classifier and tracker, store, `NetworkSource` seam + `ZbusNetworkSource`, `OfflineClock` seam + `SystemOfflineClock`, runtime, watcher task, view |
| `crates/remote/src/logind.rs` | the connection builder moves into `pub(crate) async fn connect_system_bus()` (still the only `Builder::address(` call of the crate, still in `logind.rs`); `ZbusSessionSource::connection` calls it. No other change |
| `crates/remote/src/push.rs` | `OfflineNotice` delivery: scheduler second slot, `back_online_payload`, dispatcher branch, `PushRuntime::offline_sink`, retries of notices |
| `crates/remote/src/config.rs` | `OfflineConfig`, key `offline_alerts`, `RemoteConfig::offline`, `resolve_network_store_path` |
| `crates/remote/src/routes.rs` | `Route::Offline`, `OFFLINE_PATH` (`GET|HEAD /api/offline`) |
| `crates/remote/src/server.rs` | `ServerState::with_offline_alerts`, wiring in `serve` (start, supervised watcher, clean-stop marker on clean shutdown), handler |
| `crates/remote/src/audit.rs` | three fixed audit lines |
| `crates/remote/src/main.rs` | wiring (`offline_wiring`), the D-9 warning |
| `crates/remote/assets/index.html`, `app.js`, `style.css` | "Offline history" card |
| `crates/remote/assets/sw.js` | tag `soos-online` for `kind == "back_online"` |
| `scripts/install_remote.sh` | commented `# offline_alerts = false` template line with its explanation |
| Tests | §10 |
| Docs | §11 |

`soos-daemon`, `pam_soos.so`, the IPC protocol, `soos-push-sender`, `soos-push-protocol`,
`packaging/soos-remote.service` (`RestrictAddressFamilies=AF_UNIX`: the system bus is a Unix
socket) and every manifest are unchanged. No new dependency (`nix` already has `time`).

### 1.2 Consumers of changed public items

- `RemoteConfig` gains `pub offline: OfflineConfig`: every struct literal in the tests gains
  `offline: OfflineConfig::default(),` (setup-only migration, §9).
- `PushScheduler` gains methods; `take`, `due_at`, `note` keep their exact semantics when no
  notice is pending (existing scheduler tests unchanged).
- No other crate depends on `soos-remote` (RMC-S4).

### 1.3 Out of scope (later PRs, each with its own ADR where needed)

Tier 2 watchdog (signals, delay inhibitor, sealed envelopes), tier 3 Android heartbeat,
"network changed" push (D-13), `alert_on_sleep`, any SSID/BSSID/IP/MAC/location, any
NetworkManager write, the #362 setup default.

## 2. Architecture overview

```
SystemOfflineClock ─┐                      ┌─ ZbusNetworkSource (Properties.Get ×3 / tick,
(unix, boottime,    │   run_watcher (task) │   PreparingForShutdown at stop)
 monotonic)         └─► OfflineTracker ◄───┘
                          │ recorded interval → OfflineRuntime.history (Mutex) → GET /api/offline
                          │ heartbeat / change → NetworkStore (remote-network.json, blocking pool)
                          └ released notice → OfflineNoticeSink → PushScheduler slot 2
                                                                   → run_dispatcher → soos-push-sender
```

The tracker is pure (no clock, no I/O): it receives `ClockSample`s and `NetworkObservation`s
and returns what happened. The runtime owns the store, the history shown by the view and the
sink; the watcher task is the only caller of the source during the run.

## 3. Constants (`crates/remote/src/lib.rs`, new section "Offline history (ADR 2026-10-13 …)")

| Name | Type | Value | Meaning / bound |
|---|---|---|---|
| `NETWORK_STORE_FILE_NAME` | `&str` | `"remote-network.json"` | sibling of the credential store |
| `MAX_NETWORK_STORE_BYTES` | `usize` | `8192` | larger file → `store_failed` (never read past `+1`) |
| `MAX_OFFLINE_INTERVALS` | `usize` | `32` | recorded intervals kept; the oldest is evicted |
| `OFFLINE_TICK_MS` | `u64` | `5000` | watcher period (property reads and clock samples) |
| `OFFLINE_MIN_INTERVAL_MS` | `u64` | `30_000` | recording floor (D-6), also the sleep-gap threshold |
| `OFFLINE_HEARTBEAT_MS` | `u64` | `60_000` | at most one `last_alive_unix_ms` write per this interval |
| `NETWORK_READ_DEADLINE_MS` | `u64` | `1500` | bound of one whole network observation (3 calls) |
| `OFFLINE_STOP_DEADLINE_MS` | `u64` | `1000` | bound of the stop step (shutdown read + marker write) |
| `PUSH_OFFLINE_TOPIC` | `&str` | `"soosonline"` | `Topic` of a back-online notice |

Compile-time relations: `NETWORK_READ_DEADLINE_MS < OFFLINE_TICK_MS`;
`OFFLINE_TICK_MS < OFFLINE_MIN_INTERVAL_MS`; `OFFLINE_TICK_MS <= OFFLINE_HEARTBEAT_MS`;
`DBUS_CALL_TIMEOUT_MS * 3 <= NETWORK_READ_DEADLINE_MS`; `DBUS_CALL_TIMEOUT_MS < OFFLINE_STOP_DEADLINE_MS`;
`MAX_OFFLINE_INTERVALS * 192 + 256 <= MAX_NETWORK_STORE_BYTES` (192 bytes ≥ the largest
serialized interval with four 20-digit numbers); `PUSH_OFFLINE_TOPIC` alphanumeric, ≤
`MAX_TOPIC_LEN`, different from `PUSH_TOPIC` and `PUSH_TEST_TOPIC`.

Sentinels: no constant is configurable; `0` never occurs.

## 4. Configuration (`config.rs`)

```rust
/// Offline history and back-online push settings; `Default` = off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OfflineConfig {
    /// `offline_alerts`; default `false`. Effective only with `push_notifications = true`
    /// (D-9); see [`OfflineConfig::effective`].
    pub enabled: bool,
}
impl OfflineConfig {
    /// `enabled && push.enabled`.
    #[must_use] pub fn effective(&self, push: &PushConfig) -> bool;
}
```

- `FileConfig` gains `offline_alerts: Option<bool>`; any other TOML type → `ConfigError::Syntax`
  (serde); absent → `false`. No new `ConfigError` variant (D-9).
- `pub fn resolve_network_store_path(credentials_path: &Path) -> Result<PathBuf, ConfigError>`:
  `<parent of credentials_path>/NETWORK_STORE_FILE_NAME`, same rules and error as
  `resolve_push_store_path`.

## 5. Module `crates/remote/src/offline.rs`

The module contains no `tracing` macro (audit lines go through `crate::audit`), no
`unwrap`/`expect`/indexing, no `String` field in any type that crosses into `push.rs`.

### 5.1 Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineReason { NetworkLost, WifiDisabled, Sleep, ShutdownOrCrash }
impl OfflineReason {
    /// `false` for `Sleep` only (D-5).
    #[must_use] pub const fn pushes(self) -> bool;
}

/// One recorded interval (wall times in Unix ms, duration from CLOCK_BOOTTIME when the
/// interval lies in one boot, else from the wall clock, saturating).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineInterval {
    pub reason: OfflineReason,
    pub start_unix_ms: u64,
    pub end_unix_ms: u64,
    pub duration_ms: u64,
}

/// What a back-online push says: enums and times only (the payload builder's whole input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfflineNotice {
    /// Reason of the newest merged interval.
    pub reason: OfflineReason,
    /// Merged intervals, saturating, ≥ 1.
    pub intervals: u32,
    /// Earliest start of the merged intervals.
    pub first_start_unix_ms: u64,
    /// Latest end of the merged intervals.
    pub last_end_unix_ms: u64,
    /// Duration of the newest merged interval.
    pub last_duration_ms: u64,
}
impl OfflineNotice {
    #[must_use] pub fn of(interval: &OfflineInterval) -> Self;
    /// Pure fold: counts add (saturating), span widens, reason and duration from the
    /// operand with the greater `last_end_unix_ms` (ties: `newer`).
    #[must_use] pub fn merge(older: &Self, newer: &Self) -> Self;
}

/// One clock sample: wall (Unix ms), CLOCK_BOOTTIME (ms) and CLOCK_MONOTONIC (ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample { pub unix_ms: u64, pub boot_ms: u64, pub mono_ms: u64 }

/// Raw NetworkManager reading (three scalar properties, never a string).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NmReading { pub state: u32, pub connectivity: u32, pub wireless_enabled: Option<bool> }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkObservation { Online, Offline(OfflineReason), Unknown }

/// `classify` of D-4; `Offline` carries only `NetworkLost` or `WifiDisabled`.
#[must_use] pub fn classify(reading: &NmReading) -> NetworkObservation;

/// `run` marker of the store (D-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMarker { Running, Stopped, Shutdown }
```

`NmReading.wireless_enabled` is `None` when that one property could not be read (an offline
reading is then `NetworkLost`).

### 5.2 Pure functions and the tracker

```rust
/// D-7: `Running` or `Shutdown` → one ShutdownOrCrash interval [last_alive, now] (wall clock,
/// duration = now − last_alive saturating) when ≥ OFFLINE_MIN_INTERVAL_MS; `Stopped` → None.
#[must_use] pub fn previous_run_interval(run: RunMarker, last_alive_unix_ms: u64, now_unix_ms: u64)
    -> Option<OfflineInterval>;

/// Body duration label: < 60 s → "less than a minute"; < 60 min → "N min";
/// < 48 h → "H h" or "H h M min"; otherwise "D days". Pure, never panics (u64::MAX included).
#[must_use] pub fn duration_label(ms: u64) -> String;

/// What one step produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickOutcome {
    /// Intervals recorded by this step (0..=2: a closed network interval and a sleep).
    pub recorded: Vec<OfflineInterval>,
    /// A notice released by this step (merged pending notices; only when the step's
    /// observation is not `Offline`).
    pub notice: Option<OfflineNotice>,
}

pub struct OfflineTracker { /* open interval, last sample, history (VecDeque, ≤ 32), pending notice */ }
impl OfflineTracker {
    /// History loaded from the store (oldest first; more than MAX_OFFLINE_INTERVALS keeps
    /// the newest MAX_OFFLINE_INTERVALS).
    #[must_use] pub fn new(history: Vec<OfflineInterval>) -> Self;
    /// Start step: records `previous` (from `previous_run_interval`) and, when it pushes,
    /// holds its notice until the first non-`Offline` observation.
    pub fn start(&mut self, previous: Option<OfflineInterval>, now: ClockSample) -> TickOutcome;
    /// One watcher step (rules below).
    pub fn tick(&mut self, now: ClockSample, observation: NetworkObservation) -> TickOutcome;
    /// Clean stop: closes an open network interval at `now` (recorded when ≥ floor, its
    /// notice dropped: the PC is not back online). Returns what was recorded.
    pub fn stop(&mut self, now: ClockSample) -> TickOutcome;
    /// Newest first, at most MAX_OFFLINE_INTERVALS.
    #[must_use] pub fn history_newest_first(&self) -> Vec<OfflineInterval>;
    /// Oldest first (store order).
    #[must_use] pub fn history(&self) -> Vec<OfflineInterval>;
}
```

`tick` rules, in order (all arithmetic saturating; samples whose clocks go backwards give gap 0):

1. **Sleep gap** (needs a previous sample `p`): `slept = (now.boot − p.boot) − (now.mono − p.mono)`.
   If `slept ≥ OFFLINE_MIN_INTERVAL_MS`: (a) an open network interval is closed at `p`
   (`end = p.unix_ms`, `duration = p.boot − open.start_boot`), recorded when ≥ floor, its
   notice pending; (b) a `Sleep` interval `[now.unix − slept, now.unix]`, `duration = slept`,
   is recorded (no notice, D-5).
2. **Observation**: `Offline(r)` and nothing open → open `{r, now.unix, now.boot}`;
   `Offline(WifiDisabled)` while a `NetworkLost` interval is open → the open reason becomes
   `WifiDisabled`; `Online` and an interval open → closed at `now`
   (`duration = now.boot − start_boot`), recorded when ≥ floor with a pending notice;
   `Unknown` → no change.
3. **Release**: when the observation is not `Offline`, the merged pending notice (if any) is
   returned in `notice` and cleared.
4. `p := now`. Recording appends to the history and evicts the oldest beyond
   `MAX_OFFLINE_INTERVALS`.

### 5.3 Store (`remote-network.json`)

```json
{"version":1,"run":"running","last_alive_unix_ms":1760000000000,
 "intervals":[{"reason":"network_lost","start_unix_ms":…,"end_unix_ms":…,"duration_ms":…}]}
```

```rust
pub struct NetworkStoreFile { pub run: RunMarker, pub last_alive_unix_ms: u64, pub intervals: Vec<OfflineInterval> }
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NetworkStoreError { Refused, Malformed, Io, Locked }   // fixed texts
#[derive(Clone)] pub struct NetworkStore { path: PathBuf, owner_uid: u32 }   // manual Debug
impl NetworkStore {
    #[must_use] pub fn new(path: PathBuf, owner_uid: u32) -> Self;
    /// Absent → Ok(None). Present → read_owned_file_with_capacity (O_NOFOLLOW, regular file,
    /// owner, no group/other bits, ≤ MAX_NETWORK_STORE_BYTES), then strict JSON
    /// (deny_unknown_fields, version 1, ≤ MAX_OFFLINE_INTERVALS intervals,
    /// start ≤ end for every interval).
    pub fn load(&self) -> Result<Option<NetworkStoreFile>, NetworkStoreError>;
    /// try_lock_file (flock on `<path>.lock`, Locked when held) → check_parent_dir →
    /// serialize (> MAX_NETWORK_STORE_BYTES → Io, nothing written) → write_atomic
    /// (temp 0600 create_new O_NOFOLLOW, fsync, rename, directory fsync).
    pub fn save(&self, file: &NetworkStoreFile, random: &RandomSource) -> Result<(), NetworkStoreError>;
}
```

Errors map like the push store (`Insecure`/`TooLarge` → `Refused`, `Malformed` → `Malformed`,
`Busy` → `Locked`, others → `Io`). Runtime calls run on the blocking pool; a `Locked` save is
retried every `STORE_LOCK_RETRY_MS` for at most `STORE_LOCK_TIMEOUT_MS`.

### 5.4 Seams

```rust
/// The fake-bus seam.
pub trait NetworkSource: Send + Sync + 'static {
    /// `State` and `Connectivity` (required) and `WirelessEnabled` (optional) of
    /// NetworkManager, each one `Properties.Get`.
    fn observe(&self) -> BoxFuture<'_, Result<NmReading, SourceError>>;
    /// logind `Manager.PreparingForShutdown` through one `Properties.Get`.
    fn preparing_for_shutdown(&self) -> BoxFuture<'_, Result<bool, SourceError>>;
}
/// The fake-clock seam.
pub trait OfflineClock: Send + Sync + 'static { fn sample(&self) -> ClockSample; }

#[derive(Debug, Default)] pub struct ZbusNetworkSource { /* Mutex<Option<Connection>> */ }
#[derive(Debug, Default, Clone, Copy)] pub struct SystemOfflineClock;
```

`ZbusNetworkSource` opens its connection through `crate::logind::connect_system_bus()` (pinned
address, `max_queued`, method timeout); every call is `connection.call_method(Some(dest), path,
Some("org.freedesktop.DBus.Properties"), "Get", &(iface, name))` under
`DBUS_CALL_TIMEOUT_MS`; the reply is decoded as `OwnedValue` then `u32`/`bool`; a transport
error, timeout or shape mismatch drops the connection (reopened at the next call); a method
error maps to `SourceError::Call`. The literal property names are exactly `"State"`,
`"Connectivity"`, `"WirelessEnabled"`, `"PreparingForShutdown"`. `SystemOfflineClock` uses
`SystemTime` and `nix::time::clock_gettime` (`CLOCK_BOOTTIME`, `CLOCK_MONOTONIC`); a failed
clock read gives `0` (the gap rule then sees no sleep).

### 5.5 Runtime and view

```rust
#[derive(Debug, Clone)] pub struct OfflineSettings { pub store_path: PathBuf }
pub(crate) type OfflineNoticeSink = Arc<dyn Fn(OfflineNotice) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "snake_case")]
pub enum OfflineState { Disabled, Active, Unavailable }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "snake_case")]
pub enum OfflineUnavailableReason { PushDisabled, StoreFailed }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "snake_case")]
pub enum NetworkMonitor { Unknown, Active, Unavailable }

/// JSON of `GET /api/offline` (exactly four keys).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfflineView {
    pub state: OfflineState,
    pub reason: Option<OfflineUnavailableReason>,   // Some iff Unavailable
    pub network_monitor: NetworkMonitor,            // Unknown unless Active
    pub intervals: Vec<OfflineInterval>,            // newest first, ≤ 32, empty unless Active
}
impl OfflineView {
    #[must_use] pub fn disabled() -> Self;
    #[must_use] pub fn push_disabled() -> Self;
}
```

`OfflineRuntime` (crate-private) holds the store, the tracker behind a `std::sync::Mutex` (never
held across an `.await`), the monitor state, the sink and a `persisted_failing` flag:

- `start(clock)`: on the blocking pool, `load`; `Err` → `Unavailable/StoreFailed`, audit
  `offline alerts unavailable`, no task, file untouched. `Ok(prev)` → `tracker.start(
  previous_run_interval(..))`, then save `{run: Running, last_alive: now, intervals}` (this is
  the only place the file is created); audit `offline alerts active`.
- `run_watcher(runtime, source, clock)`: forever: sample → `timeout(NETWORK_READ_DEADLINE_MS,
  source.observe())` (error/timeout → `Unknown`, monitor `Unavailable`; ok → `classify`,
  monitor `Active`) → `tracker.tick` → a released notice goes to the sink (dropped when no
  sink) → save when something was recorded or `OFFLINE_HEARTBEAT_MS` elapsed since the last
  save (monotonic) → `sleep(OFFLINE_TICK_MS)`. No error ends it.
- `stop(source, clock)` (clean shutdown only, whole step under `OFFLINE_STOP_DEADLINE_MS`):
  `preparing_for_shutdown` (bounded by `DBUS_CALL_TIMEOUT_MS`): `Ok(false)` → `Stopped`;
  `Ok(true)`, error or timeout → `Shutdown`; `tracker.stop(now)`; save with
  `last_alive_unix_ms = now`. A save that does not finish in time leaves `running` (next
  start: `shutdown_or_crash`, fail-safe).
- `view()`: `Active` → history newest first + monitor.

### 5.6 Payload (`push.rs`)

```rust
/// Declarative Web Push JSON of a back-online notice (≤ MAX_PUSH_PLAINTEXT_BYTES); its only
/// inputs are the notice (enums and times), the previews setting and the configured rp_id.
pub fn back_online_payload(notice: &OfflineNotice, previews: PushPreviews, rp_id: &str)
    -> Result<Vec<u8>, WebPushError>;
```

| previews | title | body | `soos` member |
|---|---|---|---|
| `detailed`, 1 interval | `Your PC is back online` | `Offline for <duration_label(last_duration_ms)> (<label>)` | `{v:1, kind:"back_online", reason, intervals, first_start_unix_ms, last_end_unix_ms}` |
| `detailed`, n > 1 | `Your PC is back online` | `Offline <n> times, latest for <duration> (<label>)` | same |
| `generic` | `Update from your PC` | `Open soos for details` | `reason: null`, counts and times kept |

Labels: `network_lost` → `network lost`, `wifi_disabled` → `Wi-Fi turned off`, `sleep` → `sleep`,
`shutdown_or_crash` → `shutdown or crash`. `navigate` = `https://<rp_id>/`, `lang` `en`,
`web_push` 8030. Request: `TTL` `PUSH_TTL_S`, `Urgency` `PUSH_URGENCY`, `Topic`
`PUSH_OFFLINE_TOPIC`.

### 5.7 Scheduler and dispatcher (`push.rs`)

```rust
impl PushScheduler {
    /// Notes a back-online notice at monotonic `now_ms` (merged into a pending one).
    pub fn note_offline(&mut self, notice: OfflineNotice, now_ms: u64);
    /// When the pending notice may be sent (`first_noted + PUSH_COALESCE_MS`, then the shared
    /// spacing and hourly cap); `None` without one.
    #[must_use] pub fn offline_due_at(&self) -> Option<u64>;
    /// The next due message at `now_ms` (alert summary first), recording the send in the
    /// shared budget.
    pub fn take_due(&mut self, now_ms: u64) -> Option<DueMessage>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueMessage { Alert(PushSummary), BackOnline(OfflineNotice) }
```

`take` and `due_at` keep their exact current meaning (alert summary only), and `take` records
its send in the same budget. `PushRuntime::offline_sink()` returns the sink handed to the
offline runtime (ignores notices until the push store is active; notes and wakes the
dispatcher; no I/O, no await). The dispatcher calls `take_due`; a back-online notice is sent to
every subscription sequentially with the outcome table of the alerts except carries: delivered
→ `delivered`; `Gone` → removal; rejected/refused → `rejected`, dropped; retryable/transport
error → retried with `PUSH_RETRY_DELAYS_MS` (or `Retry-After`) per subscription, then
`failed`, dropped; a newer due notice for a subscription with a pending notice retry cancels
that retry and is merged with it. Lock order unchanged (scheduler mutex alone).

## 6. HTTP surface

- `GET|HEAD /api/offline` → `Route::Offline` (`OFFLINE_PATH`), not Funnel-public (a Funnel
  caller needs a web session: `403 login_required` before any work), no CSRF header (read-only),
  no body. Answer `200 application/json` `OfflineView`:
  runtime present → `view()`; else `config.offline.enabled && !config.push.enabled` →
  `push_disabled()`; else `disabled()`. Serialization failure → `503 unavailable`.
- No new SSE event; the page fetches the view (§8).

## 7. Security invariants (new, `tests/invariants/src/remote_offline_contract.rs`)

- **ROA-S1 (RC-1)**: no `crates/remote/src` file names `AF_INET`, `AF_INET6`, `AddressFamily::Inet`,
  `nix::sys::socket`, `socket::socket(`, `TcpStream`, `UdpSocket`, `std::net` (a bare `socket(`
  would match the existing `is_socket(` checks); `packaging/soos-remote.service` keeps exactly
  `RestrictAddressFamilies=AF_UNIX`.
- **ROA-S2 (read-only bus)**: `offline.rs` uses only the method literal `"Get"` and the
  interface `org.freedesktop.DBus.Properties`; no remote source names a NetworkManager or
  logind mutating method (`"Enable"`, `"Sleep"`, `"ActivateConnection"`, `"AddAndActivateConnection"`,
  `"DeactivateConnection"`, `"Reload"`, `"SetLogging"`, `"CheckpointCreate"`, `"Update"`, `"Delete"`,
  `"Set"`, `"RequestScan"`, `"Disconnect"`, `"Inhibit"`, `"AddMatch"`); the four property literals are
  exactly the ones of §5.4; the single `Builder::address(` stays in `logind.rs` (RMC-S9 unchanged).
- **ROA-S3 (privacy)**: no remote source or asset names (case-insensitively) `ssid`, `bssid`,
  `hwaddress`, `ip4config`, `ip6config`, `accesspoint`, `activeconnection`, `primaryconnection`,
  `latitude`, `longitude`; `offline.rs` and the page's offline code additionally never name
  `hostname`, `address` or `geolocation` (`http.rs` legitimately names `geolocation=()` in its
  `Permissions-Policy`); `offline.rs` and `push.rs` offline paths have no `tracing`
  macro; `OfflineNotice` fields are only `OfflineReason`, `u32`, `u64` (struct scan) and the type
  is `Copy`.
- **ROA-S4**: page card uses `setText` only, fixed strings, the CSP and the 11-file asset set
  unchanged (RXB-S8 still green), `sw.js` tag `soos-online`.
- **ROA-S5**: docs, ADR, matrix rows and walkthrough 202 exist.

## 8. Page

- `index.html`: `<section id="offline" class="card offline" hidden aria-live="polite">` with
  `<h2>Offline history</h2>`, `<p id="offline-summary" class="detail">`,
  `<p id="offline-monitor" class="banner banner-warn" hidden>`, `<ul id="offline-history" class="offline-history table">`.
- `app.js`: `const OFFLINE_PATH = "/api/offline";`, `const OFFLINE_HISTORY_SHOWN = 10;`,
  `const OFFLINE_REASONS = {network_lost: "Network lost", wifi_disabled: "Wi-Fi turned off",
  sleep: "Sleep", shutdown_or_crash: "Shutdown or crash"}`; `fetchOffline()` (called next to
  `fetchAlerts()` after a successful status fetch and once at start); `renderOffline(view)`:
  `disabled` → card hidden; `unavailable` → "Offline alerts unavailable: turn on push
  notifications on the PC" (`push_disabled`) or "Offline alerts unavailable: the history file
  on the PC was refused" (`store_failed`); `active` → "No offline period recorded" or
  "<n> offline periods recorded"; `network_monitor === "unavailable"` → banner "Network changes
  are not monitored on this PC (NetworkManager not found); sleep and shutdown are". Each row:
  `<start date-time> – <end time> · <duration> · <reason label>` via `setText`. Unknown keys →
  "Other". No storage, no new request type beyond `fetch(GET)`, no template literal, no
  `console.`, no `.message`.
- `style.css`: `.offline-history` reuses the `.table` list rules; no `:has()` without a fallback.
- `sw.js`: `kind === "back_online"` → `tag = "soos-online"`.

## 9. Existing tests that need migration (setup only; no assertion touched)

`RemoteConfig` struct literals gain `offline: soos_remote::config::OfflineConfig::default(),`
in: `crates/remote/tests/common/harness.rs`, `crates/remote/tests/common/camera.rs`,
`crates/remote/tests/server_tests.rs`, `crates/remote/tests/alerts_server_tests.rs`,
`crates/remote/tests/push_server_tests.rs`, `crates/remote/tests/battery_server_tests.rs`,
`crates/remote/tests/android_push_tests.rs`. Precedent: Web Push ADR item (10), battery ADR.

## 10. Test list (Phase 2, Red first)

### 10.1 `crates/remote/tests/offline_tests.rs` (pure, tempdir, fake clock)

1. `test_roa_classify_states_and_connectivity` — full table of D-4 (PORTAL/LIMITED online;
   NONE offline; undocumented state → Unknown; `WirelessEnabled` false/None).
2. `test_roa_tracker_records_network_interval_with_boottime_duration`.
3. `test_roa_tracker_drops_intervals_below_floor` (29 999 vs 30 000 ms).
4. `test_roa_tracker_wifi_disabled_upgrades_open_interval`.
5. `test_roa_tracker_detects_sleep_from_clock_gap` (sleep recorded, no notice, open network
   interval closed at the pre-sleep sample).
6. `test_roa_tracker_releases_notice_only_when_not_offline` (Unknown releases, Offline holds).
7. `test_roa_tracker_merges_pending_notices`.
8. `test_roa_tracker_history_is_bounded_to_32` (oldest evicted; `new` with 40 keeps newest 32).
9. `test_roa_previous_run_classification` (running/shutdown/stopped, floor, clock backwards).
10. `test_roa_tracker_stop_closes_open_interval_without_notice`.
11. `test_roa_duration_label_vocabulary` (boundaries 59 999, 60 000, 3 599 999, 3 600 000, 172 799 999, `u64::MAX`).
12. `test_roa_notice_is_copy_and_merge_rules`.
13. `test_roa_store_round_trip_and_mode_0600`.
14. `test_roa_store_refuses_symlink_foreign_mode_oversize_and_malformed` (incl. 8193 bytes,
    33 intervals, unknown key, version 2, start > end, never rewritten).
15. `test_roa_store_max_file_fits_bound` (32 intervals of `u64::MAX` values serialize ≤ 8 KiB).
16. `test_roa_store_save_is_locked_and_atomic` (held `.lock` → `Locked`, no temp left).

### 10.2 `crates/remote/tests/config_tests.rs` (new test only)

17. `test_roa_offline_alerts_key` (default false, true/false, wrong type → Syntax, true with
    push off accepted, `effective`, `resolve_network_store_path`).

### 10.3 `crates/remote/tests/routes_tests.rs` (new test only)

18. `test_roa_offline_route_table` (GET/HEAD → `Offline`, POST → 405 `GET, HEAD`, not Funnel-public).

### 10.4 `crates/remote/tests/push_tests.rs` (new tests only)

19. `test_roa_back_online_payload_detailed_and_generic` (exact JSON, labels, plural form,
    ≤ 1 KiB, generic has `reason: null` and no reason word in title/body).
20. `test_roa_scheduler_shares_spacing_and_hourly_cap` (alert then notice spaced 30 s; 20 per
    hour across both kinds; alert first when both due; `take`/`due_at` unchanged without notice).
21. `test_roa_scheduler_merges_notices`.

### 10.5 `crates/remote/tests/offline_server_tests.rs` (served, fake bus, fake clock, `FakeTransport`)

22. `test_roa_off_installs_nothing` (offline off: zero source calls, no file, view `disabled`).
23. `test_roa_push_off_degrades` (offline on, push off: service serves, zero source calls,
    no file, view `unavailable/push_disabled`).
24. `test_roa_network_outage_sends_one_back_online_push` (decrypted payload, topic `soosonline`,
    previews detailed and generic).
25. `test_roa_unclean_previous_stop_is_shutdown_or_crash` (store with `running`, push after the
    first online reading).
26. `test_roa_clean_stop_writes_marker_and_shutdown_read` (`false` → `stopped`; `true`/error → `shutdown`).
27. `test_roa_store_failed_is_unavailable_and_never_rewritten`.
28. `test_roa_view_over_funnel_requires_session`.
29. `test_roa_network_monitor_unavailable_without_networkmanager`.
30. `test_roa_alert_and_back_online_share_the_budget` (served, both kinds, 30 s spacing).

### 10.6 `tests/invariants/src/remote_offline_contract.rs` (new; `mod` in `lib.rs`)

31. `test_roa_s1_no_inet_socket_and_unit_unchanged`.
32. `test_roa_s2_bus_is_read_only`.
33. `test_roa_s3_no_network_identifiers_and_notice_fields`.
34. `test_roa_s4_page_card_and_service_worker`.
35. `test_roa_s5_docs_adr_matrix_walkthrough`.

## 11. Documentation (Phase 6)

`Docs/REMOTE_COMPANION.md`: new section "2j. Offline history and back-online notifications
(opt-in)"; §5 key row `offline_alerts`; §6 route row; §8 limitations bullet; §9 updated
("monitoring lost" real-time notifications stay out of scope until tier 2; network-changed push
later). `AI/DECISIONS.md` ADR (appended). `AI/VERIFICATION_MATRIX.md` section with ROA1…
(appended), owner hardware rows. Walkthrough `AI/walkthroughs/202_remote_offline_alerts.md`.
`.claude/skills/dev-workflow/references/project-facts.md` constants row.
`AI/ARCHITECTURE.md` companion paragraph (one sentence).

## 12. ADR draft (appended to `AI/DECISIONS.md`)

"[2026-10-13] Offline History and Back-Online Push Notifications in `soos-remote` (GitHub #376,
tier 1 of #359 Part B) — *Amends item (1) of the 2026-10-06 Web Push ADR*: besides live
failed-password alerts and the test notification, Web Push may carry one fixed-vocabulary
back-online notice after a recorded `network_lost`, `wifi_disabled` or `shutdown_or_crash`
interval." Items: scope (D-3..D-6, D-11, D-12), read-only bus by polling (D-1, D-2, D-15),
clean-stop marker (D-7), store (D-14), defaults and degradation (D-8, D-9), shared budget
(D-10), privacy (no SSID/BSSID/IP/MAC/location; D-13), accepted risks (§13), evidence.

## 13. Accepted risks / limitations

- An outage shorter than 30 s, or one that starts and ends between two ticks, is not recorded.
- Times are as good as the PC's wall clock (an NTP correction after reconnecting can shift a
  start); durations of in-boot intervals come from `CLOCK_BOOTTIME` and are not affected.
- `shutdown_or_crash` starts at the last heartbeat (up to 60 s early) after a crash or power
  loss; a crash of `soos-remote` itself longer than 30 s (including systemd's 5 s restart delay
  plus the start) is reported as `shutdown_or_crash`, which it is (monitoring was lost).
- A back-online notice not yet sent when the service stops is lost (the history keeps the
  interval).
- On a host without NetworkManager only sleep and shutdown/crash are recorded (the page says so).
- A wired PC with the Wi-Fi radio switched off reports an unplugged cable as `wifi_disabled`.
- A process of the owner's uid can edit `remote-network.json` (forged history, bounded to 32
  entries and 8 KiB, false notices bounded by the shared 20-per-hour budget).
- The notice is after the fact: a thief who keeps the PC offline is never reported in real time
  (tier 2/3).

## 14. Acceptance criteria → spec → tests → matrix

| #359 Part B tier 1 line | Spec | Tests | Matrix |
|---|---|---|---|
| Read-only NM + logind, no NM method, static RC-1 + no-mutating-call invariant | D-1, D-2, §5.4, §7 | 31, 32, 22 | ROA1, ROA2 |
| Bounded ≤ 32, monotonic + wall, reason enum | §5.1, §5.2 | 1–8, 10 | ROA3 |
| `remote-network.json` 0600, ≤ 8 KiB, O_NOFOLLOW, owner-checked, flock + temp + fsync + rename | §5.3 | 13–16, 27 | ROA4 |
| Unclean previous stop → `shutdown_or_crash` (fake clock, fake bus) | D-7, §5.2, §5.5 | 9, 25, 26 | ROA5 |
| Fixed vocabulary, builder takes only enums and times | §5.6, D-11 | 12, 19, 33 | ROA6 |
| `push_previews = "generic"` honoured | §5.6 | 19, 24 | ROA6 |
| Coalescing and hourly limit shared | D-10, §5.7 | 20, 21, 30 | ROA7 |
| `offline_alerts` key, default, push dependency; off → no match, no file | D-8, D-9, §4 | 17, 22, 23 | ROA8 |
| Web app "Offline history" card | §8 | 34 | ROA9 |
| Privacy (no SSID/BSSID/IP/MAC/location) | §7 | 33, 24 | ROA10 |
| Docs, ADR, walkthrough | §11, §12 | 35 | ROA11 |
| Owner hardware: Wi-Fi off/on → push; suspend/resume → `sleep` row, no push; hard power-off → `shutdown_or_crash` + push | — | — | ROA12–ROA14 (⬜ Pending owner hardware) |

## 15. Revision 2 (plan evaluation round 1, `AI/plan_evaluator_report.md`): authoritative where it differs from §0–§14

- **F-1 (clock)**: `SystemOfflineClock::new(unix: UnixClock)` takes the wall time from an injected
  Unix-ms closure. `main.rs` (where `SystemTime::now` is allowed by R3-1) passes the same kind of
  closure it gives the server, and tests inject their own `Arc<dyn OfflineClock>` through
  `ServerState::with_offline_alerts(settings, source, clock)`. `offline.rs` reads only
  `CLOCK_BOOTTIME` and `CLOCK_MONOTONIC` through `nix::time::clock_gettime`, never `SystemTime` and
  never `CLOCK_REALTIME`, so invariant R3-1 holds.
- **F-5 (invalid clock)**: `OfflineClock::sample(&self) -> Option<ClockSample>`. `None` (a failed
  `clock_gettime`) skips the whole step (no read, no tick); the next valid sample is compared with the
  last valid one, so no spurious sleep is ever computed from a `0`.
- **F-2 (release)**: a held notice is released only by an `Online` observation. When NetworkManager has
  never been observed during this run (every step so far `Unknown`), a held notice is released after
  `OFFLINE_UNMONITORED_RELEASE_TICKS` (12, i.e. 60 s) consecutive `Unknown` steps (hosts without NM).
  An `Unknown` step on a host where NM was observed releases nothing. Delivery of a notice: per
  subscription, retryable outcomes and transport errors are retried with `OFFLINE_RETRY_DELAYS_MS` =
  `[5 000, 30 000, 120 000, 600 000]` (`Retry-After` honoured when given). After the last attempt the
  notice is dropped (`last_delivery = failed`), so there are at most 5 attempts over about 12.6 min.
  `Rejected`/`Refused` → dropped (`rejected`), `Gone` → removal. A newer due notice for a subscription
  with a pending notice retry cancels that retry and is merged with it.
- **F-3**: see D-15. ROA-S2 allows exactly one bus-driver call (destination `org.freedesktop.DBus`, path
  `/org/freedesktop/DBus`, interface `org.freedesktop.DBus`, member `"NameHasOwner"`) next to the
  `Properties.Get` calls. `offline.rs` therefore has two call sites (`call_method(` count 2).
- **F-4**: see D-9. `ConfigError::OfflineRequiresPush` ("offline_alerts requires push_notifications"),
  checked after the push keys. `OfflineConfig { enabled }` only (no `effective`); `with_offline_alerts`
  additionally ignores the wiring unless `config.offline.enabled && config.push.enabled` (defensive).
  `OfflineUnavailableReason` has the single variant `StoreFailed`.
- **F-6 (budgets)**: `NETWORK_READ_DEADLINE_MS` = 3000 (≥ `DBUS_CONNECT_TIMEOUT_MS` + 4 ×
  `DBUS_CALL_TIMEOUT_MS`; < `OFFLINE_TICK_MS`); the whole observation runs under it and an overrun is
  `Unknown`. Stop step: it runs after `supervised.abort_all()` and before `serve` returns, with the
  shutdown read under `OFFLINE_SHUTDOWN_READ_MS` = 1500 (connect + call; overrun or error → `Shutdown`)
  and the save under the remaining budget of `OFFLINE_STOP_DEADLINE_MS` = 3000 (lock retries up to
  `STORE_LOCK_TIMEOUT_MS` included). Every save goes through one async mutex held across the
  blocking-pool await, so no in-flight heartbeat can overwrite the clean-stop marker.
- **F-7 (dispatcher)**: `next_wake` includes `offline_due_at()` and the notice retry due times. Notice
  retries keep their own list, pruned in `after_load`. `take_due`: when both are due, the message whose
  first note is older goes first, and the alert wins ties. The offline runtime starts after
  `push.start()`. The sink is called after the tracker guard is dropped.
- **F-8**: the four property literals are pinned in `offline.rs` only; identifier scans use
  comment-stripped production code; the page check covers the `fetchOffline`/`renderOffline` bodies.
- **F-9**: the ADR names both departures (polling instead of signal subscriptions; `Properties.Get` and
  the bus driver's `NameHasOwner` are read-only method calls) and their cost (4 calls per 5 s, about
  69 000 a day; one heartbeat write per minute, about 1 440 a day).
- **F-10**: `showLogin` hides `#offline` and clears its list; `renderOffline` returns early when
  `signedOut`; a non-OK `/api/offline` answer is ignored (the status fetch decides).
- **F-11 / test changes**: test 6 now asserts that `Unknown` releases nothing while NM was observed,
  that `Online` releases, and that 12 `Unknown` steps release on a never-observed host. New:
  `test_roa_tracker_sleep_gap_boundary` (29 999 / 30 000), `test_roa_tracker_unmonitored_release`,
  `test_roa_heartbeat_saves_at_most_once_per_minute` (served, file inode), and
  `test_roa_name_has_no_owner_makes_no_networkmanager_call` (fake bus: `observe` is the seam, so the
  production NameHasOwner gate is pinned statically by ROA-S2 plus a pure helper
  `nm_reads_allowed(has_owner: bool) -> bool`). Test 17 asserts the exit-78 refusal; test 23 becomes
  `test_roa_offline_without_push_is_inert` (a hand-built state with offline on and push off wires
  nothing). The missed-deadline stop case is not driven end to end: the paused test clock cannot
  advance during `serve`'s stop. It is bounded by construction (`timeout`) and recorded as an
  owner-hardware observation.
- **F-12**: rebasing onto `origin/main` (`6f32ef8`) is left to the release step. Overlap: only
  `tests/invariants/src/lib.rs` (module list). The ADR records the daily `shutdown_or_crash` push after
  a nightly shutdown, and the 32-slot history crowded by `sleep` rows. `boot_id` is a later
  improvement. Walkthrough number 202 was assigned by the coordinator.
- New constants: `OFFLINE_UNMONITORED_RELEASE_TICKS: u32 = 12`, `OFFLINE_RETRY_DELAYS_MS: [u64; 4]`,
  `OFFLINE_SHUTDOWN_READ_MS: u64 = 1500`. Revised: `NETWORK_READ_DEADLINE_MS = 3000`,
  `OFFLINE_STOP_DEADLINE_MS = 3000`.

## 16. Revision 3 (plan evaluation round 2, APPROVED with MINOR findings)

- **R2-1**: every save goes through `Arc<tokio::sync::Mutex<()>>`. The owned guard (`lock_owned()`)
  moves into the `spawn_blocking` closure, so a write cancelled by `abort_all()` still holds it until
  the file operation ends. The `Locked` retry loop runs outside the closure, and each attempt takes
  the guard anew.
- **R2-2**: `ZbusNetworkSource::observe` calls `NameHasOwner` first and returns
  `Err(SourceError::Call)` without any NetworkManager call when `nm_reads_allowed(owner)` is
  `false`. ROA-S2 also pins the order in `offline.rs`: the `"NameHasOwner"` literal and the
  `nm_reads_allowed(` call come before the first `"State"` literal.
- **R2-3**: the unmonitored release counts consecutive `Unknown` steps whatever was observed
  before: `OFFLINE_UNMONITORED_RELEASE_TICKS` (12, about 60 s) consecutive `Unknown` steps release a
  held notice, so NetworkManager stopped mid-run cannot lose it. A single slow read (one `Unknown`
  among `Online`/`Offline` steps) never releases. Test 6 asserts that 11 consecutive `Unknown`
  steps release nothing and that `Online` releases; test 6b covers the release after 12.
- **R2-6**: notice retries do not count against the shared spacing or hourly budget (same rule
  as alert retries). Only the first send of a due message goes through `take_due`.
- **R2-5**: rebasing was attempted and refused by the harness's safety rule on stashing (the
  working tree is uncommitted). It is left to the release step, together with the append-only
  sections of `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md` and `tests/invariants/src/lib.rs`.

## 17. Revision 4 (first full workspace run)

The §8 static card markup is superseded. RMC-S50 pins the exact id set of `index.html`, so the
"Offline history" card is built at run time by `app.js` without ids, like the camera card, and
inserted before the push card. `index.html` is unchanged.

# Architect Spec — GitHub #346: Live PC Battery Level in the `soos-remote` Web App

- **Date**: 2026-10-07
- **Round**: 3 (resolves every finding of plan evaluation round 1, mapping in §15, and of round 2, mapping in §16)
- **Branch**: `feat/remote-battery-status` (from `origin/main` 05001c9). GitHub-only issue: not registered in
  `scripts/sync_issue.py` `BRANCH_TO_ISSUE`; the commit message carries `Closes #346`.
- **Binding inputs**: ADR "[2026-10-07] Live Battery Level in `soos-remote`" (`AI/DECISIONS.md`, authoritative),
  GitHub #346, the owner request of 2026-10-07 (see the PC's battery level in real time in the home-screen web app;
  owner-delegated defaults: "pick the best choices").
- **Merge order**: `feat/remote-live-camera` (#345, uncommitted in another worktree) is merged to `main` **before
  Phase 2** of this branch (Round 2, F-9 a). The tester rebases `feat/remote-battery-status` on that `main` first and
  stops with a report if `crates/remote/Cargo.toml` does not yet contain #345's `jpeg-encoder` and `soos-protocol`
  keys. §12 lists the expected conflicts.
- **Agents never** commit, push, install, restart host services, run `tailscale`, or touch `/etc/soos`,
  `~/.config` or the real `/sys` in tests.
- **Code read**: `crates/remote/src/{lib,status,server,routes,http,config,main}.rs`,
  `crates/remote/assets/{index.html,app.js,style.css}`, `crates/remote/tests/{session_tests,server_tests,
  alerts_server_tests,push_server_tests,config_tests,routes_tests}.rs`, `crates/remote/tests/common/harness.rs`,
  `tests/invariants/src/remote_{companion,brand,passkey,push,alerts}_contract.rs`, `packaging/soos-remote.service`,
  `Docs/REMOTE_COMPANION.md`, `AI/VERIFICATION_MATRIX.md` (prefixes), and the #345 worktree diff
  (`crates/remote/**`, `AI/architect_spec_remote_live_camera.md`) read-only.
- **Test prefixes**: `test_rbs_` (behaviour), `test_rbs_s<N>_` (static invariants, new file
  `tests/invariants/src/remote_battery_contract.rs`). Matrix rows `RBS1`–`RBS12` (prefix `RBS` is unused; `RLC` is
  taken by #345).

---

## 0. Decisions (restated from the ADR, plus spec-level decisions)

| # | Decision | Why |
|---|---|---|
| B-1 | `soos-remote` reads `/sys/class/power_supply` itself; no daemon, D-Bus or UPower change; no new dependency | owner default; the user service already runs as the owner, sysfs power-supply attributes are world-readable |
| B-2 | Separate `BatteryView` + `GET|HEAD /api/battery` + `event: battery`; `StatusView`, `/api/status` and `event: status` are **unchanged** | `test_rmc_status_view_json_shape_has_no_identity` and `harness::assert_status_shape` pin the exact five keys (RC-5); `harness::parse_event` and several alerts tests assert every frame is `status`/`alerts`; the alerts feature set the precedent (separate route + event name on the same stream) |
| B-3 | Same `EventSource`, new event name; no second channel | owner default "reuse the existing mechanism" |
| B-4 | On by default (`battery_status = true`), wired only through `ServerState::with_battery`; `main.rs` builds the production source only with `SysfsBattery::kernel()` (never `::system(`, which the existing RMC-S9 invariant forbids crate-wide; Round 3, R2-1) | level/charge/mains is less revealing than active/idle/locked, carries no identity; `ServerState::new` keeps battery off, so every existing server test is unaffected (no battery frame appears in their streams) |
| B-5 | Visibility = `/api/status`: `Route::Battery` is **not** Funnel-public; Funnel session re-validated before every battery event (as alerts, F-4) | ADR item (4) |
| B-6 | A dedicated sampler task (every `BATTERY_SAMPLE_INTERVAL_MS = 5000` while ≥ 1 stream) on the blocking pool, `BATTERY_READ_TIMEOUT_MS = 500`, at most one read in flight, **single-flight for every caller** (sampler, `GET`, stream open): an async read gate coalesces concurrent callers onto one real read; `unavailable` only for a timeout, a panic or an earlier read whose thread is still stuck; a gate-wait timeout is never cached nor published (§5.5) | a hung ACPI/EC driver must never delay logind readings, the accept loop or the stream; 5 s is "live" for a battery and bounds sysfs traffic; concurrent callers must share a real reading, never receive a fabricated `unavailable` (Round 2, F-1) |
| B-7 | Page line built at run time (`document.createElement`), no new id | `test_rmc_s50_ui_contract_ids_labels_and_attributes_are_kept` pins the 29 ids and the `getElementById` set (same approach as #345, its Q-1 = S-3) |
| B-8 | No `checked_unix_ms` in `BatteryView` | the page measures staleness from the status events; a cached sample time would mislead; change detection is plain equality |
| B-9 | Single battery: firmware `capacity` only (never recomputed from energy); several batteries: energy-weighted when every battery reports a usable pair of the same kind, else floor mean of capacities, `None` if any capacity is unknown | matches what the desktop indicator shows; fail closed |
| B-10 | `read_dir` failure (missing root included) → `unavailable`, never `no_battery`; `no_battery` only when the directory was read, every entry had a valid name and a readable `type`, and no system battery is present | fail closed: `no_battery` is an assertion, `unavailable` is "unknown" (Round 2, F-6: an entry skipped for its name makes the scan incomplete) |
| B-11 | The page takes battery data **only** from `event: battery` on its `EventSource`; it never fetches `/api/battery` (the route stays as an API for Shortcuts and diagnostics). The line is cleared when the stream is closed or errors and reappears with the first battery frame of the (re)opened stream | Round 2, F-2 option (a): one ordered channel, so an older response can never overwrite a newer view; every stream open (including `EventSource` auto-reconnect) sends a first battery frame, so nothing is lost |
| B-12 | `online`: `0` → `false`, `1` and `2` → `true`, anything else unknown | kernel ABI `Documentation/ABI/testing/sysfs-class-power`: 0 Offline, 1 Online Fixed, 2 Online Programmable (Round 2, F-5, answers Q-1) |
| B-13 | `main.rs::run_service` ends with exactly `let code = runtime.block_on(run(config, credentials_path, uid)); runtime.shutdown_timeout(Duration::from_millis(RUNTIME_SHUTDOWN_TIMEOUT_MS)); code` instead of a plain drop (the exit code and the RMC-S10 needles `new_current_thread()` / `EXIT_RUNTIME` are kept). The bound applies to **every** blocking-pool task still running at shutdown, not only the battery read: the push store attempts and load (`push.rs:483`, `push.rs:1293`) and, after #345, the JPEG encode (`camera.rs:374` in the #345 tree). This is safe: push store writes are temp file + `fsync` + `rename` (`push.rs:313`, `push.rs:471`), so an abandoned write leaves at worst a stray temp file, never a torn store; the encode is pure computation whose result nobody awaits any more | a read stuck in a hung driver must not turn `systemctl --user stop` into a `TimeoutStopSec` SIGKILL (Round 2, F-7; side effects stated in Round 3, R2-5) |
| B-14 | `scripts/install_remote.sh` template gains one commented `# battery_status = true` line with a one-line comment | the off switch sits where the owner configures every other key (Round 2, F-9 c) |

---

## 1. Scope & blast radius

### 1.1 Files

| File | Change |
|---|---|
| `crates/remote/src/battery.rs` | **new**: types, pure parsers, sysfs reader, `BatterySource`, `SysfsBattery`, `BatteryRuntime`, `run_sampler` |
| `crates/remote/src/lib.rs` | `pub mod battery;` + constants section "Battery level" (§3) + compile-time asserts; module doc paragraph |
| `crates/remote/src/config.rs` | `BatteryConfig`, `RemoteConfig.battery`, `FileConfig.battery_status` |
| `crates/remote/src/routes.rs` | `Route::Battery`, `BATTERY_PATH`, read-route arm |
| `crates/remote/src/http.rs` | `encode_sse_battery_event` |
| `crates/remote/src/server.rs` | `ServerState.battery` + `with_battery`; `Shared.battery`; `serve` spawns the sampler; `Route::Battery` handler; `serve_stream` battery arm; module doc paragraph |
| `crates/remote/src/main.rs` | `if config.battery.enabled { state = state.with_battery(Arc::new(SysfsBattery::kernel())) }`; `run_service` bounds runtime shutdown with `shutdown_timeout` in the exact B-13 shape |
| `crates/remote/assets/app.js` | battery line, `battery` listener, `clearBattery` on stream close/error and sign-out, header comment paragraph; **no** fetch of `/api/battery` (B-11) |
| `scripts/install_remote.sh` | one comment line and `# battery_status = true` in the `remote.toml` template (B-14) |
| `crates/remote/assets/style.css` | **unchanged** (§8.3; `.stat-tile .detail` already sets `tabular-nums`, Round 3, R2-7) |
| tests | §10 (new files + additions) and §9 (setup migrations) |
| docs | §11 |

Unchanged: `status.rs`, `index.html`, `sw.js`, `manifest.webmanifest`, `assets.rs`, the CSP in `http.rs`,
`packaging/soos-remote.service`, `Cargo.toml`/`Cargo.lock`, `deny.toml`, every other crate.

### 1.2 Consumers of changed public items

- `RemoteConfig` (new field `battery`): `config.rs::parse_config` (constructor), the struct literals in
  `crates/remote/tests/common/harness.rs:771`, `server_tests.rs:762`, `alerts_server_tests.rs:207`,
  `push_server_tests.rs:192` and, after #345 is merged, `crates/remote/tests/common/camera.rs` (§9).
- `Route` (new variant `Battery`): `server.rs::handle_connection` (exhaustive match), `routes::is_funnel_public`
  (unchanged: `Battery` falls in the non-public default), `routes::allow_header` (unchanged: `GET, HEAD`).
- `Shared::new` (new parameter or field init): only `serve`.

### 1.3 Out of scope (each needs its own ADR)

Time-to-empty/time-to-full, battery health/cycle count, peripheral batteries (`scope=Device`: mice, headsets),
push notifications on low battery, any write to sysfs, UPower.

---

## 2. Architecture overview

```
sysfs (root injectable) ──read (blocking pool, 500 ms, ≤1 in flight, single-flight gate)──► BatteryRuntime
        ▲                                                        │ cache (Instant, BatteryView), ≤ 5 s old
        │ only while ≥1 stream (sampler, every 5 s, sample())     │ watch<Option<BatteryView>>
GET /api/battery ── current() (cache, or join/perform the one read) ◄┤   (API only; the page never calls it)
/api/events ── status → [alerts] → battery (first, current()) → battery on change only (Funnel session re-validated)
page ── battery line fed only by event: battery; cleared on stream close/error
```

---

## 3. Constants (`crates/remote/src/lib.rs`, new section "Battery level (ADR 2026-10-07 \"Live Battery Level in
`soos-remote`\", architect spec `AI/architect_spec_remote_battery.md` §3)") — the only definitions

```rust
/// Production power-supply class directory (the only `/sys` literal of the crate).
pub const POWER_SUPPLY_ROOT: &str = "/sys/class/power_supply";
/// Most directory entries examined under the root; one more → `BatteryState::Unavailable`.
pub const MAX_POWER_SUPPLIES: usize = 64;
/// Most included system batteries; one more → `BatteryState::Unavailable`.
pub const MAX_BATTERIES: usize = 8;
/// Longest accepted entry name (bytes); longer → entry skipped.
pub const MAX_POWER_SUPPLY_NAME_LEN: usize = 64;
/// Largest attribute value read (bytes, trailing newline included); larger → malformed.
pub const MAX_SYSFS_VALUE_BYTES: usize = 32;
/// Most ASCII digits of an energy/charge value (µWh/µAh); more → malformed.
pub const MAX_SYSFS_MICRO_DIGITS: usize = 19;
/// Bound of one whole sysfs read on the blocking pool (ms); exceeded → `Unavailable`.
pub const BATTERY_READ_TIMEOUT_MS: u64 = 500;
/// Sampler period while at least one stream is open, and the cache lifetime (ms).
pub const BATTERY_SAMPLE_INTERVAL_MS: u64 = 5000;
/// Longest wait for blocking-pool tasks when the service runtime shuts down (ms); a read stuck in a
/// hung driver is abandoned after it (`main.rs::run_service`, B-13).
pub const RUNTIME_SHUTDOWN_TIMEOUT_MS: u64 = 1000;

const _: () = assert!(BATTERY_READ_TIMEOUT_MS < BATTERY_SAMPLE_INTERVAL_MS);
const _: () = assert!(BATTERY_SAMPLE_INTERVAL_MS < SSE_KEEPALIVE_MS);
const _: () = assert!(MAX_BATTERIES <= MAX_POWER_SUPPLIES);
const _: () = assert!(MAX_SYSFS_MICRO_DIGITS < 20); // any 19-digit value fits u64
const _: () = assert!(BATTERY_READ_TIMEOUT_MS < RUNTIME_SHUTDOWN_TIMEOUT_MS);
```

No other file repeats a value (tests import them).

---

## 4. Configuration (`crates/remote/src/config.rs`)

```rust
/// Battery level settings; `Default` = on (ADR 2026-10-07 item (4)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryConfig {
    /// `battery_status`; default `true`.
    pub enabled: bool,
}
impl Default for BatteryConfig { fn default() -> Self { Self { enabled: true } } }

pub struct RemoteConfig { /* existing fields */ /// Battery level settings (ADR 2026-10-07 "Live Battery Level in `soos-remote`").
    pub battery: BatteryConfig }

struct FileConfig { /* existing */ battery_status: Option<bool> }
// parse_config: battery: BatteryConfig { enabled: file.battery_status.unwrap_or(true) }
```

- Absent → `true`. `true`/`false` accepted. Any non-boolean (`"yes"`, `1`, `[]`) → `ConfigError::Syntax` (serde),
  exit 78. No new `ConfigError` variant. No dependency on `rp_id`, `allow_funnel` or any other key.
- Field placement: append after `push` (and after #345's `camera` once merged).

---

## 5. Module `crates/remote/src/battery.rs`

`//!` doc: purpose, the ADR, "never logs, never reads names/serial/model/manufacturer". The module contains **no**
`tracing` macro, no `SystemTime`, no `unwrap`/`expect`/indexing/unchecked arithmetic (workspace lints
`indexing_slicing`, `arithmetic_side_effects` and RMC production-code test).

### 5.1 Public types

```rust
/// Overall state of the battery view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryState { Disabled, Unavailable, NoBattery, Present }

/// Aggregated kernel `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeStatus { Charging, Discharging, Full, NotCharging, Unknown }

/// JSON body of `GET /api/battery` and of every `event: battery`. Exactly four keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct BatteryView {
    pub state: BatteryState,
    /// `Some(0..=100)` only when `Present` and known.
    pub percent: Option<u8>,
    /// `Some` iff `Present`.
    pub charge: Option<ChargeStatus>,
    /// `Some` only when `Present` and at least one non-battery system supply had a valid `online`.
    pub external_power: Option<bool>,
}
impl BatteryView {
    #[must_use] pub const fn disabled() -> Self;     // {Disabled, None, None, None}
    #[must_use] pub const fn unavailable() -> Self;  // {Unavailable, None, None, None}
    #[must_use] pub const fn no_battery() -> Self;   // {NoBattery, None, None, None}
}

/// A usable energy or charge pair of one battery (`full > 0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnergyPair { Energy { now: u64, full: u64 }, Charge { now: u64, full: u64 } }

/// One included system battery (internal reading, never serialized).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryReading { pub capacity: Option<u8>, pub status: ChargeStatus, pub energy: Option<EnergyPair> }

/// Blocking battery source; called only on the blocking pool under `BATTERY_READ_TIMEOUT_MS`.
pub trait BatterySource: Send + Sync + 'static { fn read(&self) -> BatteryView; }

/// Production source: the sysfs class directory under `root`.
#[derive(Debug, Clone)]
pub struct SysfsBattery { root: PathBuf }
impl SysfsBattery {
    /// Test hook and general constructor (tests pass a tempdir).
    #[must_use] pub fn new(root: PathBuf) -> Self;
    /// `POWER_SUPPLY_ROOT`; the only constructor `main.rs` may call (invariant `test_rbs_s3_*`).
    /// Named `kernel`, never `system`: `test_rmc_s9_*` forbids `::system(` in every file of
    /// `crates/remote/src` (Round 3, R2-1).
    #[must_use] pub fn kernel() -> Self;
}
impl BatterySource for SysfsBattery { fn read(&self) -> BatteryView { read_power_supplies(&self.root) } }
```

### 5.2 Pure parsers (all `#[must_use] pub fn`, input `&[u8]` = raw attribute bytes, never panic)

Common pre-step `trim_value(raw)`: reject if `raw.len() > MAX_SYSFS_VALUE_BYTES`; strip **one** trailing `\n` only;
the rest must be non-empty printable ASCII (0x21..=0x7E plus inner space 0x20 for `Not charging`); else malformed.

| Function | Accepts (after `trim_value`) | Result |
|---|---|---|
| `parse_capacity(raw) -> Option<u8>` | 1..=3 ASCII digits, canonical (no leading `0` unless the value is `0`), value ≤ 100 | `Some(v)`; else `None` (`"101"`, `"-1"`, `"82.5"`, `" 82"`, `"082"`, `"0x50"`, `""`, NUL, non-UTF-8, oversize) |
| `parse_status(raw) -> ChargeStatus` | exactly `Charging`, `Discharging`, `Full`, `Not charging`, `Unknown` (case-sensitive) | the variant; anything else → `Unknown` |
| `parse_online(raw) -> Option<bool>` | exactly `0`, `1` or `2` | `0` → `Some(false)`; `1` (Online Fixed) and `2` (Online Programmable) → `Some(true)` (B-12); else `None` (`3`, `01`, `-1`, `yes`, empty) |
| `parse_micro(raw) -> Option<u64>` | 1..=`MAX_SYSFS_MICRO_DIGITS` ASCII digits (leading zeros accepted) | `Some(v)`; else `None` |
| `parse_scope_included(raw: Option<&[u8]>) -> bool` | absent, `System`, `Unknown` → `true`; `Device` or malformed → `false` | |
| `parse_present(raw: Option<&[u8]>) -> bool` | only an exact `0` → `false`; absent, `1`, malformed → `true` | |
| `valid_supply_name(name: &OsStr) -> bool` | 1..=`MAX_POWER_SUPPLY_NAME_LEN` bytes of `[A-Za-z0-9_.:-]`, not starting with `.` | |

### 5.3 `pub fn aggregate(batteries: &[BatteryReading], online: &[bool]) -> BatteryView` (pure)

- `batteries.is_empty()` → `no_battery()` (caller decides unavailability before, §5.4).
- `batteries.len() > MAX_BATTERIES` → `unavailable()`.
- **percent**: one battery → its `capacity`. Several → if every battery has `energy: Some` **of the same kind**:
  `sum_now * 100 / sum_full` with `checked_add`/`checked_mul`/`checked_div` (any overflow or `sum_full == 0` →
  fall through), then `min(100)`, as `u8`; else if every `capacity` is `Some` → floor mean
  (`sum / len` with checked ops); else `None`.
- **charge**: any `Charging` → `Charging`; else any `Discharging` → `Discharging`; else all `Full` → `Full`; else
  any `NotCharging` → `NotCharging`; else `Unknown`.
- **external_power**: `online` contains `true` → `Some(true)`; else non-empty → `Some(false)`; else `None`.
- `state = Present`.

### 5.4 `pub fn read_power_supplies(root: &Path) -> BatteryView` (blocking, std only)

1. `std::fs::read_dir(root)`; any error (NotFound, EACCES, ENOTDIR) → `unavailable()`.
2. Iterate; an iteration error → `unavailable()`; the `MAX_POWER_SUPPLIES + 1`-th entry → `unavailable()`
   (stop at once; never collect more than `MAX_POWER_SUPPLIES` names).
3. An entry whose name fails `valid_supply_name` (invalid byte, non-UTF-8, empty, leading `.`, longer than
   `MAX_POWER_SUPPLY_NAME_LEN`) is skipped **and sets `type_unknown = true`** (B-10, Round 2 F-6): the scan is no
   longer complete, so it can still report `present` for the batteries it read, but never `no_battery`.
4. For each kept entry `dir = root.join(name)` (class entries are symlinks to device directories; following
   **that** link is intended; tests use plain directories): read attributes with `read_attr(&dir, "<attr>")`:
   - `open` with `O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC` (`std::os::unix::fs::OpenOptionsExt::custom_flags`
     with `nix::fcntl::OFlag` bits as `config.rs` does); `fstat` → must be a regular file (a FIFO, directory, socket,
     device or a symlink attribute → `None` **without reading**);
   - read at most `MAX_SYSFS_VALUE_BYTES + 1` bytes (`Read::take`); more than `MAX_SYSFS_VALUE_BYTES` → `None`;
     any I/O error (sysfs `EIO`/`ENODEV`/`ENODATA` while a battery is being removed) → `None`;
   - returns `Option<Vec<u8>>` (≤ 33 bytes, not secret, no zeroize needed).
5. `type` unreadable or malformed → set `type_unknown = true`, skip the entry.
6. `scope` (optional) excluded by `parse_scope_included` → skip.
7. `type == "Battery"`: `parse_present(present)` false → skip; else push `BatteryReading { capacity:
   parse_capacity(capacity), status: parse_status(status) (missing → Unknown), energy: energy pair from
   (`energy_now`, `energy_full`) if both parse and `full > 0`, else (`charge_now`, `charge_full`) likewise, else None }`;
   the `MAX_BATTERIES + 1`-th → `unavailable()`.
8. Any other valid `type` (`Mains`, `USB`, `UPS`, `Wireless`, …): `parse_online(online)` `Some(b)` → push `b`.
9. End: no battery and `type_unknown` (an invalid name or an unreadable/malformed `type`) → `unavailable()`; else
   `aggregate(&batteries, &online)`.

Attributes ever opened (allowlist, pinned by `test_rbs_s2_*`): `type`, `scope`, `present`, `capacity`, `status`,
`energy_now`, `energy_full`, `charge_now`, `charge_full`, `online`. The literals `serial_number`, `model_name`,
`manufacturer`, `uevent` never appear in `battery.rs`. Entry names never leave the function (not in the view, not
in an error, not logged).

### 5.5 Runtime (same file) — single-flight (Round 2, F-1)

```rust
pub struct BatteryRuntime {
    source: Arc<dyn BatterySource>,
    /// Set while a blocking read runs; stays set after a timeout until that thread returns.
    in_flight: Arc<AtomicBool>,
    /// Single-flight gate, held across one whole bounded read (never across a socket write or a send).
    read_gate: tokio::sync::Mutex<()>,
    /// Last read outcome and when it was stored; locked only to copy or replace a `Copy` value.
    cache: tokio::sync::Mutex<Option<(tokio::time::Instant, BatteryView)>>,
    views: watch::Sender<Option<BatteryView>>,
    streams: AtomicUsize,
    wake: Notify,
}

/// Private result of one coalesced read attempt.
enum ReadOutcome { View(BatteryView), GateTimeout }

/// Private cache-reuse rule of one `read_coalesced` call (Round 3, R2-2).
enum CacheRule {
    /// Reuse iff `Instant::now().saturating_duration_since(stored_at) <= BATTERY_SAMPLE_INTERVAL_MS`
    /// (inclusive: an entry exactly one interval old is still reused; one millisecond older is not).
    MaxAge,
    /// Reuse iff `stored_at >= start` (the entry was produced during this call).
    StoredSince(tokio::time::Instant),
}

impl BatteryRuntime {
    #[must_use] pub fn new(source: Arc<dyn BatterySource>) -> Self;
    /// For HTTP callers and the stream's first frame: `read_coalesced(CacheRule::MaxAge)` (reuse an entry
    /// whose age is `<= BATTERY_SAMPLE_INTERVAL_MS`); `GateTimeout` → `unavailable()` (returned, **not**
    /// cached, not published).
    pub async fn current(&self) -> BatteryView;
    /// For the sampler: `read_coalesced(CacheRule::StoredSince(start))` (only an entry stored at or after
    /// the call start is reused);
    /// `GateTimeout` → `None` (nothing published, the previous view stays).
    pub async fn sample(&self) -> Option<BatteryView>;
    #[must_use] pub fn subscribe(&self) -> watch::Receiver<Option<BatteryView>>;
    /// Registers one open stream (`fetch_add`, then `wake.notify_one()`, which stores a permit);
    /// the guard deregisters on drop.
    #[must_use] pub fn register_stream(self: &Arc<Self>) -> BatteryStreamGuard;
}
pub struct BatteryStreamGuard(Arc<BatteryRuntime>); // Drop: streams.fetch_sub saturating (fetch_update)

/// Never returns: while `streams == 0` publishes `None` and awaits `wake` (enable the `Notified` future, re-check
/// `streams`, then await, so a registration between the check and the await is never missed); otherwise
/// `sample().await`, `Some(view)` → `views.send_replace(Some(view))`, then `sleep(BATTERY_SAMPLE_INTERVAL_MS)`.
/// The body is an endless `loop`; `serve` wraps it like the follower (`run_sampler(rt).await; pending().await`).
pub async fn run_sampler(runtime: Arc<BatteryRuntime>);
```

`read_coalesced(rule)` (private; the age is computed with `Instant::saturating_duration_since`, so no
subtraction can underflow and no `checked_sub` is needed):

1. `start = Instant::now()`, `deadline = start + BATTERY_READ_TIMEOUT_MS` (one deadline for the gate wait **and**
   the read).
2. Fast path: a cache entry that satisfies `rule` (evaluated with a fresh `Instant::now()`) → `View(entry)` (no
   gate, no read). Pinned boundary (R2-2): under `MaxAge`, age `== BATTERY_SAMPLE_INTERVAL_MS` is a hit and
   age `== BATTERY_SAMPLE_INTERVAL_MS + 1 ms` is a miss. This is the ADR's "at most 5 s old".
3. `timeout_at(deadline, read_gate.lock())`; elapsed → `GateTimeout`.
4. Under the gate, check the cache again with the same rule (fresh `Instant::now()`) → `View(entry)` without a read (the caller joined the
   read that just finished: this is the coalescing step).
5. If `in_flight` is still set (an earlier read timed out and its thread is still stuck) → `view = unavailable()`
   without spawning. Otherwise set `in_flight`, `spawn_blocking` (the closure owns a guard that clears `in_flight`
   on drop, also on panic), `timeout_at(deadline, handle)`: `Ok(Ok(v))` → `v`; elapsed → `unavailable()` (the
   thread keeps running, `in_flight` stays set until it returns); `Ok(Err(JoinError))` → `unavailable()`.
6. Store `(Instant::now(), view)` in the cache, release the gate, return `View(view)`.

Cached results are therefore always the outcome of an actual attempt (a value, a timeout, a panic or a still-stuck
driver); a contention result (`GateTimeout`) is never cached and never published. Because the gate holder's read
ends by its own deadline, which precedes every waiter's deadline, a waiter normally finds the holder's result in
step 4; `GateTimeout` is a defensive bound for a late scheduler.

Bounds: ≤ 1 blocking thread busy with sysfs at any time (a hung driver: every later attempt returns `unavailable()`
from step 5 until it finishes, no thread pile-up); every caller waits at most `BATTERY_READ_TIMEOUT_MS`; the cache
mutex is held only to copy or replace a `Copy` value; the gate is never held across an await other than the gate
wait and the bounded blocking join.

---

## 6. HTTP surface

### 6.1 Routes (`routes.rs`)

```rust
/// `GET|HEAD /api/battery` (ADR 2026-10-07).
Battery,
/// Path of `GET|HEAD /api/battery`.
pub const BATTERY_PATH: &str = "/api/battery";
// route(): BATTERY_PATH => Some(Route::Battery) in the read-route match.
```
`POST` → `405` with `Allow: GET, HEAD`; `/api/battery/`, `/API/BATTERY`, `/api/battery2` → `404`;
query ignored; `accepts_body` false; `is_funnel_public(Route::Battery) == false`.

### 6.2 Handler (`server.rs`)

`Route::Battery => Some(battery_response(&shared).await)`: `shared.battery` `None` → `BatteryView::disabled()`, else
`runtime.current().await`; `200 application/json` via `serde_json::to_vec`, serialisation failure →
`Response::json(503, "unavailable")`. Reached only after every existing dispatch gate (host `421`, classification
`403`, Funnel session `403 login_required`). HEAD: existing `answer` path (headers, empty body). No logind call.

### 6.3 Stream (`serve_stream`)

- `let battery = shared.battery.clone();` `let _battery_stream = battery.as_ref().map(BatteryRuntime::register_stream);`
  registered **after** the existing slot/subscriber registration and **before** the SSE head.
- After the first status event and the existing first alerts block: if `battery` is `Some`: Funnel session
  re-validated (`session_still_valid`, end on failure, fixed `debug!("stream session ended")`), then, in this
  exact order with **no `.await` between the first two steps** (Round 3, R2-3):
  1. `let mut views = runtime.subscribe();` (a new receiver marks the current watch value as seen);
  2. `let view = runtime.current().await;` (single-flight: it joins the read the registration woke the sampler
     for, §5.5);
  3. send `encode_sse_battery_event(json)`, `last_battery = Some(view)`, `battery_views = Some(views)`.
  The receiver is never created (nor `borrow_and_update()` called) after the first battery write. Why this is
  sufficient: the cache is always written before `send_replace` (§5.5 step 6, `run_sampler`), so at step 1 the
  watch value is never newer than the cache, `current()` returns that value or a newer one, and any view the
  sampler publishes after step 1 (including during the step 3 write on a slow link) makes `changed()` fire; an
  equal view is then filtered by `last_battery`. Auditor constraint: static review only, no dedicated test.
- New `select!` arm (after the alerts arms, before `session_check_at`): `battery_views.changed()` (pending when
  `None`); `Err` → break; `Some(view)` with `Some(view) != last_battery` → re-validate a Funnel session → send →
  `last_battery = Some(view)`; `None` or equal → nothing. A write failure ends the stream (as today).
- No battery keep-alive (status keep-alives keep the stream and the page fresh). Battery events never alter
  `last_sent`, `last_sent_at`, `keepalive_pending` (the status keep-alive arithmetic is untouched).
- Ordering guarantee on open: `status`, then `alerts` (if enabled), then `battery` (if wired).

### 6.4 `http.rs`

```rust
/// One battery event: `event: battery\ndata: <json>\n\n` (`json` carries no newline).
#[must_use] pub fn encode_sse_battery_event(json: &str) -> Vec<u8>;
```

### 6.5 Supervision (`serve`)

If `state.battery` is `Some(source)` (only when `config.battery.enabled`, enforced in `with_battery`): build
`Arc<BatteryRuntime>`, pass it to `Shared`, and `supervised.spawn(async move { run_sampler(rt).await; pending })`
next to the follower/dispatcher (a panic → `ServeError::TaskPanicked`, like the others). Shutdown aborts it.

### 6.6 `ServerState`

```rust
battery: Option<Arc<dyn BatterySource>>,   // `new` → None
/// Wires the battery level (production and tests). Ignored unless `config.battery.enabled` (ADR item (4)).
#[must_use] pub fn with_battery(mut self, source: Arc<dyn BatterySource>) -> Self;
```

### 6.7 JSON examples

```json
{"state":"present","percent":82,"charge":"discharging","external_power":false}
{"state":"no_battery","percent":null,"charge":null,"external_power":null}
{"state":"disabled","percent":null,"charge":null,"external_power":null}
```

---

## 7. Security invariants (new)

| ID | Invariant |
|---|---|
| RBS-I1 | Battery data reaches exactly the callers that may read `/api/status`; anonymous Funnel → `403 login_required`; a revoked/expired Funnel session ends the stream before the next battery event |
| RBS-I2 | No identity or device identifier: view = 4 keys; supply names, serial, model, manufacturer never read or emitted |
| RBS-I3 | Every sysfs read bounded: entries ≤ 64, batteries ≤ 8, value ≤ 32 bytes, regular files only, `O_NOFOLLOW | O_NONBLOCK`, 500 ms (gate wait included), ≤ 1 in flight, single-flight; never blocks the runtime thread; runtime shutdown bounded by `RUNTIME_SHUTDOWN_TIMEOUT_MS` |
| RBS-I4 | Fail closed: errors → `unavailable`/unknown fields, never a guessed value; `no_battery` only after a complete, readable scan (valid names, readable types); `unavailable` never fabricated by contention |
| RBS-I5 | No logging of battery values (no tracing macro in `battery.rs`; server battery lines are fixed text without fields) |
| RBS-I6 | Read-only: no write/`create`/`set_permissions` on any sysfs path; no new dependency; `#![forbid(unsafe_code)]` kept; no root, no unit change |
| RBS-I7 | `/api/status` and `event: status` unchanged (RC-5 exact shape) |
| RBS-I8 | Off switch total: `battery_status = false` → zero sysfs access, `disabled` view, no battery event |
| RBS-I9 | Page: `textContent` only, no new id, no storage, CSP unchanged; battery data only from `event: battery` (no fetch), cleared when the stream closes or errors |

Existing invariants that now also cover `battery.rs` automatically (they scan `crates/remote/src`):
`test_rmc_production_code_never_panics_or_prints`, `test_rmc_logging_never_names_identity_header_or_session_fields`,
`test_rmc_s2_no_network_socket_type_is_named`, `test_rmc_s4_remote_is_a_leaf_crate`,
`test_rmc_s9_bus_rules_match_the_presence_worker` (Round 3, R2-1). The tester and developer must treat the **full**
RMC-S9 needle list (`tests/invariants/src/remote_companion_contract.rs:1148-1180`: `Connection::system`,
`Connection::session`, `Builder::system`, `Builder::session`, `::system(`, `::session(`,
`DBUS_SYSTEM_BUS_ADDRESS`, `DBUS_SESSION_BUS_ADDRESS`, `object_server`, `#[proxy`, `zbus::proxy`, `receive_signal`,
`MessageStream`, `CacheProperties::Yes`, `CacheProperties::Lazily`, `serve_at`, `request_name`, `#[interface`,
`SignalStream`, `zbus::blocking`, `Address::system`, and `env::var`/`env::vars`/`std::env`/`env!(` outside
`main.rs`) as binding on `battery.rs` and on every battery line added to `server.rs`, `main.rs`, `lib.rs` and the
other files of `crates/remote/src`; none of them may be introduced, even in an identifier or a string literal.

Latency budget: not applicable (the PAM/auth path is untouched).

---

## 8. Page

### 8.1 `app.js` (Round 2: F-2 option (a), F-8)

- **No** `BATTERY_PATH` constant, no `fetch` of `/api/battery`, no `fetchBattery` (B-11). Battery data reaches the
  page only through `event: battery`, on the same ordered stream as `status`, so no older response can overwrite a
  newer view. Each stream open, including the browser's own `EventSource` reconnect, sends a first battery frame.
- Built once at load (after the `getElementById` block):
  ```js
  const batteryNode = document.createElement("p");
  batteryNode.className = "detail battery";
  batteryNode.hidden = true;
  updatedNode.parentNode.insertBefore(batteryNode, updatedNode); // between #activity and #updated
  let batteryView = null;
  ```
  No id, no `getElementById` (the S50 set stays the 29 ids), no `innerHTML`.
- `function batteryText(view)` → string or `null` (`null` = hide):
  - not an object, or `state` not a string → `null`;
  - `disabled` → `null`; `unavailable` → `"Battery: unknown"` (Q-2 accepted); `no_battery` → `"No battery"`;
  - `present`: percent part `Number.isInteger(p) && p >= 0 && p <= 100 ? "Battery " + p + "%" : "Battery level unknown"`;
    `let suffix` by `charge`: `charging` → `", charging"`, `discharging` → `", on battery"`, `full` → `", full"`,
    `not_charging` → `", plugged in, not charging"`, anything else (`unknown`, `null`, unexpected) → `""`;
    then exactly `if (suffix === "" && view.external_power === true) { suffix = ", plugged in"; }` — the plugged-in
    fallback applies only when the charge state said nothing, so `discharging` with an underpowered charger reads
    "Battery 50%, on battery" (the kernel's own word), never "on battery, plugged in" (F-8);
  - any other `state` → `null`.
- `function renderBattery(view)`: `batteryView = view; showBattery(true)`.
  `function clearBattery()`: `batteryView = null; showBattery(false)`.
  `function showBattery(reachable)`: `const text = reachable && !signedOut ? batteryText(batteryView) : null;`
  `batteryNode.hidden = text === null; setText(batteryNode, text === null ? " " : text);`
- `render(view, reachable)` calls `showBattery(reachable)` at its end (stale/unreachable hides the line, a fresh
  status restores it; a cleared view stays hidden until the next battery frame).
- `openStream`: `source.addEventListener("battery", function (event) { try { renderBattery(JSON.parse(event.data)); } catch (_error) { /* ignored; the next frame recovers */ } });`
- `closeStream()` calls `clearBattery()` (also reached from `openStream`, so a reopened stream never shows the
  previous stream's value before its own first frame); `source.onerror` calls `clearBattery()` before
  `fetchStatus(true)` (the reconnect's first frame restores it). While the stream is down the status card may
  still be refreshed by `fetchStatus`, but the battery line stays hidden: no value is shown that the stream is not
  keeping current.
- `showLogin`: `clearBattery()` (the tile is hidden with `#status-card` anyway).
- Header comment: one paragraph "Battery level (ADR 2026-10-07 "Live Battery Level in soos-remote"): level, charge
  state and mains presence only, from event: battery on the status stream only, shown with textContent; hidden while
  the PC is unreachable or the stream is down."

### 8.2 `index.html`, `sw.js`, manifest, CSP: unchanged.

### 8.3 `style.css`

Unchanged (Round 3, R2-7). The battery line carries `class="detail battery"` and inherits every style of
`.stat-tile .detail` (`style.css:426-430`), which already sets `font-variant-numeric: tabular-nums`; no rule is
added, so `style.css` leaves the #345 conflict set and `test_rmc_s45`–`s49` are untouched.

---

## 9. Existing tests that need migration (setup only; no assertion touched)

| File | Change | Nature |
|---|---|---|
| `crates/remote/tests/common/harness.rs:771` | add `battery: soos_remote::config::BatteryConfig::default(),` to the `RemoteConfig` literal | compile-only setup migration (precedent: `alerts`, `push`, #345 `camera`) |
| `crates/remote/tests/server_tests.rs:762` | same | same |
| `crates/remote/tests/alerts_server_tests.rs:207` | same | same |
| `crates/remote/tests/push_server_tests.rs:192` | same | same |
| `crates/remote/tests/common/camera.rs` (after #345 merge) | same | same |

Record them in `AI/tester_contract_remote_battery.md` §"Contract Migrations" as setup-only. Every other existing
test is unchanged and must stay green: none of them calls `with_battery`, so no `event: battery` frame ever reaches
`harness::parse_event` (asserts `status`), `alerts_server_tests` (asserts `status`/`alerts` only) or the exact-shape
tests. `test_rmc_status_view_json_shape_has_no_identity` and `assert_status_shape` stay byte-for-byte.

---

## 10. Test list (Phase 2)

Helpers (tester-owned, new): `crates/remote/tests/common/battery.rs` with `FakeSysfs` (tempdir root;
`battery(name, &[(attr, bytes)])`, `supply(name, type, &[(attr, bytes)])`, `set(name, attr, bytes)`, `fifo(name,
attr)` via `nix::unistd::mkfifo`, `symlink_attr`), and `ScriptedBattery` (a `BatterySource` returning queued views
and counting calls, with three modes: immediate; **delayed** — blocks for a fixed real-time duration such as 50 ms,
then returns; **gated** — blocks until its gate opens, to simulate a hung driver, and reports when a call has
entered `read` and when it has returned from `read`).

Time model (Round 2, F-4; replaces the round-1 note):
- Server tests run `#[tokio::test(start_paused = true)]` on the current-thread runtime.
- **Virtual time is moved only explicitly.** Crossing `BATTERY_SAMPLE_INTERVAL_MS`, `BATTERY_READ_TIMEOUT_MS` or
  `SSE_KEEPALIVE_MS` uses `tokio::time::advance` (the `Harness::advance_ms` pattern, `harness.rs:820`). No test relies
  on auto-advance: while a `spawn_blocking` task runs, tokio 1.53.1 inhibits auto-advance
  (`runtime/blocking/schedule.rs`, `time/clock.rs`), and a `yield_now` + `std::thread::sleep` loop never parks the
  runtime, so the clock would not move anyway.
- **Real-time waits are only for blocking-pool completion** (a delayed `ScriptedBattery`, a `FakeSysfs` read): helper
  `wait_battery(..)` interleaves `yield_now` with `std::thread::sleep(1 ms)` for up to 2 s of real time and never
  advances the clock (the `wait_view` pattern, `push_server_tests.rs:466`).
- **Cache boundary (Round 3, R2-2).** In paused time a read does not move the clock, so an entry is stored at the
  exact virtual instant of its request. `current()` reuses it up to and including an age of
  `BATTERY_SAMPLE_INTERVAL_MS` (§5.5 `CacheRule::MaxAge`). A test that needs an HTTP read to **miss** the cache
  therefore advances `BATTERY_SAMPLE_INTERVAL_MS + 1`; a plain `advance(BATTERY_SAMPLE_INTERVAL_MS)` is a hit. The
  sampler (`CacheRule::StoredSince`) always reads anew after its own sleep and is unaffected.
- **Gated sources release in `Drop`.** A gated `ScriptedBattery` returns a `GateRelease` guard; the test creates it
  before its first assertion, and its `Drop` opens the gate. A failing assertion unwinds, the guard opens the gate,
  the blocking thread returns, and the runtime drop finishes: the test fails instead of hanging.

### 10.1 `crates/remote/tests/battery_tests.rs` (new; pure and tempdir)

1. `test_rbs_single_battery_discharging_on_battery` — BAT0 `capacity 82\n`, `status Discharging\n`, ADP0 `Mains`
   `online 0` → `{Present, Some(82), Some(Discharging), Some(false)}`. Firmware-capacity rule (B-9, F-3 a): the same
   BAT0 with `energy_now 50000000` / `energy_full 100000000` → still `Some(82)` (not 50); likewise with
   `charge_now 30` / `charge_full 100` → `Some(82)`.
2. `test_rbs_single_battery_charging_on_mains` — `Charging`, Mains `online 1` → `Some(true)`.
3. `test_rbs_no_battery_desktop` — empty root → `NoBattery`; only `Mains online 1` → `NoBattery` (all optional fields `None`).
4. `test_rbs_unreadable_root_is_unavailable` — missing root, root is a file, root mode `000` (skip the last check when
   running as root) → `Unavailable`.
5. `test_rbs_malformed_capacity_is_unknown` — table: `""`, `"\n"`, `"abc"`, `"101"`, `"-1"`, `"82.5"`, `" 82"`, `"082"`,
   `"0x50"`, `"1000"`, 33-byte value, `"8\x002"`, `[0xff]`, `"82\n\n"` → `percent None`, state `Present`; `"0"`, `"100\n"`,
   `"7"` accepted.
6. `test_rbs_status_strings_are_exact` — the five kernel strings map; `charging`, `CHARGING`, `Full ` (trailing space),
   `Foo`, oversize, missing file → `Unknown`.
7. `test_rbs_peripheral_and_empty_bays_are_ignored` — `scope Device` battery ignored (desktop with a mouse battery →
   `NoBattery`); `scope System` included; malformed scope ignored; `present 0` skipped; `present` absent/`1` included.
8. `test_rbs_multiple_batteries_are_aggregated` — 20 Wh at 50 % + 80 Wh at 100 % (energy pairs) → 90; mixed
   `energy_*`/`charge_*` kinds → mean of capacities; one battery without a pair → mean; one capacity malformed and no
   full pair set → `None`; `full = 0` pair ignored; `now > full` clamps at 100.
9. `test_rbs_charge_aggregation_table` — `aggregate` table: Charging+Discharging → Charging; Discharging+Full →
   Discharging; Full+Full → Full; Full+NotCharging → NotCharging; Unknown+Full → Unknown.
10. `test_rbs_entry_and_battery_bounds` — 65 entries → `Unavailable`; 9 batteries → `Unavailable`; exactly 64 entries /
    8 batteries accepted; names `.hidden`, 65-byte name, `bat 0`, non-ASCII → skipped: each of them next to a valid
    BAT0 → `Present` (BAT0's values), and each of them alone, or next to only a `Mains` supply → `Unavailable`, never
    `NoBattery` (F-6, B-10).
11. `test_rbs_non_regular_attributes_are_not_read` — `capacity` as FIFO (test finishes promptly, no block), as
    directory, as symlink to a valid file → `percent None`.
12. `test_rbs_type_unreadable_never_claims_no_battery` — an entry with `type` missing/malformed and no other battery →
    `Unavailable`; plus a valid battery elsewhere → `Present`.
13. `test_rbs_external_power_rules` — `USB online 1` → `Some(true)`; `Mains online 2` → `Some(true)` (B-12, F-5);
    `online 3`, `01`, `-1`, empty, malformed → ignored (`None` when alone); `UPS online 0` → `Some(false)`; `scope
    Device` supply ignored; `parse_online` table `0`/`1`/`2`/`2\n` → `false`/`true`/`true`/`true`.
14. `test_rbs_view_json_shape_has_no_identity` — exact JSON of the three §6.7 examples; exactly the keys
    `charge, external_power, percent, state`; with `serial_number`, `model_name`, `manufacturer` files containing a
    marker and entry name `BATMARKER0`, the serialized view and `Debug` contain no marker.
15. `test_rbs_parsers_never_panic` (proptest, 512 cases) — arbitrary `Vec<u8>` ≤ 64 bytes into every parser;
    `parse_capacity` returns `Some(v)` only for the canonical decimal of `v ≤ 100` (optionally + one `\n`).
16. `test_rbs_aggregate_never_overflows` — energies at `u64::MAX` and 19-digit values → no panic, falls back per §5.3.
17. `test_rbs_sysfs_source_rereads_live` — `SysfsBattery::new(root)`: read, change `capacity`, read again → new value;
    with an energy pair whose ratio disagrees with `capacity` the re-read follows `capacity` (F-3 a).

### 10.2 `crates/remote/tests/config_tests.rs` (new test only)

18. `test_rbs_battery_config_key` — absent → `enabled true`; `battery_status = false`/`true`; `"yes"`, `1`, `[]` →
    `ConfigError::Syntax`; `BatteryConfig::default().enabled == true`; independent of `rp_id`/`allow_funnel`.

### 10.3 `crates/remote/tests/routes_tests.rs` (new test only)

19. `test_rbs_battery_route` — GET/HEAD (with and without query) → `Battery`; POST/Other → `MethodNotAllowed` with
    `allow_header == "GET, HEAD"`; `/api/battery/`, `/API/BATTERY`, `/api/battery2` → `NotFound`;
    `!is_funnel_public(Route::Battery)`; `!accepts_body(Post, BATTERY_PATH)`.

### 10.4 `crates/remote/tests/battery_server_tests.rs` (new; own setup like `alerts_server_tests.rs`, own frame reader)

20. `test_rbs_get_battery_tailnet` — `FakeSysfs` wired: `200`, body = the view, standard headers (`no-store`, CSP);
    HEAD → same headers, empty body.
21. `test_rbs_battery_visibility_matches_status` — non-allowlisted tailnet login `403`; anonymous Funnel `403
    login_required` with zero source reads; Funnel with a session `200`; wrong host `421`.
22. `test_rbs_disabled_or_unwired` — `battery_status = false` + `with_battery` → `disabled`, source never called,
    stream has no `battery` frame over 2 × `BATTERY_SAMPLE_INTERVAL_MS`; enabled but not wired → `disabled`.
23. `test_rbs_stream_first_frames_order` — alerts off: `status`, `battery`; alerts on: `status`, `alerts`, `battery`.
    The first `battery` frame's data equals the exact JSON of the `FakeSysfs` view
    (`{"state":"present","percent":82,"charge":"discharging","external_power":false}`), and after
    `advance(BATTERY_SAMPLE_INTERVAL_MS)` with sysfs unchanged no further `battery` frame (in particular no
    `unavailable`) has arrived (F-3 b, F-1). Re-checked against the pinned rule (R2-2): this advance drives the
    sampler, whose `StoredSince` rule performs a real re-read; equality then suppresses the frame. No HTTP read is
    involved, so the `MaxAge` boundary does not apply.
24. `test_rbs_stream_live_update_on_change_only` — change `capacity` → one `battery` frame within
    `BATTERY_SAMPLE_INTERVAL_MS` + slack with the new percent; unchanged sysfs → no further `battery` frame over
    3 intervals while status keep-alives continue; plug change (`online` 0→1) → a frame. Re-checked against the
    pinned rule (R2-2): every update comes from the sampler (`StoredSince`), so the `MaxAge` boundary does not
    apply.
25. `test_rbs_hung_source_is_bounded` — gated `ScriptedBattery` (guard created first, F-4), step by step:
    1. send `GET /api/battery`, wait (real time) until the source reports the call entered, then
       `advance(BATTERY_READ_TIMEOUT_MS)` → the response is `unavailable` (cached, an actual timeout);
    2. `advance(BATTERY_SAMPLE_INTERVAL_MS + 1)` (R2-2: the cached entry is now one millisecond past the interval,
       so the GET misses the cache), second GET → `unavailable` with `calls == 1`: this response can only come from
       §5.5 step 5 (earlier read still stuck, no new spawn);
    3. open a stream: it still receives its first `status` frame and its first `battery` frame (`unavailable`);
       `advance(SSE_KEEPALIVE_MS)` → a status keep-alive arrives; `calls == 1` throughout (every sampler tick also
       ends in step 5); close the stream and `settle` until the server has seen EOF (`server.rs:2230-2234`), so
       the sampler is idle again before step 4;
    4. drop the guard, wait (real time) until the source reports the call returned, then
       `advance(BATTERY_SAMPLE_INTERVAL_MS + 1)` (R2-2: every cached `unavailable` is now past the interval), GET →
       `present` with `calls == 2`.
26. `test_rbs_panicking_source_is_unavailable` — source panics → `unavailable`, server keeps serving, later reads work
    (in-flight flag cleared).
27. `test_rbs_sampler_idle_without_streams` — no stream for 3 intervals → zero source calls; one stream opened →
    calls resume; stream closed → calls stop again.
28. `test_rbs_get_uses_cache_within_interval` (no stream open, sampler idle) — first GET at `T0` → `calls == 1`;
    a second GET without any advance → `calls == 1`; `advance(BATTERY_SAMPLE_INTERVAL_MS)` (age exactly the
    interval, inclusive hit, R2-2), GET → `calls == 1`; `advance(1 ms)` (age interval + 1 ms), GET → `calls == 2`.
    Both sides of the boundary are asserted, so a `<` instead of `<=` or a `>=`/`>` slip in the rule fails.
29. `test_rbs_funnel_session_revalidated_before_battery_event` — Funnel stream with a session, first `battery` frame
    received; the session is revoked (logout from another connection); sysfs `capacity` is changed; then
    `advance(BATTERY_SAMPLE_INTERVAL_MS)` only — the total virtual time since the stream opened stays below
    `SSE_KEEPALIVE_MS` (asserted in the test from the constants), so the keep-alive `session_check_at` has not run
    (F-3 c): the stream ends with no battery frame after the revocation. Control half: the same sequence without the
    revocation delivers a `battery` frame with the new percent (proves the change was sampled and would have been sent).
30. `test_rbs_status_contract_unchanged_with_battery` — with battery wired, `/api/status` and every `status` frame
    pass the five-key shape check (re-implemented locally, not by editing the harness).
31. `test_rbs_never_logs_battery_values` — capture tracing at `TRACE` (existing pattern of
    `test_rmc_server_never_logs_identity_or_request_data`); `FakeSysfs` battery `BATMARKER0` with `capacity 87`,
    `status Discharging`, Mains `online 1`; after GETs, a stream and one live update (capacity `86`), the captured log
    contains none of the field-aware markers (F-3 d, Round 3 R2-4): `capacity=`, `capacity:`, `percent=`,
    `percent:`, `"percent":`, `external_power=`, `external_power:`, `charge=`, `discharging`, `Discharging`, `=87`,
    `: 87`, `87%`, `=86`, `86%`, and not `BATMARKER0`. The bare words `capacity` and `percent` are **not** markers:
    the fixed lines "funnel capacity reached" (`server.rs:1088`) and "anonymous funnel capacity reached"
    (`server.rs:1133`) contain the first, and any later fixed text could contain the second. The tester confirms by
    `grep` over `crates/remote/src` that no fixed log message contains any marker of this list before freezing it.

### 10.5 `tests/invariants/src/remote_battery_contract.rs` (new; `mod remote_battery_contract;` in `lib.rs`)

32. `test_rbs_s1_battery_module_never_logs_and_never_writes` — `battery.rs` (production part): no `tracing`/`log`
    macro, no `File::create`, `OpenOptions::new().write`, `fs::write`, `set_permissions`, `remove_file`; contains
    `O_NOFOLLOW` and `O_NONBLOCK`; no `unsafe`.
33. `test_rbs_s2_attribute_allowlist_and_no_identifiers` — every attribute string literal passed to `read_attr` is in
    the §5.4 allowlist; `serial_number`, `model_name`, `manufacturer`, `uevent` never appear in `crates/remote/src`.
34. `test_rbs_s3_sysfs_root_single_source` — over `strip_comments(production_part(..))` of every file of
    `crates/remote/src` (the `remote_companion_contract` helpers, F-9 d; doc comments citing the path are ignored),
    `"/sys` appears exactly once, in `lib.rs` (`POWER_SUPPLY_ROOT`); `main.rs` calls `SysfsBattery::kernel()` and
    never `SysfsBattery::new(`; no file of `crates/remote/src` contains `SysfsBattery::system` (R2-1; RMC-S9 forbids
    `::system(` anyway).
35. `test_rbs_s4_page_battery_line` — `app.js` contains `addEventListener("battery"`, `createElement("p")`, the
    `"detail battery"` class assignment, `insertBefore`, `function clearBattery`; the body of `source.onerror` and of
    `function closeStream` each contain `clearBattery()`; **F-2 pin (option a)**: `app.js` contains neither
    `/api/battery` nor `BATTERY_PATH` nor `fetchBattery`; **F-8 pin**: `app.js` contains
    `suffix === "" && view.external_power === true`, the literal `", plugged in"` (closing quote included) exactly
    once (that fallback) and `", on battery"` exactly once; no `getElementById("battery`; `index.html` contains no `battery`; `sw.js`
    contains no `battery`.
36. `test_rbs_s5_documented` — `Docs/REMOTE_COMPANION.md` has a section whose heading contains `Battery level`
    mentioning `battery_status`, `/sys/class/power_supply`, `GET /api/battery`, `event: battery`, `5 s`,
    `login_required`; the §5 table has a `battery_status` row with default `true`; the §6 table has a
    `GET /api/battery` row; `AI/ARCHITECTURE.md` §13 and `AI/MOCK_STRATEGY.md` mention `battery`;
    `scripts/install_remote.sh` contains the line `# battery_status = true` (B-14).
37. `test_rbs_s6_no_new_dependency` — `crates/remote/Cargo.toml` `[dependencies]` key set equals a pinned list copied
    from the manifest of the `main` that already contains #345 (precondition of Phase 2, header "Merge order"; F-9 a).
### 10.6 Round 2 additions (test 38 in `battery_server_tests.rs`, test 39 in `remote_battery_contract.rs`)

Total: 17 + 1 + 1 + 13 + 7 = 39 tests.

38. `test_rbs_concurrent_readers_share_one_read` (F-1) — (i) runtime level: delayed
    `ScriptedBattery` (50 ms real time, `present` 64 %), `tokio::join!(rt.current(), rt.current(), rt.sample())` →
    all three `present` 64 and `calls == 1`; (ii) server level: the same source wired, a stream request and a
    `GET /api/battery` written before either response is read → the stream's first `battery` frame and the GET body
    are both `present` 64 and `calls == 1`; no `unavailable` anywhere; (iii) after `advance(BATTERY_READ_TIMEOUT_MS)`
    nothing was cached as `unavailable` (a further GET within the interval still returns `present` 64, `calls == 1`).
    Re-checked against the pinned rule (R2-2): the entry is then 500 ms old (`<=` interval, a hit), and 500 ms is
    below the sampler period, so no sampler tick runs during (iii).
39. `test_rbs_s7_runtime_shutdown_is_bounded` (invariant; F-7) — `main.rs::run_service` (production part, comments
    stripped) contains `shutdown_timeout(` and `RUNTIME_SHUTDOWN_TIMEOUT_MS`, and its `shutdown_timeout(` occurs after
    its `block_on(` (B-13 shape, R2-5); `lib.rs` defines `RUNTIME_SHUTDOWN_TIMEOUT_MS` once.

---

## 11. Documentation to update (Phase 6)

- `Docs/REMOTE_COMPANION.md`: header scope note (battery line); §1 bullet; new section `## 2g. Battery level`
  (the next free letter after #345's `## 2f. Live camera view`; tests locate it by title): source, attributes read, bounds,
  single-flight reads, the page fed only by `event: battery`,
  aggregation, `unavailable`/`no_battery` meaning, 5 s cadence, visibility = status (`login_required` on Funnel),
  `battery_status = false`; §5 row `battery_status | no | true | TOML boolean; false disables every sysfs read`;
  §6 rows `GET /api/battery` and the `event: battery` sentence in the `/api/events` row; §7 troubleshooting
  ("Battery: unknown" → `cat /sys/class/power_supply/*/type`); §8 limitation (kernel/firmware granularity, ≤ ~5 s
  delay, `external_power` unknown without a readable `online`, a single battery without a readable `capacity`
  shows "Battery level unknown" even if it exposes an energy pair (B-9, fail closed), a read stuck in a hung driver
  is abandoned after `RUNTIME_SHUTDOWN_TIMEOUT_MS` at service stop, and the same bound applies to every other
  blocking task then still running (push store writes, which are temp file + `fsync` + `rename` and so never leave
  a torn store, and #345's JPEG encode; B-13, R2-5)); §9 out of scope (time-to-empty, health, peripherals); §4
  installation: the commented `battery_status` template line.
- `AI/ARCHITECTURE.md` §13: one paragraph (battery view, separate event, sysfs read-only, no daemon change).
- `AI/MOCK_STRATEGY.md` "Remote Companion Doubles": `FakeSysfs` tempdir root and `ScriptedBattery`.
- `AI/VERIFICATION_MATRIX.md`: rows §13 below, intro line naming the ADR and #346.
- `.agents/skills/dev-workflow/references/project-facts.md`: one row "`soos-remote` battery level" with the §3
  constants and the `battery_status` key (after #345's camera row).
- `AI/walkthroughs/NN_remote_battery_status.md`: the next free number on the rebased `main` at Phase 6 (190 is the
  last today; #345 is expected to take 191, so 192 unless that changes; re-check with `ls AI/walkthroughs | sort -V`).
- `AI/DECISIONS.md`: the ADR (done in Phase 1).

---

## 12. Expected conflicts with #345 (`feat/remote-live-camera`)

| File | Conflict | Resolution |
|---|---|---|
| `AI/DECISIONS.md` | both append a bullet at the end | keep both, camera first |
| `crates/remote/src/lib.rs` | adjacent `pub mod` lines and new constant sections | keep both; `pub mod battery;` before `pub mod camera;` (alphabetical) |
| `crates/remote/src/config.rs` | `RemoteConfig` field, `FileConfig` keys, `parse_config` literal | keep both fields (`camera`, then `battery`) |
| `crates/remote/src/routes.rs` | `Route` variants and the read-route match | keep both |
| `crates/remote/src/server.rs` | `ServerState`/`Shared` fields, `new`, `serve` spawn list, `handle_connection` match | keep both; the battery stream arm is in `serve_stream`, which #345 does not change |
| `crates/remote/src/main.rs` | wiring block | keep both |
| `crates/remote/assets/app.js` | top-level element block after the `getElementById` block (`batteryNode`, `batteryView`) and the new battery functions | keep both; battery code is in the top-level element block, `openStream` (the `battery` listener), `closeStream`, `source.onerror`, `render` and `showLogin`; `fetchStatus` is untouched. #345's `app.js` diff touches none of `openStream`, `closeStream`, `onerror` (its camera card goes after `#push-section`; `clearCameraCanvas`/`renderCamera` are new functions), so the real conflict surface is the adjacent top-level additions only (Round 3, R2-6) |
| test `RemoteConfig` literals | both add a field | add both fields |
| `scripts/install_remote.sh` | both append to the `remote.toml` template | battery comment + `# battery_status = true` after #345's camera block |
| `tests/invariants/src/lib.rs` | adjacent `mod remote_*_contract;` lines | keep both, alphabetical (`remote_battery_contract` before `remote_camera_contract`) |
| `AI/ARCHITECTURE.md` §13 | both append a paragraph | keep both, camera first |
| `Docs/REMOTE_COMPANION.md` | header note, §1 bullets, new `## 2f.` (camera) / `## 2g.` (battery), §4, §5 rows, §6 rows, §7, §8, §9 | keep both everywhere; battery after camera; battery section is `## 2g.` |
| `AI/VERIFICATION_MATRIX.md` | both add an intro line and a row block (`RLC*`, `RBS*`) | keep both, `RBS` block after `RLC` |
| `AI/MOCK_STRATEGY.md` "Remote Companion Doubles" | both add doubles | keep both |
| `.agents/skills/dev-workflow/references/project-facts.md` | both add a constants row | keep both, battery row after camera row |
| `AI/plan_evaluator_report.md`, `AI/candid_review_report.md` | per-branch reports overwrite each other | take this branch's version (they describe the branch under review) |
| walkthrough number | both take "the next free number" | #345 first (191 expected), battery takes the next free one after the rebase |

---

## 13. Acceptance criteria → tests → matrix rows (status `⬜ Pending (Phase 2)` until evidence)

| Row | Criterion | Tests |
|---|---|---|
| RBS1 | `battery_status` key: default `true`, boolean only, off switch total (no read, `disabled`, no event) | 18, 22 |
| RBS2 | Sysfs reader: single battery, charging/discharging, mains, live re-read, injectable root | 1, 2, 17 |
| RBS3 | No-battery desktops and peripheral batteries: `no_battery` only after a complete readable scan; `scope=Device` and `present=0` ignored | 3, 7, 12 |
| RBS4 | Fail closed: unreadable root → `unavailable`; malformed values → unknown fields; exact parsers (`online` 0/1/2); no panic; an invalid entry name never yields `no_battery` | 4, 5, 6, 10, 13, 15 |
| RBS5 | Multiple batteries: energy-weighted / mean / unknown; charge aggregation; overflow-safe | 8, 9, 16 |
| RBS6 | Bounds: ≤ 64 entries, ≤ 8 batteries, ≤ 32-byte regular files, `O_NOFOLLOW | O_NONBLOCK`, 500 ms, ≤ 1 in flight, single-flight without fabricated `unavailable`, panic-safe, bounded runtime shutdown | 10, 11, 25, 26, 32, 38, 39 |
| RBS7 | No identity: four-key view, no names/serial/model/manufacturer, no logging of values | 14, 31, 32, 33 |
| RBS8 | `GET|HEAD /api/battery` route and visibility equal to `/api/status` (anonymous Funnel `403 login_required`) | 19, 20, 21 |
| RBS9 | Live update on the existing stream: first-frame order and content, change-only events, 5 s sampler only while streaming, cache, concurrent readers share one read; Funnel session re-validated before each battery event | 23, 24, 27, 28, 29, 38 |
| RBS10 | `/api/status` and `event: status` unchanged; every existing test green with setup-only migrations | 30, §9 |
| RBS11 | Page line: run-time element, `textContent`, fed only by `event: battery` (no fetch), cleared on stream close/error, no contradictory "on battery, plugged in", hidden when unreachable/signed out/disabled, CSP and ids unchanged; docs and install template; single `/sys` source; no new dependency | 34, 35, 36, 37, existing `test_rmc_s8_*`, `test_rmc_s45`–`s50` |
| RBS12 | Hardware (owner): the iPhone home-screen app shows the same level as the desktop indicator and follows plug/unplug within about 5 s, also right after opening the app (no "Battery: unknown" on a normal open); a desktop without a battery shows "No battery"; with an underpowered USB-C charger the line reads "on battery", never "on battery, plugged in" | manual |

---

## 14. Open questions for the plan evaluator

Round 1 questions are answered (see §15): Q-1 → `online=2` is `true` (F-5, B-12); Q-2 → "Battery: unknown" for
`unavailable` accepted. No open question in Round 2 or Round 3.

---

## 15. Round 2 — resolution of the plan evaluation (round 1, `REVISION_REQUIRED`)

| Finding | Severity | Resolution | Where |
|---|---|---|---|
| F-1 concurrent reads fabricate a cached/published `unavailable` | MAJOR | Single-flight runtime: async `read_gate` held across the bounded read; gate wait and read share one `BATTERY_READ_TIMEOUT_MS` deadline; cache re-checked under the gate (coalescing); `unavailable` only for a timeout, a panic or a still-stuck earlier read (`in_flight`); a gate-wait timeout (`GateTimeout`) is never cached and never published (sampler keeps the previous view); sampler uses `sample()` (`not_before = start`), HTTP/stream use `current()` (`not_before = now − 5 s`). ≤ 1 blocking thread kept. New test 38 (runtime-level `join!` and server-level stream + GET, both `present`, `calls == 1`, nothing cached as `unavailable`); test 23 checks the first frame's content | B-6, §2, §5.5, §6.3, tests 23, 38; ADR item (2) amended |
| F-2 a `GET` response overwrites a newer stream view | MAJOR | Option (a): the page takes battery data only from `event: battery`; no `fetchBattery`, no `/api/battery` in `app.js`; `clearBattery()` on `closeStream`, `source.onerror` and `showLogin`, so a down stream shows no stale value and every (re)opened stream restores it with its first frame. `GET /api/battery` stays as an API. Pinned by test 35 | B-11, §1.1, §2, §8.1, test 35; ADR item (5) amended |
| F-3 a test 1/17 cannot catch an energy-preferring single battery | MINOR | Fixtures with `capacity 82` and an energy (50/100) or charge (30/100) pair → `82` | tests 1, 17 |
| F-3 b test 23 checks only event names | MINOR | Exact JSON of the first battery frame; no further frame (no `unavailable`) after one interval | test 23 |
| F-3 c test 29 passes through the keep-alive session check | MINOR | Revocation + sysfs change + one `BATTERY_SAMPLE_INTERVAL_MS` advance, total virtual time asserted < `SSE_KEEPALIVE_MS`; control half without revocation proves the frame would be sent | test 29 |
| F-3 d test 31 marker may collide | MINOR | Field-aware markers (`percent`, `capacity`, `external_power`, `discharging`, `"percent":`, `=87`, `: 87`, `87%`, `=86`, `86%`) plus `BATMARKER0` | test 31 |
| F-4 incomplete time model; hung gate can hang the runtime drop | MINOR | §10 time model rewritten: explicit `tokio::time::advance` for every interval/timeout crossing, real-time waits only for blocking-pool completion, gated `ScriptedBattery` releases in a `GateRelease` `Drop` guard created before the first assertion; test 25 rewritten step by step | §10 helpers and time model, tests 25, 28 |
| F-5 / Q-1 `online=2` | MINOR | `0` → `false`, `1` and `2` → `true` (kernel ABI "Online Fixed"/"Online Programmable"), else unknown | B-12, §5.2, test 13; ADR item (2) amended |
| F-6 invalid entry names allow `no_battery` after an incomplete scan | MINOR | A name-skipped entry sets `type_unknown`; without a battery the result is `unavailable` | B-10, §5.4 steps 3 and 9, test 10 |
| F-7 runtime drop waits forever for a hung read | MINOR | `run_service` ends with `runtime.shutdown_timeout(Duration::from_millis(RUNTIME_SHUTDOWN_TIMEOUT_MS))` (1000 ms, > `BATTERY_READ_TIMEOUT_MS` by const assert); documented in Docs §8; static test 39 | B-13, §1.1, §3, §11, test 39 |
| F-8 "on battery, plugged in" | MINOR | Plugged-in suffix only when the charge suffix is empty (`unknown`/other); pinned statically in test 35 and in the RBS12 manual check | §8.1, test 35, RBS12 |
| F-9 a test 37 depends on #345 | MINOR | Phase 2 runs on a `main` that already contains #345 (header "Merge order"; the tester stops and reports otherwise); test 37 pins that manifest's key set | header, test 37 |
| F-9 b §12 misses shared docs and the walkthrough number | MINOR | §12 now lists `install_remote.sh`, `tests/invariants/src/lib.rs`, `AI/ARCHITECTURE.md` §13, `Docs/REMOTE_COMPANION.md` (all sections, `## 2g.`), `AI/VERIFICATION_MATRIX.md`, `AI/MOCK_STRATEGY.md`, `project-facts.md`, the per-branch reports and the walkthrough number; §11 adds the `project-facts.md` row | §11, §12 |
| F-9 c install template | MINOR | Adopted: one comment line and `# battery_status = true` appended after #345's camera block; asserted by test 36 | B-14, §1.1, §11, §12, test 36 |
| F-9 d test 34 sees doc comments | MINOR | Test 34 runs on `strip_comments(production_part(..))` (the `remote_companion_contract` helpers) | test 34 |
| Q-2 "Battery: unknown" for `unavailable` | answered | Accepted; kept in §8.1. After F-1 it no longer appears on normal opens | §8.1, §14 |

Unchanged by Round 2 (the evaluator found them sound): threat model and visibility (B-5, RBS-I1), bounds of §3 and
§5.4, confidentiality (RBS-I2, RBS-I5), on-by-default (B-4), dependencies (no new crate), test integrity (setup-only
migrations of §9), B-8 (no timestamp in `BatteryView`).

Note: the §15 F-1 row describes the Round 2 cache rule as `not_before = now − 5 s`; Round 3 (R2-2) restates it as
`CacheRule::MaxAge` (`age <= BATTERY_SAMPLE_INTERVAL_MS`, inclusive) and `CacheRule::StoredSince(start)`; §5.5 is
authoritative.

---

## 16. Round 3 — resolution of the plan evaluation (round 2, `REVISION_REQUIRED`)

| Finding | Severity | Resolution | Where |
|---|---|---|---|
| R2-1 `SysfsBattery::system()` trips the existing `::system(` needle of `test_rmc_s9_bus_rules_match_the_presence_worker` | CRITICAL | Production constructor renamed `SysfsBattery::kernel()` (contains neither `::system(` nor `::session(` nor any other RMC-S9 needle; checked against the full list at `remote_companion_contract.rs:1148-1180`). Test 34 now pins `SysfsBattery::kernel()` and the absence of `SysfsBattery::system` anywhere in `crates/remote/src`. RMC-S9 added to the §7 list of existing invariants covering `battery.rs`, with its full needle list declared binding on every battery line. The ADR does not name the constructor, so it needs no change | B-4, §1.1, §5.1, §7, test 34 |
| R2-2 cache boundary contradicts test 28; test 25 never reaches step 5 | MAJOR | §5.5 pins a private `CacheRule`: `MaxAge` reuses iff `age <= BATTERY_SAMPLE_INTERVAL_MS` (inclusive, computed with `saturating_duration_since`), `StoredSince(start)` for the sampler; the ADR wording "at most 5 s old" is kept. Test 28 asserts both sides: hit at exactly the interval (`calls == 1`), miss one millisecond later (`calls == 2`). Test 25 advances `BATTERY_SAMPLE_INTERVAL_MS + 1` before its second GET (which can then only be answered by step 5, `calls == 1`) and before its final GET; its stream phase is closed before the final step so the sampler cannot add reads; the gated source also reports when a call returned. Tests 23, 24 (sampler path) and 38 (500 ms age) re-checked and annotated; §10 time model gains a "cache boundary" rule | §5.5, §10 time model, tests 23, 24, 25, 28, 38 |
| R2-3 subscription point of the stream's battery receiver | MINOR | §6.3 now fixes the order: `subscribe()`, then `current().await` with no `.await` in between, then the first write; never subscribe or `borrow_and_update()` after the first battery write; rationale (cache written before `send_replace`) stated. Auditor static constraint, no new test | §6.3 |
| R2-4 test 31 marker `capacity` collides with "funnel capacity reached" | MINOR | Bare `capacity`/`percent` dropped; field-aware forms only (`capacity=`, `capacity:`, `percent=`, `percent:`, `"percent":`, `external_power=`, `external_power:`, `charge=`), plus the charge words, value markers and `BATMARKER0`; the tester greps existing fixed log text before freezing the list | test 31 |
| R2-5 `shutdown_timeout` also cuts off other blocking tasks | MINOR | B-13 states the exact `run_service` tail (`let code = runtime.block_on(run(..)); runtime.shutdown_timeout(..); code`), names the other affected tasks (`push.rs:483`, `push.rs:1293`, #345 `camera.rs:374`) and why it is safe (temp file + `fsync` + `rename`, pure encode); Docs §8 says the same; test 39 also pins `shutdown_timeout(` after `block_on(` | B-13, §1.1, §11, test 39 |
| R2-6 stale §12 `app.js` row | MINOR | Row corrected: top-level element block, `openStream`, `closeStream`, `source.onerror`, `render`, `showLogin`; `fetchStatus` untouched; #345 touches none of those functions | §12 |
| R2-7 redundant §8.3 CSS rule | MINOR | Rule dropped; `style.css` unchanged (inherits `tabular-nums` from `.stat-tile .detail`, `style.css:426-430`); removed from the #345 conflict set | §1.1, §8.3 |
| Observations | — | Line references fixed (`harness.rs:820`, `push_server_tests.rs:466`); Docs §8 limitation for a single battery without a readable `capacity` added; `HEAD /api/battery` doing the same bounded read and the extra sampler loop after `notify_one` need no change (as the evaluator found) | §10, §11 |

Unchanged by Round 3: threat model and visibility (B-5, RBS-I1), bounds, confidentiality, on-by-default (B-4
apart from the constructor name), dependencies, the setup-only migrations of §9, the 39-test count, and the
§13 matrix rows (RBS1–RBS12; test numbers unchanged).

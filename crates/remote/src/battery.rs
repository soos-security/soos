//! Live battery level of the PC (ADR 2026-10-07 "Live Battery Level in `soos-remote`",
//! architect spec `AI/architect_spec_remote_battery.md` §5).
//!
//! The reader scans the kernel power-supply class directory (`POWER_SUPPLY_ROOT`, injectable
//! for tests) with bounded, read-only accesses: at most `MAX_POWER_SUPPLIES` entries and
//! `MAX_BATTERIES` system batteries, regular files only, opened with `O_NOFOLLOW |
//! O_NONBLOCK`, at most `MAX_SYSFS_VALUE_BYTES` per value. Only ten allowlisted attributes
//! are ever opened; it never reads identifying attributes (names, serial, model,
//! manufacturer), and entry names never leave this module. Every failure is fail-closed:
//! `unavailable` or an unknown field, never a guessed value, and `no_battery` only after a
//! complete, readable scan.
//!
//! The runtime ([`BatteryRuntime`]) runs each read on the blocking pool under
//! `BATTERY_READ_TIMEOUT_MS`, with at most one read in flight and a single-flight gate shared
//! by every caller (the sampler, `GET /api/battery` and the first frame of a stream), and
//! caches the outcome for at most `BATTERY_SAMPLE_INTERVAL_MS`. The sampler reads every
//! `BATTERY_SAMPLE_INTERVAL_MS` only while at least one stream is open.
//!
//! This module never logs.

use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nix::fcntl::OFlag;
use tokio::sync::{watch, Mutex, Notify};
use tokio::time::{sleep, timeout_at, Instant};

use crate::{
    BATTERY_READ_TIMEOUT_MS, BATTERY_SAMPLE_INTERVAL_MS, MAX_BATTERIES, MAX_POWER_SUPPLIES,
    MAX_POWER_SUPPLY_NAME_LEN, MAX_SYSFS_MICRO_DIGITS, MAX_SYSFS_VALUE_BYTES, POWER_SUPPLY_ROOT,
};

// ---------------------------------------------------------------------------------------
// Public types (spec §5.1)
// ---------------------------------------------------------------------------------------

/// Overall state of the battery view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryState {
    /// `battery_status = false`, or no source wired.
    Disabled,
    /// The power-supply class could not be read completely (or the read timed out).
    Unavailable,
    /// A complete, readable scan found no system battery.
    NoBattery,
    /// At least one system battery is present.
    Present,
}

/// Aggregated kernel `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeStatus {
    /// `Charging`.
    Charging,
    /// `Discharging`.
    Discharging,
    /// `Full`.
    Full,
    /// `Not charging`.
    NotCharging,
    /// `Unknown`, missing or malformed.
    Unknown,
}

/// JSON body of `GET /api/battery` and of every `event: battery`. Exactly four keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct BatteryView {
    /// Overall state.
    pub state: BatteryState,
    /// `Some(0..=100)` only when `Present` and known.
    pub percent: Option<u8>,
    /// `Some` iff `Present`.
    pub charge: Option<ChargeStatus>,
    /// `Some` only when `Present` and at least one non-battery system supply had a valid
    /// `online`.
    pub external_power: Option<bool>,
}

impl BatteryView {
    /// The view of a disabled or unwired battery level.
    #[must_use]
    pub const fn disabled() -> Self {
        Self::without_values(BatteryState::Disabled)
    }

    /// The view of an unreadable power-supply class.
    #[must_use]
    pub const fn unavailable() -> Self {
        Self::without_values(BatteryState::Unavailable)
    }

    /// The view of a PC without a system battery.
    #[must_use]
    pub const fn no_battery() -> Self {
        Self::without_values(BatteryState::NoBattery)
    }

    const fn without_values(state: BatteryState) -> Self {
        Self {
            state,
            percent: None,
            charge: None,
            external_power: None,
        }
    }
}

/// A usable energy or charge pair of one battery (`full > 0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnergyPair {
    /// `energy_now` / `energy_full` (µWh).
    Energy {
        /// `energy_now`.
        now: u64,
        /// `energy_full`.
        full: u64,
    },
    /// `charge_now` / `charge_full` (µAh).
    Charge {
        /// `charge_now`.
        now: u64,
        /// `charge_full`.
        full: u64,
    },
}

/// One included system battery (internal reading, never serialized).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryReading {
    /// Firmware `capacity`, when canonical and at most 100.
    pub capacity: Option<u8>,
    /// Kernel `status`.
    pub status: ChargeStatus,
    /// The usable energy pair, else the usable charge pair.
    pub energy: Option<EnergyPair>,
}

/// Blocking battery source; called only on the blocking pool under
/// `BATTERY_READ_TIMEOUT_MS`.
pub trait BatterySource: Send + Sync + 'static {
    /// One complete reading.
    fn read(&self) -> BatteryView;
}

/// Production source: the power-supply class directory under `root`.
#[derive(Debug, Clone)]
pub struct SysfsBattery {
    root: PathBuf,
}

impl SysfsBattery {
    /// A source over `root` (tests pass a tempdir).
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The kernel class directory `POWER_SUPPLY_ROOT`; the only constructor `main.rs` uses.
    #[must_use]
    pub fn kernel() -> Self {
        Self::new(PathBuf::from(POWER_SUPPLY_ROOT))
    }
}

impl BatterySource for SysfsBattery {
    fn read(&self) -> BatteryView {
        read_power_supplies(&self.root)
    }
}

// ---------------------------------------------------------------------------------------
// Pure parsers (spec §5.2)
// ---------------------------------------------------------------------------------------

/// The value of one attribute: at most `MAX_SYSFS_VALUE_BYTES` bytes, one trailing newline
/// stripped, the rest non-empty printable ASCII with spaces only inside.
fn trim_value(raw: &[u8]) -> Option<&[u8]> {
    if raw.len() > MAX_SYSFS_VALUE_BYTES {
        return None;
    }
    let value = raw.strip_suffix(b"\n").unwrap_or(raw);
    let printable = |b: &u8| (0x21..=0x7e).contains(b);
    let (first, last) = (value.first()?, value.last()?);
    if !printable(first) || !printable(last) {
        return None;
    }
    value
        .iter()
        .all(|b| *b == b' ' || printable(b))
        .then_some(value)
}

/// The decimal value of 1..=`max_digits` ASCII digits; `None` on any other byte or overflow.
fn decimal(digits: &[u8], max_digits: usize) -> Option<u64> {
    if digits.is_empty() || digits.len() > max_digits {
        return None;
    }
    let mut value: u64 = 0;
    for byte in digits {
        let digit = char::from(*byte).to_digit(10)?;
        value = value.checked_mul(10)?.checked_add(u64::from(digit))?;
    }
    Some(value)
}

/// `capacity`: the canonical decimal (no leading zero unless `0`) of a value at most 100.
#[must_use]
pub fn parse_capacity(raw: &[u8]) -> Option<u8> {
    let value = trim_value(raw)?;
    if value.len() > 1 && value.first() == Some(&b'0') {
        return None;
    }
    let parsed = decimal(value, 3)?;
    if parsed > 100 {
        return None;
    }
    u8::try_from(parsed).ok()
}

/// `status`: exactly one of the five kernel strings, anything else `Unknown`.
#[must_use]
pub fn parse_status(raw: &[u8]) -> ChargeStatus {
    match trim_value(raw) {
        Some(b"Charging") => ChargeStatus::Charging,
        Some(b"Discharging") => ChargeStatus::Discharging,
        Some(b"Full") => ChargeStatus::Full,
        Some(b"Not charging") => ChargeStatus::NotCharging,
        _ => ChargeStatus::Unknown,
    }
}

/// `online`: `0` offline, `1` (Online Fixed) and `2` (Online Programmable) online, anything
/// else unknown (kernel ABI `sysfs-class-power`).
#[must_use]
pub fn parse_online(raw: &[u8]) -> Option<bool> {
    match trim_value(raw) {
        Some(b"0") => Some(false),
        Some(b"1" | b"2") => Some(true),
        _ => None,
    }
}

/// `energy_*` / `charge_*`: 1..=`MAX_SYSFS_MICRO_DIGITS` ASCII digits (leading zeros
/// accepted).
#[must_use]
pub fn parse_micro(raw: &[u8]) -> Option<u64> {
    decimal(trim_value(raw)?, MAX_SYSFS_MICRO_DIGITS)
}

/// `scope`: absent, `System` or `Unknown` are included; `Device` or malformed are excluded.
#[must_use]
pub fn parse_scope_included(raw: Option<&[u8]>) -> bool {
    match raw {
        None => true,
        Some(raw) => matches!(trim_value(raw), Some(b"System" | b"Unknown")),
    }
}

/// `present`: only an exact `0` is an empty bay; absent, `1` or malformed count as present.
#[must_use]
pub fn parse_present(raw: Option<&[u8]>) -> bool {
    !matches!(raw.and_then(trim_value), Some(b"0"))
}

/// A class entry name: 1..=`MAX_POWER_SUPPLY_NAME_LEN` bytes of `[A-Za-z0-9_.:-]`, not
/// starting with `.`.
#[must_use]
pub fn valid_supply_name(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_POWER_SUPPLY_NAME_LEN
        && bytes.first() != Some(&b'.')
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

// ---------------------------------------------------------------------------------------
// Aggregation (spec §5.3)
// ---------------------------------------------------------------------------------------

/// Pure aggregation of the included batteries and the `online` values of the other system
/// supplies.
#[must_use]
pub fn aggregate(batteries: &[BatteryReading], online: &[bool]) -> BatteryView {
    if batteries.is_empty() {
        return BatteryView::no_battery();
    }
    if batteries.len() > MAX_BATTERIES {
        return BatteryView::unavailable();
    }
    let percent = match batteries {
        [single] => single.capacity,
        several => weighted_percent(several).or_else(|| mean_capacity(several)),
    };
    let external_power = if online.contains(&true) {
        Some(true)
    } else if online.is_empty() {
        None
    } else {
        Some(false)
    };
    BatteryView {
        state: BatteryState::Present,
        percent,
        charge: Some(aggregate_charge(batteries)),
        external_power,
    }
}

/// Energy-weighted percent when every battery has a usable pair of the same kind; `None` on
/// a missing pair, mixed kinds or any overflow.
fn weighted_percent(batteries: &[BatteryReading]) -> Option<u8> {
    let mut kind_is_energy: Option<bool> = None;
    let mut sum_now: u64 = 0;
    let mut sum_full: u64 = 0;
    for battery in batteries {
        let (is_energy, now, full) = match battery.energy? {
            EnergyPair::Energy { now, full } => (true, now, full),
            EnergyPair::Charge { now, full } => (false, now, full),
        };
        match kind_is_energy {
            None => kind_is_energy = Some(is_energy),
            Some(kind) if kind != is_energy => return None,
            Some(_) => {}
        }
        sum_now = sum_now.checked_add(now)?;
        sum_full = sum_full.checked_add(full)?;
    }
    let ratio = sum_now.checked_mul(100)?.checked_div(sum_full)?;
    u8::try_from(ratio.min(100)).ok()
}

/// Floor mean of the capacities; `None` if any capacity is unknown.
fn mean_capacity(batteries: &[BatteryReading]) -> Option<u8> {
    let mut sum: u32 = 0;
    for battery in batteries {
        sum = sum.checked_add(u32::from(battery.capacity?))?;
    }
    let len = u32::try_from(batteries.len()).ok()?;
    u8::try_from(sum.checked_div(len)?).ok()
}

/// Charging > Discharging > all Full > Not charging > Unknown.
fn aggregate_charge(batteries: &[BatteryReading]) -> ChargeStatus {
    let any = |status: ChargeStatus| batteries.iter().any(|b| b.status == status);
    if any(ChargeStatus::Charging) {
        ChargeStatus::Charging
    } else if any(ChargeStatus::Discharging) {
        ChargeStatus::Discharging
    } else if batteries.iter().all(|b| b.status == ChargeStatus::Full) {
        ChargeStatus::Full
    } else if any(ChargeStatus::NotCharging) {
        ChargeStatus::NotCharging
    } else {
        ChargeStatus::Unknown
    }
}

// ---------------------------------------------------------------------------------------
// Blocking sysfs reader (spec §5.4)
// ---------------------------------------------------------------------------------------

/// One attribute of `dir`: regular file only (`O_NOFOLLOW | O_NONBLOCK`, `fstat`), at most
/// `MAX_SYSFS_VALUE_BYTES` bytes; any error or a larger value → `None`.
fn read_attr(dir: &Path, attr: &str) -> Option<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC).bits())
        .open(dir.join(attr))
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let limit = MAX_SYSFS_VALUE_BYTES.saturating_add(1);
    let mut buf = Vec::with_capacity(limit);
    file.take(u64::try_from(limit).unwrap_or(u64::MAX))
        .read_to_end(&mut buf)
        .ok()?;
    (buf.len() <= MAX_SYSFS_VALUE_BYTES).then_some(buf)
}

/// Both values of a pair parsed, `full > 0`.
fn usable_pair(now: Option<Vec<u8>>, full: Option<Vec<u8>>) -> Option<(u64, u64)> {
    let now = parse_micro(&now?)?;
    let full = parse_micro(&full?)?;
    (full > 0).then_some((now, full))
}

/// The energy pair of a battery directory, else its charge pair.
fn energy_pair(dir: &Path) -> Option<EnergyPair> {
    if let Some((now, full)) =
        usable_pair(read_attr(dir, "energy_now"), read_attr(dir, "energy_full"))
    {
        return Some(EnergyPair::Energy { now, full });
    }
    usable_pair(read_attr(dir, "charge_now"), read_attr(dir, "charge_full"))
        .map(|(now, full)| EnergyPair::Charge { now, full })
}

/// Blocking scan of the class directory `root` (std only); see the module documentation.
#[must_use]
pub fn read_power_supplies(root: &Path) -> BatteryView {
    let Ok(entries) = std::fs::read_dir(root) else {
        return BatteryView::unavailable();
    };
    let mut examined: usize = 0;
    let mut type_unknown = false;
    let mut batteries: Vec<BatteryReading> = Vec::with_capacity(MAX_BATTERIES);
    let mut online: Vec<bool> = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            return BatteryView::unavailable();
        };
        examined = examined.saturating_add(1);
        if examined > MAX_POWER_SUPPLIES {
            return BatteryView::unavailable();
        }
        let name = entry.file_name();
        if !valid_supply_name(&name) {
            type_unknown = true;
            continue;
        }
        let dir = root.join(&name);
        let kind_raw = read_attr(&dir, "type");
        let Some(kind) = kind_raw.as_deref().and_then(trim_value) else {
            type_unknown = true;
            continue;
        };
        if !parse_scope_included(read_attr(&dir, "scope").as_deref()) {
            continue;
        }
        if kind == b"Battery" {
            if !parse_present(read_attr(&dir, "present").as_deref()) {
                continue;
            }
            if batteries.len() >= MAX_BATTERIES {
                return BatteryView::unavailable();
            }
            batteries.push(BatteryReading {
                capacity: read_attr(&dir, "capacity")
                    .as_deref()
                    .and_then(parse_capacity),
                status: read_attr(&dir, "status")
                    .as_deref()
                    .map_or(ChargeStatus::Unknown, parse_status),
                energy: energy_pair(&dir),
            });
        } else if let Some(value) = read_attr(&dir, "online").as_deref().and_then(parse_online) {
            online.push(value);
        }
    }
    if batteries.is_empty() && type_unknown {
        return BatteryView::unavailable();
    }
    aggregate(&batteries, &online)
}

// ---------------------------------------------------------------------------------------
// Runtime (spec §5.5): single-flight, bounded, cached
// ---------------------------------------------------------------------------------------

/// Private result of one coalesced read attempt.
enum ReadOutcome {
    /// An actual outcome (fresh, cached, timed out, panicked or still stuck).
    View(BatteryView),
    /// The gate could not be taken before the deadline; never cached nor published.
    GateTimeout,
}

/// Private cache-reuse rule of one `read_coalesced` call.
#[derive(Clone, Copy)]
enum CacheRule {
    /// Reuse iff the entry is at most `BATTERY_SAMPLE_INTERVAL_MS` old (inclusive).
    MaxAge,
    /// Reuse iff the entry was stored at or after this instant.
    StoredSince(Instant),
}

impl CacheRule {
    fn accepts(self, stored_at: Instant) -> bool {
        match self {
            Self::MaxAge => {
                Instant::now().saturating_duration_since(stored_at)
                    <= Duration::from_millis(BATTERY_SAMPLE_INTERVAL_MS)
            }
            Self::StoredSince(start) => stored_at >= start,
        }
    }
}

/// Clears the in-flight flag when dropped (also on a panic of the source or when the
/// blocking task never runs).
struct InFlightGuard(Arc<AtomicBool>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Shared battery runtime of one `serve` call.
pub struct BatteryRuntime {
    source: Arc<dyn BatterySource>,
    /// Set while a blocking read runs; stays set after a timeout until that thread returns.
    in_flight: Arc<AtomicBool>,
    /// Single-flight gate, held across one whole bounded read (never across a socket write
    /// or a send).
    read_gate: Mutex<()>,
    /// Last read outcome and when it was stored; locked only to copy or replace it.
    cache: Mutex<Option<(Instant, BatteryView)>>,
    /// Latest sampled view while streams exist; `None` when idle.
    views: watch::Sender<Option<BatteryView>>,
    /// Open streams.
    streams: AtomicUsize,
    /// Wakes the sampler when a stream registers (permit-storing).
    wake: Notify,
}

impl BatteryRuntime {
    /// A runtime over `source`, with an empty cache and no stream.
    #[must_use]
    pub fn new(source: Arc<dyn BatterySource>) -> Self {
        Self {
            source,
            in_flight: Arc::new(AtomicBool::new(false)),
            read_gate: Mutex::new(()),
            cache: Mutex::new(None),
            views: watch::Sender::new(None),
            streams: AtomicUsize::new(0),
            wake: Notify::new(),
        }
    }

    /// For HTTP callers and the first frame of a stream: a cached entry at most
    /// `BATTERY_SAMPLE_INTERVAL_MS` old, else the one coalesced read; a gate-wait timeout is
    /// `unavailable` (returned, never cached nor published).
    pub async fn current(&self) -> BatteryView {
        match self.read_coalesced(CacheRule::MaxAge).await {
            ReadOutcome::View(view) => view,
            ReadOutcome::GateTimeout => BatteryView::unavailable(),
        }
    }

    /// For the sampler: only an entry stored at or after the call start is reused; a
    /// gate-wait timeout is `None` (nothing published).
    pub async fn sample(&self) -> Option<BatteryView> {
        match self
            .read_coalesced(CacheRule::StoredSince(Instant::now()))
            .await
        {
            ReadOutcome::View(view) => Some(view),
            ReadOutcome::GateTimeout => None,
        }
    }

    /// A receiver of the sampled views.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Option<BatteryView>> {
        self.views.subscribe()
    }

    /// Registers one open stream and wakes the sampler; the guard deregisters on drop.
    #[must_use]
    pub fn register_stream(self: &Arc<Self>) -> BatteryStreamGuard {
        let registered = self
            .streams
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
            .is_ok();
        self.wake.notify_one();
        BatteryStreamGuard {
            runtime: Arc::clone(self),
            registered,
        }
    }

    /// The cached view if `rule` accepts it.
    async fn cached(&self, rule: CacheRule) -> Option<BatteryView> {
        let entry = *self.cache.lock().await;
        entry.and_then(|(stored_at, view)| rule.accepts(stored_at).then_some(view))
    }

    /// One deadline for the gate wait and the read; the cache is re-checked under the gate
    /// (coalescing); at most one blocking read in flight.
    async fn read_coalesced(&self, rule: CacheRule) -> ReadOutcome {
        let start = Instant::now();
        let deadline = start
            .checked_add(Duration::from_millis(BATTERY_READ_TIMEOUT_MS))
            .unwrap_or(start);
        if let Some(view) = self.cached(rule).await {
            return ReadOutcome::View(view);
        }
        let Ok(gate) = timeout_at(deadline, self.read_gate.lock()).await else {
            return ReadOutcome::GateTimeout;
        };
        if let Some(view) = self.cached(rule).await {
            return ReadOutcome::View(view);
        }
        let view = if self
            .in_flight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            // An earlier read timed out and its thread is still stuck: no second thread.
            BatteryView::unavailable()
        } else {
            let guard = InFlightGuard(Arc::clone(&self.in_flight));
            let source = Arc::clone(&self.source);
            let handle = tokio::task::spawn_blocking(move || {
                let _guard = guard;
                source.read()
            });
            match timeout_at(deadline, handle).await {
                Ok(Ok(view)) => view,
                Ok(Err(_)) | Err(_) => BatteryView::unavailable(),
            }
        };
        *self.cache.lock().await = Some((Instant::now(), view));
        drop(gate);
        ReadOutcome::View(view)
    }
}

/// Registration of one open stream; deregisters (saturating) on drop.
pub struct BatteryStreamGuard {
    runtime: Arc<BatteryRuntime>,
    registered: bool,
}

impl Drop for BatteryStreamGuard {
    fn drop(&mut self) {
        if self.registered {
            let _ = self
                .runtime
                .streams
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                    Some(n.saturating_sub(1))
                });
        }
    }
}

/// Never returns. While no stream is open it publishes `None` and waits for a registration;
/// otherwise it samples, publishes a fresh view and sleeps `BATTERY_SAMPLE_INTERVAL_MS`.
pub async fn run_sampler(runtime: Arc<BatteryRuntime>) {
    let interval = Duration::from_millis(BATTERY_SAMPLE_INTERVAL_MS);
    loop {
        if runtime.streams.load(Ordering::SeqCst) == 0 {
            runtime.views.send_replace(None);
            let mut notified = pin!(runtime.wake.notified());
            notified.as_mut().enable();
            if runtime.streams.load(Ordering::SeqCst) == 0 {
                notified.await;
            }
            continue;
        }
        if let Some(view) = runtime.sample().await {
            runtime.views.send_replace(Some(view));
        }
        sleep(interval).await;
    }
}

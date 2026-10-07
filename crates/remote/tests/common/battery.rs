//! Shared battery test doubles of GitHub #346 (ADR 2026-10-07 "Live Battery Level in
//! `soos-remote`", architect spec `AI/architect_spec_remote_battery.md` §10 helpers):
//!
//! - [`FakeSysfs`]: a tempdir standing for `/sys/class/power_supply` (plain directories, one
//!   per supply, one file per attribute); never the real `/sys`;
//! - [`ScriptedBattery`]: a [`BatterySource`] returning a settable view and counting calls,
//!   in four modes (immediate, delayed by a fixed real-time duration, gated until a
//!   [`GateRelease`] guard is dropped, panicking);
//! - [`Tracked`]: wraps any source and counts started and finished reads, so a test can wait
//!   (real time) until no read runs before it moves the virtual clock.
//!
//! Include with `#[path = "common/battery.rs"] mod battery;`.

#![allow(
    dead_code,
    unused_imports,
    reason = "Shared test fixtures library used conditionally across test modules"
)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use tempfile::TempDir;

use soos_remote::battery::{BatterySource, BatteryState, BatteryView, ChargeStatus, SysfsBattery};

/// Real-time fallback after which a gated read returns anyway (a broken test, not a slow CI).
pub const GATE_FALLBACK: Duration = Duration::from_secs(10);
/// Real-time bound of every "wait until the blocking pool is done" helper.
pub const REAL_WAIT: Duration = Duration::from_secs(2);

/// A present view (`percent`, `charge`, `external_power`).
pub fn present(percent: u8, charge: ChargeStatus, external_power: Option<bool>) -> BatteryView {
    BatteryView {
        state: BatteryState::Present,
        percent: Some(percent),
        charge: Some(charge),
        external_power,
    }
}

// ---------------------------------------------------------------------------------------
// FakeSysfs
// ---------------------------------------------------------------------------------------

/// A fake power-supply class directory in a tempdir.
pub struct FakeSysfs {
    dir: TempDir,
    /// Holds symlink targets outside the class directory.
    outside: TempDir,
}

impl Default for FakeSysfs {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSysfs {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            outside: tempfile::tempdir().unwrap(),
        }
    }

    /// The class directory (the injectable root).
    pub fn root(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    /// A production source reading this root.
    pub fn source(&self) -> SysfsBattery {
        SysfsBattery::new(self.root())
    }

    /// Reads this root through the production reader.
    pub fn read(&self) -> BatteryView {
        self.source().read()
    }

    fn entry(&self, name: &OsStr) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Creates the entry `name` (raw bytes allowed) holding `attrs`.
    pub fn raw_entry(&self, name: &[u8], attrs: &[(&str, &[u8])]) -> PathBuf {
        let dir = self.entry(OsStr::from_bytes(name));
        fs::create_dir_all(&dir).unwrap();
        for (attr, value) in attrs {
            fs::write(dir.join(attr), value).unwrap();
        }
        dir
    }

    /// A supply of `type` `kind` (written as `kind\n`) plus `attrs`.
    pub fn supply(&self, name: &str, kind: &str, attrs: &[(&str, &[u8])]) -> PathBuf {
        let dir = self.raw_entry(name.as_bytes(), attrs);
        fs::write(dir.join("type"), format!("{kind}\n")).unwrap();
        dir
    }

    /// A system battery (`type` `Battery`) plus `attrs`.
    pub fn battery(&self, name: &str, attrs: &[(&str, &[u8])]) -> PathBuf {
        self.supply(name, "Battery", attrs)
    }

    /// A `Mains` supply with `online` = `online` (`0`/`1`/…).
    pub fn mains(&self, name: &str, online: &str) -> PathBuf {
        self.supply(
            name,
            "Mains",
            &[("online", format!("{online}\n").as_bytes())],
        )
    }

    /// Replaces (or creates) one attribute of an existing entry.
    pub fn set(&self, name: &str, attr: &str, value: &[u8]) {
        let path = self.entry(OsStr::new(name)).join(attr);
        let _ = fs::remove_file(&path);
        fs::write(path, value).unwrap();
    }

    /// Removes one attribute of an existing entry.
    pub fn remove(&self, name: &str, attr: &str) {
        let path = self.entry(OsStr::new(name)).join(attr);
        let _ = fs::remove_file(path);
    }

    /// Makes `attr` of `name` a FIFO (a reader that opens it without `O_NONBLOCK` blocks).
    pub fn fifo(&self, name: &str, attr: &str) -> PathBuf {
        let path = self.entry(OsStr::new(name)).join(attr);
        let _ = fs::remove_file(&path);
        nix::unistd::mkfifo(&path, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
        path
    }

    /// Makes `attr` of `name` a directory.
    pub fn dir_attr(&self, name: &str, attr: &str) {
        let path = self.entry(OsStr::new(name)).join(attr);
        let _ = fs::remove_file(&path);
        fs::create_dir_all(path).unwrap();
    }

    /// Makes `attr` of `name` a symlink to a regular file outside the root holding
    /// `target_value`.
    pub fn symlink_attr(&self, name: &str, attr: &str, target_value: &[u8]) {
        let target = self.outside.path().join(format!("{name}-{attr}"));
        fs::write(&target, target_value).unwrap();
        let path = self.entry(OsStr::new(name)).join(attr);
        let _ = fs::remove_file(&path);
        std::os::unix::fs::symlink(&target, path).unwrap();
    }

    /// Sets the mode of the root itself.
    pub fn chmod_root(&self, mode: u32) {
        fs::set_permissions(self.dir.path(), fs::Permissions::from_mode(mode)).unwrap();
    }
}

impl Drop for FakeSysfs {
    fn drop(&mut self) {
        // A test may have made the root unreadable; restore it so the tempdir is removed.
        let _ = fs::set_permissions(self.dir.path(), fs::Permissions::from_mode(0o700));
    }
}

// ---------------------------------------------------------------------------------------
// ScriptedBattery
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Immediate,
    /// Blocks the calling (blocking-pool) thread for this real-time duration, then returns.
    Delayed(Duration),
    /// Blocks until the gate opens (a hung driver).
    Gated,
    /// Panics inside `read`.
    Panic,
}

struct ScriptInner {
    view: Mutex<BatteryView>,
    mode: Mutex<Mode>,
    calls: AtomicUsize,
    entered: AtomicUsize,
    returned: AtomicUsize,
    gate_open: Mutex<bool>,
    gate: Condvar,
}

/// A scripted [`BatterySource`]; clones share their state.
#[derive(Clone)]
pub struct ScriptedBattery(Arc<ScriptInner>);

impl ScriptedBattery {
    pub fn new(view: BatteryView) -> Self {
        Self(Arc::new(ScriptInner {
            view: Mutex::new(view),
            mode: Mutex::new(Mode::Immediate),
            calls: AtomicUsize::new(0),
            entered: AtomicUsize::new(0),
            returned: AtomicUsize::new(0),
            gate_open: Mutex::new(true),
            gate: Condvar::new(),
        }))
    }

    pub fn delayed(view: BatteryView, delay: Duration) -> Self {
        let source = Self::new(view);
        source.set_mode(Mode::Delayed(delay));
        source
    }

    pub fn panicking() -> Self {
        let source = Self::new(BatteryView::unavailable());
        source.set_mode(Mode::Panic);
        source
    }

    pub fn set_view(&self, view: BatteryView) {
        *self.0.view.lock().unwrap() = view;
    }

    pub fn set_mode(&self, mode: Mode) {
        *self.0.mode.lock().unwrap() = mode;
    }

    /// Closes the gate: every later `read` blocks until the returned guard is dropped (also
    /// when an assertion unwinds), after which reads are immediate again.
    #[must_use]
    pub fn gate(&self) -> GateRelease {
        *self.0.gate_open.lock().unwrap() = false;
        self.set_mode(Mode::Gated);
        GateRelease(self.clone())
    }

    /// Calls of `read` so far.
    pub fn calls(&self) -> usize {
        self.0.calls.load(Ordering::SeqCst)
    }

    /// Calls that entered `read`.
    pub fn entered(&self) -> usize {
        self.0.entered.load(Ordering::SeqCst)
    }

    /// Calls that returned from `read` (a panicking call never returns).
    pub fn returned(&self) -> usize {
        self.0.returned.load(Ordering::SeqCst)
    }

    /// Waits in real time (never moving the clock) until `entered() >= n`.
    pub fn wait_entered(&self, n: usize) {
        let start = Instant::now();
        while self.entered() < n {
            assert!(
                start.elapsed() < REAL_WAIT,
                "the source never saw call {n} (entered {})",
                self.entered()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Waits in real time until `returned() >= n`.
    pub fn wait_returned(&self, n: usize) {
        let start = Instant::now();
        while self.returned() < n {
            assert!(
                start.elapsed() < REAL_WAIT,
                "call {n} never returned (returned {})",
                self.returned()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl BatterySource for ScriptedBattery {
    fn read(&self) -> BatteryView {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.entered.fetch_add(1, Ordering::SeqCst);
        let mode = *self.0.mode.lock().unwrap();
        match mode {
            Mode::Immediate => {}
            Mode::Delayed(delay) => std::thread::sleep(delay),
            Mode::Gated => {
                let mut open = self.0.gate_open.lock().unwrap();
                let start = Instant::now();
                while !*open && start.elapsed() < GATE_FALLBACK {
                    open = self
                        .0
                        .gate
                        .wait_timeout(open, Duration::from_millis(50))
                        .unwrap()
                        .0;
                }
            }
            Mode::Panic => panic!("scripted battery source panics (test double)"),
        }
        let view = *self.0.view.lock().unwrap();
        self.0.returned.fetch_add(1, Ordering::SeqCst);
        view
    }
}

/// Opens the gate of a [`ScriptedBattery`] on drop and makes later reads immediate.
pub struct GateRelease(ScriptedBattery);

impl Drop for GateRelease {
    fn drop(&mut self) {
        self.0.set_mode(Mode::Immediate);
        let mut open = self.0 .0.gate_open.lock().unwrap();
        *open = true;
        self.0 .0.gate.notify_all();
    }
}

// ---------------------------------------------------------------------------------------
// Tracked
// ---------------------------------------------------------------------------------------

struct Counters {
    started: AtomicUsize,
    finished: AtomicUsize,
}

/// Wraps a source and counts started and finished reads; clones share the counters.
pub struct Tracked<S> {
    inner: S,
    counters: Arc<Counters>,
}

impl<S> Tracked<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            counters: Arc::new(Counters {
                started: AtomicUsize::new(0),
                finished: AtomicUsize::new(0),
            }),
        }
    }

    /// A handle observing the counters.
    pub fn probe(&self) -> Probe {
        Probe(Arc::clone(&self.counters))
    }
}

impl<S: BatterySource> BatterySource for Tracked<S> {
    fn read(&self) -> BatteryView {
        self.counters.started.fetch_add(1, Ordering::SeqCst);
        let view = self.inner.read();
        self.counters.finished.fetch_add(1, Ordering::SeqCst);
        view
    }
}

/// Observes the counters of a [`Tracked`] source.
#[derive(Clone)]
pub struct Probe(Arc<Counters>);

impl Probe {
    pub fn started(&self) -> usize {
        self.0.started.load(Ordering::SeqCst)
    }

    pub fn finished(&self) -> usize {
        self.0.finished.load(Ordering::SeqCst)
    }

    /// No read is running right now.
    pub fn idle(&self) -> bool {
        self.started() == self.finished()
    }
}

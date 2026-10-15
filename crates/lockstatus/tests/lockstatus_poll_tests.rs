//! Contract tests of the polling cadence, back-off and `--watch` loop (GitHub #401; matrix LKA5).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::cell::Cell;
use std::io::Write;
use std::time::{Duration, Instant};

use common::{my_uid, runtime_dir, Behavior, MockDaemon};
use soos_lockstatus::poll::{clamp_interval_ms, watch, Backoff, Poller};
use soos_lockstatus::view::{LockView, ViewState};
use soos_lockstatus::{
    DEFAULT_POLL_INTERVAL_MS, ERROR_BACKOFF_MS, MAX_JSON_LINE_BYTES, MAX_POLL_INTERVAL_MS,
    MIN_POLL_INTERVAL_MS,
};
use soos_protocol::LockWidgetState;

#[test]
fn test_lka5_interval_constants_and_clamp() {
    assert_eq!(DEFAULT_POLL_INTERVAL_MS, 500);
    assert_eq!(MIN_POLL_INTERVAL_MS, 250);
    assert_eq!(MAX_POLL_INTERVAL_MS, 2000);
    assert_eq!(ERROR_BACKOFF_MS, [1000, 2000, 4000]);
    assert_eq!(clamp_interval_ms(0), 250);
    assert_eq!(clamp_interval_ms(1), 250);
    assert_eq!(clamp_interval_ms(249), 250);
    assert_eq!(clamp_interval_ms(250), 250);
    assert_eq!(clamp_interval_ms(700), 700);
    assert_eq!(clamp_interval_ms(2000), 2000);
    assert_eq!(clamp_interval_ms(2001), 2000);
    assert_eq!(clamp_interval_ms(u64::MAX), 2000);
}

#[test]
fn test_lka5_backoff_steps_and_reset() {
    let interval = Duration::from_millis(500);
    let mut b = Backoff::default();
    assert_eq!(b.next_delay(interval, true), interval);
    assert_eq!(b.next_delay(interval, false), Duration::from_millis(1000));
    assert_eq!(b.next_delay(interval, false), Duration::from_millis(2000));
    assert_eq!(b.next_delay(interval, false), Duration::from_millis(4000));
    assert_eq!(b.next_delay(interval, false), Duration::from_millis(4000));
    assert_eq!(b.next_delay(interval, true), interval);
    assert_eq!(b.next_delay(interval, false), Duration::from_millis(1000));
    // Never shorter than the interval.
    let long = Duration::from_millis(2000);
    let mut b = Backoff::default();
    assert_eq!(b.next_delay(long, false), long);
    assert_eq!(b.next_delay(long, false), long);
    assert_eq!(b.next_delay(long, false), Duration::from_millis(4000));
}

#[test]
fn test_lka5_poller_clamps_and_polls_through_the_shared_path() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::NotRecognized, 8));
    let dir = runtime_dir();
    let mut poller = Poller::new(
        daemon.path.clone(),
        Some(dir.path().to_path_buf()),
        my_uid(),
        0,
    );
    assert_eq!(poller.interval(), Duration::from_millis(250));
    let (view, delay) = poller.poll();
    assert_eq!(
        view,
        LockView {
            state: ViewState::NotRecognized,
            sequence: 8
        }
    );
    assert_eq!(delay, Duration::from_millis(250));
    assert!(dir
        .path()
        .join(soos_lockstatus::shared::LOCK_FILE_NAME)
        .exists());
}

#[test]
fn test_lka5_poller_backs_off_on_daemon_failures_only() {
    let daemon = MockDaemon::start(Behavior::Close);
    let dir = runtime_dir();
    let mut poller = Poller::new(
        daemon.path.clone(),
        Some(dir.path().to_path_buf()),
        my_uid(),
        500,
    );
    let (view, delay) = poller.poll();
    assert_eq!(view, LockView::UNAVAILABLE);
    assert_eq!(delay, Duration::from_millis(1000));
    // Without a runtime directory the daemon is never contacted: not a daemon failure.
    let mut quiet = Poller::new(daemon.path.clone(), None, my_uid(), 500);
    let before = daemon.connections();
    for _ in 0..3 {
        let (view, delay) = quiet.poll();
        assert_eq!(view, LockView::UNAVAILABLE);
        assert_eq!(delay, Duration::from_millis(500));
    }
    assert_eq!(daemon.connections(), before);
}

/// Writer that fails after `limit` lines (a closed pipe).
struct Lines {
    lines: Vec<String>,
    limit: usize,
    buf: Vec<u8>,
}

impl Write for Lines {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.lines.len() >= self.limit {
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        self.buf.extend_from_slice(data);
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            self.lines.push(String::from_utf8(line).unwrap());
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn test_lka5_watch_emits_first_view_and_changes_only() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Idle, 0));
    let dir = runtime_dir();
    let mut poller = Poller::new(
        daemon.path.clone(),
        Some(dir.path().to_path_buf()),
        my_uid(),
        250,
    );
    let mut out = Lines {
        lines: Vec::new(),
        limit: 3,
        buf: Vec::new(),
    };
    let polls = Cell::new(0u32);
    let start = Instant::now();
    let script = |n: u32| match n {
        0..=2 => Behavior::Answer(LockWidgetState::Idle, 0),
        3..=4 => Behavior::Answer(LockWidgetState::Scanning, 1),
        5..=7 => Behavior::Answer(LockWidgetState::NotRecognized, 2),
        // A fourth change: its write hits the closed pipe.
        _ => Behavior::Answer(LockWidgetState::Idle, 2),
    };
    let result = watch(&mut poller, &mut out, &|| {
        let n = polls.get();
        daemon.set_behavior(script(n));
        polls.set(n + 1);
        start.elapsed() < Duration::from_secs(10)
    });
    assert!(
        result.is_err(),
        "the closed pipe ends the loop with an error"
    );
    assert_eq!(
        out.lines,
        [
            "{\"state\":\"idle\",\"available\":true,\"sequence\":0}\n",
            "{\"state\":\"scanning\",\"available\":true,\"sequence\":1}\n",
            "{\"state\":\"not_recognized\",\"available\":true,\"sequence\":2}\n",
        ]
    );
    for line in &out.lines {
        assert!(line.len() <= MAX_JSON_LINE_BYTES);
    }
}

#[test]
fn test_lka5_watch_never_exceeds_four_hertz() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Idle, 0));
    let dir = runtime_dir();
    let mut poller = Poller::new(
        daemon.path.clone(),
        Some(dir.path().to_path_buf()),
        my_uid(),
        0,
    );
    let mut sink = Vec::new();
    let start = Instant::now();
    watch(&mut poller, &mut sink, &|| {
        start.elapsed() < Duration::from_millis(1100)
    })
    .unwrap();
    let requests = daemon.requests();
    // 1.1 s at 250 ms: at most 5 polls (t = 0, 250, 500, 750, 1000).
    assert!(requests >= 2 && requests <= 5, "{requests}");
    assert_eq!(String::from_utf8(sink).unwrap().lines().count(), 1);
}

#[test]
fn test_lka5_watch_stops_when_asked() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 4));
    let dir = runtime_dir();
    let mut poller = Poller::new(
        daemon.path.clone(),
        Some(dir.path().to_path_buf()),
        my_uid(),
        250,
    );
    let mut sink = Vec::new();
    watch(&mut poller, &mut sink, &|| false).unwrap();
    assert!(sink.is_empty());
    assert_eq!(daemon.requests(), 0);
}

#[test]
fn test_lka5_poller_keeps_the_last_view_while_the_lock_is_busy() {
    use std::os::unix::fs::OpenOptionsExt;
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 5));
    let dir = runtime_dir();
    let mut poller = Poller::new(
        daemon.path.clone(),
        Some(dir.path().to_path_buf()),
        my_uid(),
        250,
    );
    let scanning = LockView {
        state: ViewState::Scanning,
        sequence: 5,
    };
    assert_eq!(poller.poll().0, scanning);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .mode(0o600)
        .open(dir.path().join(soos_lockstatus::shared::LOCK_FILE_NAME))
        .unwrap();
    let held = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let before = daemon.requests();
    let (view, delay) = poller.poll();
    assert_eq!(view, scanning, "a busy lock keeps the last view");
    assert_eq!(
        delay,
        Duration::from_millis(250),
        "a busy lock is not a daemon failure"
    );
    assert_eq!(daemon.requests(), before);
    drop(held);
    // Before any answer, a busy lock reads unavailable.
    let mut fresh = Poller::new(
        daemon.path.clone(),
        Some(dir.path().to_path_buf()),
        my_uid(),
        250,
    );
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join(soos_lockstatus::shared::LOCK_FILE_NAME))
        .unwrap();
    let held = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fresh.poll().0, LockView::UNAVAILABLE);
    drop(held);
}

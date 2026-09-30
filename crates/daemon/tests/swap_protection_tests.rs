//! Swap-protection outcome contract (GitHub #201, review finding DMN-12).
//!
//! `mlockall` is the only page-locking layer wired in production. Its outcome must be visible:
//! a refusal is logged at `warn` level (never `debug`) and recorded in [`HealthState`] so
//! diagnostics can report it; success is logged at `info` level.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Test suite assertions"
)]

use std::io::Write;
use std::sync::{Arc, Mutex};

use soos_daemon::mlock::{enable_swap_protection, record_swap_protection};
use soos_daemon::HealthState;

/// Thread-safe in-memory log sink for a scoped tracing subscriber.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Runs `f` under a subscriber that only records events at `max_level` or above.
fn capture_at(max_level: tracing::Level, f: impl FnOnce()) -> String {
    let sink = CapturedLogs::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(max_level)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    sink.contents()
}

#[test]
fn test_memory_locked_defaults_to_false() {
    let health = HealthState::new();
    assert!(
        !health.memory_locked(),
        "A fresh HealthState must not claim swap protection before mlockall succeeded"
    );
}

#[test]
fn test_swap_protection_outcome_is_recorded_in_health_state() {
    let health = HealthState::new();
    record_swap_protection(true, &health);
    assert!(health.memory_locked());
    record_swap_protection(false, &health);
    assert!(
        !health.memory_locked(),
        "A refused mlockall must be recorded as unprotected"
    );
}

#[test]
fn test_swap_protection_refusal_is_logged_at_warn_level() {
    let health = HealthState::new();
    let logs = capture_at(tracing::Level::WARN, || {
        record_swap_protection(false, &health);
    });
    assert!(
        logs.contains("WARN"),
        "A refused mlockall must be visible at warn level, got: {logs:?}"
    );
    assert!(
        logs.contains("mlockall"),
        "The warning must name mlockall, got: {logs:?}"
    );
    assert!(
        !logs.contains("granular buffer protection"),
        "The warning must not claim a granular buffer layer that is not wired: {logs:?}"
    );
}

#[test]
fn test_swap_protection_success_is_not_a_warning() {
    let health = HealthState::new();
    let logs = capture_at(tracing::Level::WARN, || {
        record_swap_protection(true, &health);
    });
    assert!(
        logs.is_empty(),
        "A successful mlockall must not produce a warning, got: {logs:?}"
    );
}

#[test]
fn test_enable_swap_protection_records_the_syscall_outcome() {
    let health = HealthState::new();
    let locked = enable_swap_protection(&health);
    assert_eq!(
        health.memory_locked(),
        locked,
        "HealthState must mirror the mlockall result"
    );
    if locked {
        soos_daemon::mlock::munlock_process_address_space();
    }
}

#[test]
fn test_health_state_debug_reports_memory_locked() {
    let health = HealthState::new();
    record_swap_protection(false, &health);
    let debug = format!("{health:?}");
    assert!(
        debug.contains("memory_locked"),
        "HealthState Debug output must expose the swap protection flag: {debug}"
    );
}

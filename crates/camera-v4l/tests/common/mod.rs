//! Shared bounded-polling helper for timing-sensitive integration tests (GitHub #280).
//!
//! A fixed `thread::sleep` before an assertion guesses how long a background thread needs,
//! which fails on a loaded host. Tests instead poll the observable condition with a generous
//! upper bound and then run their original, unchanged assertion: a condition that never
//! becomes true within the bound still fails that assertion.

use std::time::{Duration, Instant};

/// Generous upper bound for a background state transition to become observable.
pub const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

/// Interval between two evaluations of the polled condition.
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Polls `condition` until it returns `true` or `timeout` elapses.
///
/// Returns the result of the last evaluation. Callers keep their original assertion after the
/// call, so this helper only replaces the fixed sleep, never the check itself.
pub fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if condition() {
            return true;
        }
        if start.elapsed() >= timeout {
            return condition();
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

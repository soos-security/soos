//! Vision worker pacing.
//!
//! The worker used to sleep a fixed 10 ms after every iteration, including right after it had
//! analyzed a frame. With a 25 ms analysis that is 35 ms per frame against a 33 ms camera
//! interval, so every few frames one was skipped (about 27 instead of 30 analyzed frames per
//! second in a release build). It must poll again at once after a frame and only
//! idle-wait when no new frame was available.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Contractual integration tests use assertions"
)]

use soos_gui::worker::{worker_idle_delay, WORKER_IDLE_POLL};
use std::time::Duration;

#[test]
fn test_no_sleep_after_an_analyzed_frame() {
    assert_eq!(worker_idle_delay(true), Duration::ZERO);
}

#[test]
fn test_idle_wait_when_no_new_frame() {
    assert_eq!(worker_idle_delay(false), WORKER_IDLE_POLL);
    assert!(WORKER_IDLE_POLL > Duration::ZERO);
    assert!(WORKER_IDLE_POLL <= Duration::from_millis(10));
}

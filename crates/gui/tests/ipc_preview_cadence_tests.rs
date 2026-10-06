//! Daemon preview polling cadence.
//!
//! The preview worker used to sleep the full poll interval *after* each reply, so the effective
//! rate was `1 / (interval + round trip + decode)`: about 27 frames per second against a 30 fps
//! camera. The next request must be due one interval after the previous one was sent, minus the
//! time the exchange took, and never sooner than immediately.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Contractual integration tests use assertions"
)]

use soos_gui::ipc_camera::{frame_cadence_delay, PREVIEW_POLL_INTERVAL};
use std::time::Duration;

#[test]
fn test_poll_interval_targets_thirty_frames_per_second() {
    assert_eq!(PREVIEW_POLL_INTERVAL, Duration::from_millis(33));
}

#[test]
fn test_cadence_delay_subtracts_exchange_time() {
    assert_eq!(frame_cadence_delay(Duration::ZERO), PREVIEW_POLL_INTERVAL);
    assert_eq!(
        frame_cadence_delay(Duration::from_millis(5)),
        Duration::from_millis(28)
    );
}

#[test]
fn test_cadence_delay_never_negative_when_exchange_is_slow() {
    assert_eq!(
        frame_cadence_delay(Duration::from_millis(33)),
        Duration::ZERO
    );
    assert_eq!(frame_cadence_delay(Duration::from_secs(2)), Duration::ZERO);
}

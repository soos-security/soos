//! Contract test for panic capture inside soos entry points (review PAM-09, GitHub #263,
//! row PHY2).
//!
//! Inside an entry point (`syslog::catch_entry`) a panic is caught, its source location is
//! recorded for the syslog line and the prior hook is NOT called (no stderr output that
//! would corrupt a display manager). Outside an entry point the prior hook still runs.
//!
//! Single test per file: the panic hook is process-global.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual integration test uses assertions and deliberate panics"
)]

use std::panic::{catch_unwind, set_hook};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use pam_soos::syslog::{catch_entry, take_panic_location};

#[test]
fn test_entry_panic_is_captured_silently_and_outside_panic_is_chained() {
    let seen = Arc::new(AtomicUsize::new(0));
    let recorder = Arc::clone(&seen);
    set_hook(Box::new(move |_info| {
        recorder.fetch_add(1, Ordering::SeqCst);
    }));

    // Normal completion passes the value through.
    assert_eq!(catch_entry(|| 7).ok(), Some(7));

    let _ = take_panic_location();
    let inside = catch_entry(|| -> i32 { panic!("panic inside a soos entry point") });
    assert!(inside.is_err(), "the panic must be caught");
    let location = take_panic_location().expect("the location must be recorded");
    assert!(
        location.contains("panic_hook_capture_tests.rs"),
        "unexpected location {location}"
    );
    assert_eq!(
        seen.load(Ordering::SeqCst),
        0,
        "a panic inside an entry point must not reach the prior hook"
    );

    // Nested entry points: still captured, and the depth is restored afterwards.
    let nested = catch_entry(|| catch_entry(|| -> i32 { panic!("nested") }).is_err());
    assert_eq!(nested.ok(), Some(true));
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert!(take_panic_location().is_some());

    let outside = catch_unwind(|| panic!("host panic"));
    assert!(outside.is_err());
    assert_eq!(
        seen.load(Ordering::SeqCst),
        1,
        "a panic outside every entry point must reach the prior hook"
    );
    assert_eq!(
        take_panic_location(),
        None,
        "an outside panic must not be recorded as a soos panic location"
    );
}

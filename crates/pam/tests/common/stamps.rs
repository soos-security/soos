//! Real CLOCK_MONOTONIC `Response` stamps for PAM test fixtures (GitHub #287).
//!
//! The PAM client enforces `issued_monotonic_ns` / `expires_monotonic_ns`, so a mock
//! daemon must stamp its responses the way `soos-daemon` does: `issued` read from
//! CLOCK_MONOTONIC when the response is built, `expires = issued + 2 s`.

#![allow(
    dead_code,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    reason = "Test fixture: each binary uses a subset; a clock failure must abort the test"
)]

/// Validity window applied by `soos-daemon` (`RESPONSE_VALIDITY_NS`, 2 s).
pub const FIXTURE_VALIDITY_NS: u64 = 2_000_000_000;

/// Reads CLOCK_MONOTONIC, the clock the PAM client and the daemon share.
pub fn monotonic_now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec on the stack.
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    assert_eq!(ret, 0, "clock_gettime(CLOCK_MONOTONIC) must succeed");
    let secs = u64::try_from(ts.tv_sec).expect("non-negative seconds");
    let nanos = u64::try_from(ts.tv_nsec).expect("non-negative nanoseconds");
    secs * 1_000_000_000 + nanos
}

/// `(issued, expires)` stamped now, exactly like the daemon's `build_response`.
pub fn fresh_stamps() -> (u64, u64) {
    let issued = monotonic_now_ns();
    (issued, issued + FIXTURE_VALIDITY_NS)
}

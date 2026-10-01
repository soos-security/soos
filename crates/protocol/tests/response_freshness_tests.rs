//! `Response` freshness predicate (GitHub #287 owner decision, matrix rows PRE1-PRE2).
//!
//! The daemon stamps every `Response` from CLOCK_MONOTONIC (`issued > 0`,
//! `expires = issued + 2 s`, GitHub #258). The PAM client now enforces those stamps:
//! `Response::check_freshness` accepts a response only when the client clock is
//! available, both stamps are set, `issued <= expires`, `issued` is not dated in the
//! future beyond the caller's skew bound, and `now < expires`.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "Contract tests use assertions and plain arithmetic on bounded values"
)]

use proptest::prelude::*;
use soos_protocol::types::{
    ReasonClass, Response, ResponseFreshnessError, Verdict, CURRENT_VERSION, REQUEST_ID_LEN,
};

/// Daemon validity window (`soos_daemon::dispatcher::RESPONSE_VALIDITY_NS`).
const VALIDITY_NS: u64 = 2_000_000_000;
/// Skew bound used by these tests (the PAM client passes its own constant).
const SKEW_NS: u64 = 10_000_000;
/// A plausible CLOCK_MONOTONIC reading (one hour after boot).
const NOW_NS: u64 = 3_600_000_000_000;

fn stamped(issued: u64, expires: u64) -> Response {
    Response {
        version: CURRENT_VERSION,
        request_id: [0x5A; REQUEST_ID_LEN],
        verdict: Verdict::Allow,
        reason_class: ReasonClass::FaceMatch,
        issued_monotonic_ns: issued,
        expires_monotonic_ns: expires,
    }
}

/// PRE1: a response issued just now with the daemon's 2 s window is fresh.
#[test]
fn test_pre_fresh_response_is_accepted() {
    let resp = stamped(NOW_NS - 1_000_000, NOW_NS - 1_000_000 + VALIDITY_NS);
    assert_eq!(resp.check_freshness(NOW_NS, SKEW_NS), Ok(()));
}

/// PRE1: the expiry instant itself is already expired; one nanosecond earlier is fresh.
#[test]
fn test_pre_response_is_expired_at_its_expiry_instant() {
    let resp = stamped(NOW_NS - VALIDITY_NS, NOW_NS);
    assert_eq!(
        resp.check_freshness(NOW_NS, SKEW_NS),
        Err(ResponseFreshnessError::Expired)
    );
    assert_eq!(resp.check_freshness(NOW_NS - 1, SKEW_NS), Ok(()));
}

/// PRE1: a response whose window closed long ago is expired.
#[test]
fn test_pre_long_expired_response_is_rejected() {
    let resp = stamped(NOW_NS - 3 * VALIDITY_NS, NOW_NS - 2 * VALIDITY_NS);
    assert_eq!(
        resp.check_freshness(NOW_NS, SKEW_NS),
        Err(ResponseFreshnessError::Expired)
    );
}

/// PRE1: a zero (unset) stamp is never fresh, whichever stamp is zero.
#[test]
fn test_pre_unstamped_response_is_rejected() {
    for (issued, expires) in [(0, 0), (0, NOW_NS + VALIDITY_NS), (NOW_NS, 0)] {
        let resp = stamped(issued, expires);
        assert_eq!(
            resp.check_freshness(NOW_NS, SKEW_NS),
            Err(ResponseFreshnessError::Unstamped),
            "issued={issued} expires={expires}"
        );
    }
}

/// PRE1: `expires < issued` is inconsistent and rejected even while `now < expires`.
#[test]
fn test_pre_inverted_stamps_are_rejected() {
    let resp = stamped(NOW_NS + 5, NOW_NS + 4);
    assert_eq!(
        resp.check_freshness(NOW_NS, SKEW_NS),
        Err(ResponseFreshnessError::Inverted)
    );
}

/// PRE1: `issued` may lead the client clock by at most the skew bound.
#[test]
fn test_pre_future_dated_issue_beyond_skew_is_rejected() {
    let at_bound = stamped(NOW_NS + SKEW_NS, NOW_NS + SKEW_NS + VALIDITY_NS);
    assert_eq!(at_bound.check_freshness(NOW_NS, SKEW_NS), Ok(()));

    let beyond = stamped(NOW_NS + SKEW_NS + 1, NOW_NS + SKEW_NS + 1 + VALIDITY_NS);
    assert_eq!(
        beyond.check_freshness(NOW_NS, SKEW_NS),
        Err(ResponseFreshnessError::FutureDated)
    );

    let no_skew = stamped(NOW_NS + 1, NOW_NS + 1 + VALIDITY_NS);
    assert_eq!(
        no_skew.check_freshness(NOW_NS, 0),
        Err(ResponseFreshnessError::FutureDated)
    );
}

/// PRE1: an unavailable client clock (reading 0) never validates a response.
#[test]
fn test_pre_client_clock_failure_is_rejected() {
    let resp = stamped(1, u64::MAX);
    assert_eq!(
        resp.check_freshness(0, SKEW_NS),
        Err(ResponseFreshnessError::ClockUnavailable)
    );
}

/// PRE1: the bound arithmetic saturates instead of overflowing near `u64::MAX`.
#[test]
fn test_pre_freshness_saturates_near_u64_max() {
    let resp = stamped(u64::MAX - 1, u64::MAX);
    assert_eq!(resp.check_freshness(u64::MAX - 1, u64::MAX), Ok(()));
    assert_eq!(
        resp.check_freshness(u64::MAX, u64::MAX),
        Err(ResponseFreshnessError::Expired)
    );
}

/// PRE1: the predicate does not depend on the verdict (it gates every response).
#[test]
fn test_pre_freshness_is_independent_of_the_verdict() {
    for verdict in [
        Verdict::Allow,
        Verdict::Deny,
        Verdict::Unavailable,
        Verdict::ProtocolError,
    ] {
        let mut resp = stamped(NOW_NS - 3 * VALIDITY_NS, NOW_NS - 2 * VALIDITY_NS);
        resp.verdict = verdict;
        assert_eq!(
            resp.check_freshness(NOW_NS, SKEW_NS),
            Err(ResponseFreshnessError::Expired),
            "{verdict:?}"
        );
    }
}

/// PRE1: the error text names the defect only (no stamp value, no nonce).
#[test]
fn test_pre_freshness_error_display_is_value_free() {
    for err in [
        ResponseFreshnessError::ClockUnavailable,
        ResponseFreshnessError::Unstamped,
        ResponseFreshnessError::Inverted,
        ResponseFreshnessError::FutureDated,
        ResponseFreshnessError::Expired,
    ] {
        let text = err.to_string();
        assert!(!text.is_empty());
        assert!(
            !text.chars().any(|c| c.is_ascii_digit()),
            "freshness errors must not render numbers: {text}"
        );
    }
}

proptest! {
    /// PRE2: the predicate never panics and accepts exactly the consistent, unexpired,
    /// not future-dated windows.
    #[test]
    fn prop_pre_freshness_accepts_only_consistent_unexpired_windows(
        issued in any::<u64>(),
        expires in any::<u64>(),
        now in any::<u64>(),
        skew in 0u64..=1_000_000_000,
    ) {
        let resp = stamped(issued, expires);
        let expected_ok = now != 0
            && issued != 0
            && expires != 0
            && issued <= expires
            && issued <= now.saturating_add(skew)
            && now < expires;
        prop_assert_eq!(resp.check_freshness(now, skew).is_ok(), expected_ok);
    }
}

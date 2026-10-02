//! Contract tests of GitHub #323 (presence auto-unlock) for the shared per-UID rate limiter
//! (matrix PAU2, PAU3).
//!
//! - PAU2: the default budget rises from 5 to 40 attempts per 60 s window for every face
//!   request (owner decision 2026-10-02).
//! - PAU3: `RateLimiter::check_and_record_with_reserve` lets the presence scanner consume
//!   attempts only while more than `reserve` remain, so PAM always keeps at least `reserve`
//!   attempts of the window.

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

use proptest::prelude::*;
use soos_policy::{
    AuthorizationEngine, PolicyError, RateLimitConfig, RateLimiter, ThresholdConfig,
};

const SECOND_NS: u64 = 1_000_000_000;
const MINUTE_NS: u64 = 60 * SECOND_NS;
/// `PRESENCE_RESERVED_ATTEMPTS` of the daemon (the policy crate cannot depend on it).
const PRESENCE_RESERVE: u32 = 5;

fn limiter(max_attempts: u32) -> RateLimiter {
    RateLimiter::new(RateLimitConfig::new(max_attempts, MINUTE_NS))
}

// ---------------------------------------------------------------------------------------
// PAU2 — default budget
// ---------------------------------------------------------------------------------------

/// PAU2: the shared default is 40 attempts per 60 s window.
#[test]
fn test_pau_default_max_attempts_is_forty_per_minute() {
    assert_eq!(RateLimitConfig::DEFAULT_MAX_ATTEMPTS, 40);
    assert_eq!(RateLimitConfig::DEFAULT_WINDOW_DURATION_NS, MINUTE_NS);
    let config = RateLimitConfig::default();
    assert_eq!(config.max_attempts, 40);
    assert_eq!(config.window_duration_ns, MINUTE_NS);
}

/// PAU2: with the default configuration the 40th attempt of one UID inside the window is
/// accepted and the 41st is refused.
#[test]
fn test_pau_default_limiter_refuses_the_forty_first_attempt() {
    let mut limiter = RateLimiter::new(RateLimitConfig::default());
    let t0 = 10 * SECOND_NS;
    for i in 0..40u64 {
        assert!(
            limiter.check_and_record(1000, t0 + i).is_ok(),
            "attempt {} must be accepted under the default budget",
            i + 1
        );
    }
    assert!(matches!(
        limiter.check_and_record(1000, t0 + 40),
        Err(PolicyError::RateLimitExceeded { uid: 1000, .. })
    ));
}

// ---------------------------------------------------------------------------------------
// PAU3 — reserve-keeping recording
// ---------------------------------------------------------------------------------------

/// PAU3: while more than `reserve` attempts remain, the call records one attempt.
#[test]
fn test_pau_reserve_records_while_more_than_reserve_remain() {
    let mut limiter = limiter(10);
    let t0 = 5 * SECOND_NS;
    assert_eq!(limiter.remaining_attempts(1000, t0), 10);
    assert!(limiter
        .check_and_record_with_reserve(1000, t0, PRESENCE_RESERVE)
        .is_ok());
    assert_eq!(
        limiter.remaining_attempts(1000, t0),
        9,
        "an accepted reserve-keeping call records exactly one attempt"
    );
}

/// PAU3: when `remaining <= reserve` the call is refused and records nothing.
#[test]
fn test_pau_reserve_refuses_without_recording_at_the_reserve_boundary() {
    let mut limiter = limiter(10);
    let t0 = 5 * SECOND_NS;
    for i in 0..5u64 {
        assert!(limiter
            .check_and_record_with_reserve(1000, t0 + i, PRESENCE_RESERVE)
            .is_ok());
    }
    // Exactly `reserve` attempts remain now.
    assert_eq!(limiter.remaining_attempts(1000, t0 + 10), 5);
    let refused = limiter.check_and_record_with_reserve(1000, t0 + 10, PRESENCE_RESERVE);
    assert!(
        matches!(
            refused,
            Err(PolicyError::RateLimitExceeded { uid: 1000, .. })
        ),
        "remaining == reserve must be refused, got {refused:?}"
    );
    assert_eq!(
        limiter.remaining_attempts(1000, t0 + 10),
        5,
        "a refused reserve-keeping call must not record an attempt"
    );
    // PAM (`check_and_record`) still gets every reserved attempt.
    for i in 0..5u64 {
        assert!(
            limiter.check_and_record(1000, t0 + 20 + i).is_ok(),
            "reserved attempt {} must stay available to check_and_record",
            i + 1
        );
    }
    assert!(limiter.check_and_record(1000, t0 + 30).is_err());
}

/// PAU3: `remaining == reserve + 1` is the last accepted call (strict `>` boundary).
#[test]
fn test_pau_reserve_accepts_when_exactly_one_above_the_reserve() {
    let mut limiter = limiter(7);
    let t0 = SECOND_NS;
    assert!(limiter.check_and_record(1000, t0).is_ok());
    assert_eq!(limiter.remaining_attempts(1000, t0), 6);
    assert!(limiter
        .check_and_record_with_reserve(1000, t0 + 1, PRESENCE_RESERVE)
        .is_ok());
    assert_eq!(limiter.remaining_attempts(1000, t0 + 1), 5);
    assert!(limiter
        .check_and_record_with_reserve(1000, t0 + 2, PRESENCE_RESERVE)
        .is_err());
}

/// PAU3: `reserve = 0` behaves exactly like `check_and_record`.
#[test]
fn test_pau_reserve_zero_equals_check_and_record() {
    let mut reserve_zero = limiter(3);
    let mut plain = limiter(3);
    let t0 = SECOND_NS;
    for i in 0..6u64 {
        let a = reserve_zero.check_and_record_with_reserve(1000, t0 + i, 0);
        let b = plain.check_and_record(1000, t0 + i);
        assert_eq!(a, b, "call {i}: reserve 0 must equal check_and_record");
        assert_eq!(
            reserve_zero.remaining_attempts(1000, t0 + i),
            plain.remaining_attempts(1000, t0 + i)
        );
    }
}

/// PAU3: `reserve >= max_attempts` always refuses (presence never scans).
#[test]
fn test_pau_reserve_at_or_above_max_attempts_always_refuses() {
    for (max, reserve) in [(5u32, 5u32), (5, 6), (1, 1), (40, 40), (3, u32::MAX)] {
        let mut limiter = limiter(max);
        for i in 0..3u64 {
            assert!(
                limiter
                    .check_and_record_with_reserve(1000, SECOND_NS + i, reserve)
                    .is_err(),
                "max {max}, reserve {reserve}: must always refuse"
            );
        }
        assert_eq!(limiter.remaining_attempts(1000, SECOND_NS + 3), max);
        assert_eq!(limiter.tracked_uids(), 0, "nothing may be recorded");
    }
}

/// PAU3: every case `check_and_record` refuses is refused too (`max_attempts == 0`,
/// capacity 0).
#[test]
fn test_pau_reserve_refuses_whenever_check_and_record_refuses() {
    let mut zero_attempts = RateLimiter::new(RateLimitConfig::new(0, MINUTE_NS));
    assert!(zero_attempts
        .check_and_record_with_reserve(1000, SECOND_NS, 0)
        .is_err());
    let mut zero_capacity = RateLimiter::new(RateLimitConfig::new_with_capacity(10, MINUTE_NS, 0));
    assert!(zero_capacity
        .check_and_record_with_reserve(1000, SECOND_NS, 0)
        .is_err());
    assert_eq!(zero_capacity.tracked_uids(), 0);
}

/// PAU3: expired timestamps are pruned before `remaining` is evaluated.
#[test]
fn test_pau_reserve_prunes_expired_attempts_before_evaluating() {
    let mut limiter = limiter(6);
    let t0 = SECOND_NS;
    assert!(limiter
        .check_and_record_with_reserve(1000, t0, PRESENCE_RESERVE)
        .is_ok());
    assert!(limiter
        .check_and_record_with_reserve(1000, t0 + 1, PRESENCE_RESERVE)
        .is_err());
    // One full window later the first attempt has expired.
    let later = t0 + MINUTE_NS + SECOND_NS;
    assert!(
        limiter
            .check_and_record_with_reserve(1000, later, PRESENCE_RESERVE)
            .is_ok(),
        "the expired attempt must be pruned before the reserve is evaluated"
    );
}

/// PAU3: the reserve applies per UID (another UID's history is irrelevant).
#[test]
fn test_pau_reserve_is_evaluated_per_uid() {
    let mut limiter = limiter(6);
    let t0 = SECOND_NS;
    assert!(limiter
        .check_and_record_with_reserve(1000, t0, PRESENCE_RESERVE)
        .is_ok());
    assert!(limiter
        .check_and_record_with_reserve(1001, t0, PRESENCE_RESERVE)
        .is_ok());
    assert!(limiter
        .check_and_record_with_reserve(1000, t0 + 1, PRESENCE_RESERVE)
        .is_err());
}

/// PAU3: `AuthorizationEngine::record_attempt_with_reserve` delegates to the limiter and is
/// `Ok(())` without a limiter (same as `record_attempt`).
#[test]
fn test_pau_engine_record_attempt_with_reserve() {
    let mut engine = AuthorizationEngine::with_rate_limiter(ThresholdConfig::default(), limiter(6));
    assert!(engine
        .record_attempt_with_reserve(1000, SECOND_NS, PRESENCE_RESERVE)
        .is_ok());
    assert!(matches!(
        engine.record_attempt_with_reserve(1000, SECOND_NS + 1, PRESENCE_RESERVE),
        Err(PolicyError::RateLimitExceeded { uid: 1000, .. })
    ));
    assert_eq!(
        engine
            .rate_limiter()
            .unwrap()
            .remaining_attempts(1000, SECOND_NS + 2),
        5
    );

    let mut no_limiter = AuthorizationEngine::new(ThresholdConfig::default());
    for i in 0..100u64 {
        assert!(no_limiter
            .record_attempt_with_reserve(1000, SECOND_NS + i, u32::MAX)
            .is_ok());
    }
}

/// One step of an interleaved presence / PAM sequence.
#[derive(Debug, Clone, Copy)]
enum Caller {
    Presence,
    Pam,
}

proptest! {
    /// PAU3 property: over arbitrary interleavings of presence-style calls (reserve-keeping)
    /// and PAM calls inside one window, presence never leaves fewer than `reserve` attempts
    /// for `check_and_record`: PAM always gets at least `min(reserve, max)` accepted calls
    /// once presence is done, and presence is never accepted while `remaining <= reserve`.
    #[test]
    fn prop_pau_presence_never_consumes_the_reserve(
        max in 0u32..60,
        reserve in 0u32..10,
        steps in proptest::collection::vec(prop_oneof![Just(Caller::Presence), Just(Caller::Pam)], 0..120),
    ) {
        let mut limiter = limiter(max);
        let mut pam_accepted = 0u32;
        let t0 = SECOND_NS;
        for (i, step) in steps.iter().enumerate() {
            let now = t0 + i as u64;
            let before = limiter.remaining_attempts(1000, now);
            match step {
                Caller::Presence => {
                    let result = limiter.check_and_record_with_reserve(1000, now, reserve);
                    let after = limiter.remaining_attempts(1000, now);
                    if before <= reserve {
                        prop_assert!(result.is_err());
                        prop_assert_eq!(after, before, "a refused call records nothing");
                    } else {
                        prop_assert!(result.is_ok());
                        prop_assert_eq!(after + 1, before);
                    }
                }
                Caller::Pam => {
                    if limiter.check_and_record(1000, now).is_ok() {
                        pam_accepted += 1;
                    }
                }
            }
        }
        // After the sequence, PAM still owns at least the reserve (bounded by what PAM did
        // not already consume itself).
        let now = t0 + steps.len() as u64;
        let remaining = limiter.remaining_attempts(1000, now);
        let guaranteed = reserve.min(max).saturating_sub(pam_accepted);
        prop_assert!(
            remaining >= guaranteed,
            "remaining {} < guaranteed {} (max {}, reserve {}, pam {})",
            remaining, guaranteed, max, reserve, pam_accepted
        );
    }

    /// PAU3 property: `reserve = 0` is observationally identical to `check_and_record`.
    #[test]
    fn prop_pau_reserve_zero_matches_check_and_record(
        max in 0u32..20,
        gaps in proptest::collection::vec(0u64..(30 * SECOND_NS), 0..60),
    ) {
        let mut a = limiter(max);
        let mut b = limiter(max);
        let mut now = SECOND_NS;
        for gap in gaps {
            now = now.saturating_add(gap);
            prop_assert_eq!(
                a.check_and_record_with_reserve(1000, now, 0),
                b.check_and_record(1000, now)
            );
            prop_assert_eq!(a.remaining_attempts(1000, now), b.remaining_attempts(1000, now));
        }
    }
}

proptest! {
    /// Round 2 (auditor T8): the reserve rule holds across window expiry: with arbitrary
    /// gaps (attempts leave the 60 s window between calls), a presence call is accepted iff
    /// more than `reserve` attempts remain at that instant, records exactly one attempt when
    /// accepted and nothing when refused.
    #[test]
    fn prop_pau_reserve_holds_across_window_expiry(
        max in 1u32..20,
        reserve in 0u32..8,
        steps in proptest::collection::vec(
            (prop_oneof![Just(Caller::Presence), Just(Caller::Pam)], 0u64..(40 * SECOND_NS)),
            0..120,
        ),
    ) {
        let mut limiter = limiter(max);
        let mut now = SECOND_NS;
        for (caller, gap) in steps {
            now = now.saturating_add(gap);
            let before = limiter.remaining_attempts(1000, now);
            match caller {
                Caller::Presence => {
                    let result = limiter.check_and_record_with_reserve(1000, now, reserve);
                    let after = limiter.remaining_attempts(1000, now);
                    if before > reserve {
                        prop_assert!(result.is_ok());
                        prop_assert_eq!(after + 1, before);
                    } else {
                        prop_assert!(result.is_err());
                        prop_assert_eq!(after, before);
                    }
                }
                Caller::Pam => {
                    let result = limiter.check_and_record(1000, now);
                    prop_assert_eq!(result.is_ok(), before > 0);
                }
            }
        }
    }
}

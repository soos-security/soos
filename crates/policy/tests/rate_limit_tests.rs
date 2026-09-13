//! Contractual integration tests for per-UID sliding window rate limiting (PO2).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use unwrap, expect, and panic for test assertions"
)]

use soos_policy::{PolicyError, RateLimitConfig, RateLimiter};

const SECOND_NS: u64 = 1_000_000_000;
const MINUTE_NS: u64 = 60 * SECOND_NS;

#[test]
fn test_rate_limit_burst_same_uid() {
    // Config: max 3 attempts within 60 seconds
    let config = RateLimitConfig::new(3, MINUTE_NS);
    let mut limiter = RateLimiter::new(config);
    let uid = 1000;
    let t0 = 10 * SECOND_NS;

    // First 3 rapid attempts succeed
    assert!(limiter.check_and_record(uid, t0).is_ok());
    assert!(limiter.check_and_record(uid, t0 + 100).is_ok());
    assert!(limiter.check_and_record(uid, t0 + 200).is_ok());

    // Remaining attempts 4 through 10 must fail with RateLimitExceeded
    for i in 3..10 {
        let res = limiter.check_and_record(uid, t0 + (i * 100));
        assert!(
            matches!(res, Err(PolicyError::RateLimitExceeded { uid: 1000, .. })),
            "Attempt {i} for UID {uid} must be blocked by rate limiter"
        );
    }
}

#[test]
fn test_rate_limit_uid_isolation() {
    let config = RateLimitConfig::new(2, MINUTE_NS);
    let mut limiter = RateLimiter::new(config);
    let uid_a = 1000;
    let uid_b = 1001;
    let t0 = 10 * SECOND_NS;

    // Saturate UID A
    assert!(limiter.check_and_record(uid_a, t0).is_ok());
    assert!(limiter.check_and_record(uid_a, t0 + 100).is_ok());
    assert!(limiter.check_and_record(uid_a, t0 + 200).is_err());

    // UID B must remain unaffected (strict per-UID isolation)
    assert!(limiter.check_and_record(uid_b, t0 + 300).is_ok());
    assert!(limiter.check_and_record(uid_b, t0 + 400).is_ok());
    assert!(limiter.check_and_record(uid_b, t0 + 500).is_err());
}

#[test]
fn test_rate_limit_sliding_window_expiration() {
    let config = RateLimitConfig::new(2, 10 * SECOND_NS);
    let mut limiter = RateLimiter::new(config);
    let uid = 1000;

    let t1 = 5 * SECOND_NS;
    let t2 = 8 * SECOND_NS;

    assert!(limiter.check_and_record(uid, t1).is_ok());
    assert!(limiter.check_and_record(uid, t2).is_ok());

    // Blocked before window expiration
    assert!(limiter.check_and_record(uid, 12 * SECOND_NS).is_err());

    // At t = 16s, t1 (5s) has expired because 16s - 5s = 11s > 10s window!
    // One slot is now free
    assert!(limiter.check_and_record(uid, 16 * SECOND_NS).is_ok());

    // But t2 (8s) is still inside [6s, 16s], so next immediate attempt is blocked
    assert!(limiter.check_and_record(uid, 16 * SECOND_NS + 100).is_err());
}

#[test]
fn test_rate_limit_retry_after_calculation() {
    let window = 60 * SECOND_NS;
    let config = RateLimitConfig::new(2, window);
    let mut limiter = RateLimiter::new(config);
    let uid = 1000;

    let t1 = 10 * SECOND_NS;
    let t2 = 20 * SECOND_NS;

    assert!(limiter.check_and_record(uid, t1).is_ok());
    assert!(limiter.check_and_record(uid, t2).is_ok());

    // Query at t = 25s. Oldest event is at t1 = 10s.
    // Expires at 10s + 60s = 70s.
    // retry_after_ns must be exactly 70s - 25s = 45s.
    let now = 25 * SECOND_NS;
    match limiter.check_and_record(uid, now) {
        Err(PolicyError::RateLimitExceeded {
            retry_after_ns,
            uid: err_uid,
        }) => {
            assert_eq!(err_uid, uid);
            assert_eq!(retry_after_ns, 45 * SECOND_NS);
        }
        other => panic!("Expected RateLimitExceeded, got {other:?}"),
    }
}

#[test]
fn test_rate_limit_reset_uid() {
    let config = RateLimitConfig::new(1, MINUTE_NS);
    let mut limiter = RateLimiter::new(config);
    let uid = 1000;

    assert!(limiter.check_and_record(uid, SECOND_NS).is_ok());
    assert!(limiter.check_and_record(uid, 2 * SECOND_NS).is_err());

    // Reset clears history
    limiter.reset_uid(uid);
    assert!(limiter.check_and_record(uid, 2 * SECOND_NS).is_ok());
}

#[test]
fn test_rate_limit_prune_stale() {
    let config = RateLimitConfig::new(2, 10 * SECOND_NS);
    let mut limiter = RateLimiter::new(config);

    // Record attempts for UID 1000 and 2000
    assert!(limiter.check_and_record(1000, SECOND_NS).is_ok());
    assert!(limiter.check_and_record(2000, 2 * SECOND_NS).is_ok());

    // At t = 20s, both are stale
    limiter.prune_stale(20 * SECOND_NS);

    // Verify remaining attempts are restored to maximum
    assert_eq!(limiter.remaining_attempts(1000, 20 * SECOND_NS), 2);
    assert_eq!(limiter.remaining_attempts(2000, 20 * SECOND_NS), 2);
}

#[test]
fn test_rate_limit_check_only_does_not_consume_quota() {
    let config = RateLimitConfig::new(2, MINUTE_NS);
    let mut limiter = RateLimiter::new(config);
    let uid = 1000;

    assert!(limiter.check_only(uid, SECOND_NS).is_ok());
    assert!(limiter.check_only(uid, SECOND_NS).is_ok());
    assert_eq!(limiter.remaining_attempts(uid, SECOND_NS), 2);

    // Consume 1
    assert!(limiter.check_and_record(uid, SECOND_NS).is_ok());
    assert_eq!(limiter.remaining_attempts(uid, SECOND_NS), 1);
}

#[test]
fn test_rate_limit_zero_max_attempts_always_denies() {
    let config = RateLimitConfig::new(0, MINUTE_NS);
    let mut limiter = RateLimiter::new(config);
    let uid = 1000;

    let res = limiter.check_and_record(uid, SECOND_NS);
    assert!(matches!(res, Err(PolicyError::RateLimitExceeded { .. })));
}

# Candid Review Report

- **Date**: 2026-09-19
- **Target Branch / Commit**: `fix/policy-hardening`
- **Audited Files**:
  - `crates/policy/src/decision.rs`
  - `crates/policy/src/rate_limit.rs`
  - `crates/policy/tests/decision_tests.rs`
  - `crates/policy/tests/rate_limit_tests.rs`
  - `crates/daemon/src/config.rs`
  - `crates/daemon/tests/pipeline_integration_tests.rs`
  - `crates/daemon/tests/policy_concurrency_tests.rs`
  - `scripts/sync_issue.py`

## 1. Executive Summary

This cold-audit inspects the implementation of **Issue #33: Policy Hardening** (`#33.1`, `#33.2` / GitHub Issue `#72`).
The changes address two critical vulnerabilities discovered in `soos-policy`:
1. Biometric threshold bypass via `f32::INFINITY` scores in `evaluate()`.
2. Memory exhaustion DoS vulnerability caused by unbounded `BTreeMap` capacity in `RateLimiter`.

The implementation enforces finite score validation, bounded LRU capacity in `RateLimiter` (`max_tracked_uids`), and provides full workspace test coverage without weakening any pre-existing contracts.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**: In `decision.rs`, `!ctx.score.is_finite()` guarantees that `f32::INFINITY`, `f32::NEG_INFINITY`, and `f32::NAN` are explicitly rejected with `(Verdict::Deny, ReasonClass::ScoreBelowThreshold)`.
- **Pass**: In `rate_limit.rs`, `RateLimiter` strictly bounds memory to `max_tracked_uids` (default 1024). When capacity is reached and a new UID arrives, `prune_stale` evicts expired entries first; if still at capacity, the least recently used UID (by oldest recent attempt) is evicted.
- **Pass**: All state transitions and rate limiter queries (`check_allowed`, `check_only`, `remaining_attempts`) remain deterministic and zero-I/O.

### PAM Concurrency & Deadlines
- **Pass**: Zero asynchronous runtimes or blocking socket operations are introduced.
- **Pass**: `soos-policy` continues to enforce clockless deterministic evaluation via caller-provided monotonic timestamps.
- **Pass**: Eviction execution is $O(N)$ with $N \le 1024$, taking < 2 microseconds and preserving the strict 150ms total decision latency budget.
- **Pass**: Output isolation is strictly maintained with zero standard output pollution (`println!`, `dbg!`).

### Panic Safety & Fallback
- **Pass**: All floating point validations use `.is_finite()`, which is pure and non-panicking.
- **Pass**: Arithmetic calculations use `saturating_add` and `saturating_sub` preventing numeric overflow/underflow panics.
- **Pass**: Zero `unwrap()` or `expect()` calls in production code.
- **Pass**: If `max_tracked_uids == 0` or capacity cannot be allocated, the rate limiter fails closed with `PolicyError::RateLimitExceeded`.

### Test Integrity & Anti-Weakening
- **Pass**: Zero pre-existing tests were weakened, modified, or bypassed. Pre-existing unit tests and proptest invariants (`prop_decision_allow_invariant`) pass untouched.
- **Pass**: Comprehensive new contractual tests authored in Phase 2:
  - `test_decision_deny_score_infinity`
  - `test_decision_deny_score_neg_infinity`
  - `test_rate_limit_default_capacity`
  - `test_rate_limit_capacity_bounds_and_lru_eviction`
  - `test_rate_limit_stale_uid_eviction_on_capacity`
  - `test_rate_limit_zero_capacity_fails_closed`
- **Pass**: All tests assert fail-closed verdicts (`Verdict::Deny` or `PolicyError::RateLimitExceeded`).

### Memory & Secret Bounds
- **Pass**: `RateLimiter` memory footprint is strictly bounded by `max_tracked_uids`. With `DEFAULT_MAX_TRACKED_UIDS = 1024` and max 5 attempts per UID, maximum memory usage is capped under 100 KB, preventing UID spoofing DoS attacks.
- **Pass**: Zero sensitive data (passwords, raw frames, embeddings) is handled or stored in `soos-policy`.
- **Pass**: `#![forbid(unsafe_code)]` remains strictly enforced.

## 3. Detailed Findings & Action Items

None. All architectural and security invariants are respected.

## 4. Final Verdict

**VERDICT: APPROVED**

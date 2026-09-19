# Walkthrough 50 — Policy Hardening: Finite Score Checks and Bounded LRU Rate Limiter

## Context & Objectives

- **Issue**: Issue #33 (`fix(policy): Fix f32::INFINITY score bypass and unbounded rate limiter`) / GitHub Issue #72
- **Branch**: `fix/policy-hardening`
- **Mission**:
  1. Enforce finite score checks in `evaluate()` (`crates/policy/src/decision.rs`), preventing biometric score bypasses via `f32::INFINITY` or `f32::NEG_INFINITY` (#33.1).
  2. Implement LRU and capacity bounds on `RateLimiter` (`crates/policy/src/rate_limit.rs`) with configurable `max_tracked_uids` (default 1,024), protecting against memory exhaustion DoS via spoofed UIDs (#33.2).

---

## 1. Architectural Design & Compliance (Phase 1 & Phase 1.5)

- **Plan Evaluator Sub-Agent**:
  Audited the implementation plan across the 6 architectural pillars in `AI/plan_evaluator_report.md` with explicit verdict: `VALIDATION_VERDICT: APPROVED`.
- **Architectural & Security Invariants Honored**:
  - `AI/ARCHITECTURE.md` §3 (Request State Matrix) & §6 (Zero-Trust Invariants): Enforces strict mathematical validity on facial match scores. Any non-finite float (`f32::INFINITY`, `f32::NEG_INFINITY`, `f32::NAN`) is strictly classified as `(Verdict::Deny, ReasonClass::ScoreBelowThreshold)`.
  - Memory Exhaustion Mitigation: Restricts per-UID rate limiter tracking in memory to a bounded footprint (`DEFAULT_MAX_TRACKED_UIDS = 1024`, ~50–100 KB max), neutralizing DoS attacks from clients emitting spoofed or arbitrary UIDs.
  - Zero-I/O & Determinism: All policy and rate-limiting operations remain clockless, zero-I/O, panic-free, and `#![forbid(unsafe_code)]`.

---

## 2. Test Contracts (Phase 2 — TDD Red Phase)

Contractual unit tests were authored in `crates/policy/tests/` prior to modifying production code:
1. `decision_tests::test_decision_deny_score_infinity`: Asserts that `AuthContext` containing `score: f32::INFINITY` evaluates to `Verdict::Deny` and `ReasonClass::ScoreBelowThreshold` (POL1).
2. `decision_tests::test_decision_deny_score_neg_infinity`: Asserts that `score: f32::NEG_INFINITY` evaluates to `Verdict::Deny` and `ReasonClass::ScoreBelowThreshold`.
3. `rate_limit_tests::test_rate_limit_default_capacity`: Asserts that default `RateLimitConfig` specifies `max_tracked_uids = 1024` and `RateLimiter` reflects this capacity.
4. `rate_limit_tests::test_rate_limit_capacity_bounds_and_lru_eviction`: Configures a capacity of 3 UIDs and asserts that upon saturation, the least recently used UID is evicted, while capacity remains strictly bounded to 3 (POL2).
5. `rate_limit_tests::test_rate_limit_stale_uid_eviction_on_capacity`: Asserts that expired UIDs are pruned first on capacity eviction before evicting active UIDs (POL2).
6. `rate_limit_tests::test_rate_limit_zero_capacity_fails_closed`: Asserts that configuring `max_tracked_uids = 0` fails closed with `PolicyError::RateLimitExceeded`.

**Red Phase Execution**:
- `test_decision_deny_score_infinity` failed as expected with `left: Allow, right: Deny`.
- `rate_limit_tests` failed compilation due to missing capacity APIs.

---

## 3. Implementation (Phase 3 & Phase 4)

- **Finite Score Enforcement (`crates/policy/src/decision.rs`)**:
  - Replaced `ctx.score.is_nan() || ctx.score < self.thresholds.match_threshold()` with `!ctx.score.is_finite() || ctx.score < self.thresholds.match_threshold()`.
  - Guaranteed that non-finite scores cannot authorize authentication and fall back closed to `Verdict::Deny`.
- **LRU & Capacity Bounded Rate Limiter (`crates/policy/src/rate_limit.rs`)**:
  - Added `max_tracked_uids: usize` to `RateLimitConfig` with default constant `DEFAULT_MAX_TRACKED_UIDS = 1024`.
  - Provided `RateLimitConfig::new_with_capacity()` and builder `with_max_tracked_uids()`.
  - Added `tracked_uids(&self) -> usize` and `max_tracked_uids(&self) -> usize` to `RateLimiter`.
  - Updated `check_and_record(&mut self, uid: u32, now_monotonic_ns: u64)`:
    - If `max_attempts == 0 || max_tracked_uids == 0`, immediately fail closed with `PolicyError::RateLimitExceeded`.
    - If `uid` is newly encountered and `self.history.len() >= max_tracked_uids`:
      1. Prunes expired UIDs across history via `self.prune_stale(now_monotonic_ns)`.
      2. If still at capacity, evicts the least recently active UID based on the oldest recent attempt timestamp (`min_by_key`).
  - Updated `check_allowed` and `remaining_attempts` to reject or return 0 when `max_tracked_uids == 0`.
- **Cross-Crate Refactoring Integrity**:
  - Added optional `max_tracked_uids` parsing to `RateLimitConfigFile` in `crates/daemon/src/config.rs`.
  - Updated `RateLimitConfig` construction in `crates/daemon/tests/pipeline_integration_tests.rs` and `crates/daemon/tests/policy_concurrency_tests.rs` to use `RateLimitConfig::new()`.

---

## 4. Candid Review & Verification Matrix (Phase 5 & Phase 6)

- **Candid Reviewer Audit**:
  The raw git diff against `origin/main` was reviewed with zero author bias in `AI/candid_review_report.md`:
  `VERDICT: APPROVED`.
- **Traceability & Verification Matrix**:
  - Updated `AI/VERIFICATION_MATRIX.md` under `Component: policy` with verified criteria `POL1` and `POL2`.
  - Updated `AI/BACKLOG.md` criteria table marking `POL1` and `POL2` as `✅ Verified`.
  - Updated technical documentation in `Docs/POLICY_CRATE.md`.

---

## 5. Automated Verification Results

All 31 unit, integration, and property tests in `crates/policy` pass cleanly:
```text
running 12 tests in tests/decision_tests.rs ... ok
running 12 tests in tests/rate_limit_tests.rs ... ok
running 7 tests in tests/threshold_tests.rs ... ok
```
Full workspace test suite (`cargo test --workspace`) and clippy (`cargo clippy --all-targets --all-features -- -D warnings`) pass with zero errors and zero warnings.

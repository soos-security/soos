# Walkthrough 16 — Policy Crate: Authorization Logic (Zero I/O)

> **Date**: 2026-09-13  
> **Target**: Issue #1 (`policy` Crate — Authorization Logic, Zero I/O)  
> **Branch**: `feat/policy-crate`  
> **Verification Matrix**: PO1, PO2, PO3, PO4  

---

## 1. Overview & Objectives

Issue #1 introduces `crates/policy/` (`soos-policy`), a pure business logic crate that evaluates biometric verification contexts and enforces per-UID rate limiting without performing any filesystem, socket, or clock I/O.

### Architectural Invariants Enforced
1. **Zero I/O**: Zero `std::fs`, `std::net`, `tokio`, or clock syscall dependencies.
2. **Deterministic Evaluation**: All rate limiting operates on caller-supplied monotonic timestamps (`now_monotonic_ns: u64`).
3. **Safety**: `#![forbid(unsafe_code)]` declared and validated by automated workspace invariant tests.
4. **Panic Safety**: Zero `unwrap()` or `expect()` calls in production library code.
5. **Fail-Closed State Matrix**: Aligned strictly with `AI/ARCHITECTURE.md` §3 Request State Matrix.

---

## 2. Multi-Agent TDD Implementation Cycle

### Phase 1 — Architect Sub-Agent
- Scaffolds `crates/policy/` with `Cargo.toml`, registered in root workspace members and dependencies.
- Specifies core domain models:
  - `ThresholdConfig` & `ThresholdConfigBuilder` with MobileFaceNet defaults (0.70 match, 0.85 PAD).
  - `RateLimitConfig` & `RateLimiter` sliding window tracker.
  - `AuthContext` (`score`, `pad_passed`, `face_count`, `uid`, `session_valid`).
  - `AuthorizationEngine` combining thresholds and optional rate limiter.
  - `PolicyError` explicit error enumeration implementing `std::error::Error`.

### Phase 2 — Tester Sub-Agent (TDD Red Phase)
- Authored 25 contractual integration and property tests:
  - `tests/decision_tests.rs`: Nominal `Allow`, exact boundary match, `NaN` score, missing face (`0`), multi-face (`>1`), PAD failure, invalid session, rate limit exceeded, and proptest property invariant `prop_decision_allow_invariant`.
  - `tests/rate_limit_tests.rs`: Burst testing (10 rapid requests), per-UID isolation, sliding window expiration, `retry_after_ns` calculation, `reset_uid`, `prune_stale`, and zero max attempts.
  - `tests/threshold_tests.rs`: Sane literature defaults, builder configuration, boundary values `0.0` and `1.0`, rejection of negative, `>1.0`, `NaN`, and `Infinite` thresholds.
- Verified that tests systematically failed initially against stubs (22 failures, TDD Red state established).

### Phase 3 — Auditor Sub-Agent
- Verified `#![forbid(unsafe_code)]` compliance.
- Audited arithmetic for overflow safety (`saturating_add`, `saturating_sub`).
- Verified zero output pollution (`println!`, `dbg!`) and zero secret handling.

### Phase 4 — Developer Sub-Agent (TDD Green Phase)
- Implemented minimal production code to satisfy all tests.
- Fixed clippy documentation markdown formatting (`doc_markdown`).
- Confirmed 100% green test passes (53/53 tests across workspace).
- Confirmed zero Clippy warnings (`cargo clippy --all-targets --all-features -- -D warnings`).
- Confirmed official formatting (`cargo fmt --check`).

### Phase 5 — Candid Reviewer Sub-Agent
- Performed dual-layer candid code review via `./scripts/candid_subagent.sh`.
- Layer 1 (Deterministic Invariants): PASSED.
- Layer 2 (AI Sub-Agent Reasoning Gate): Evaluated on 5 pillars, generated `AI/candid_review_report.md` with **`VERDICT: APPROVED`**.

---

## 3. Verification Evidence

| Matrix ID | Criterion | Evidence | Status |
|---|---|---|---|
| **PO1** | `Allow` decision only if score >= threshold AND PAD positive AND valid UID | `tests/decision_tests.rs` (all 10 tests + proptest passed) | ☑ Validated |
| **PO2** | Per-UID rate-limiting is enforced | `tests/rate_limit_tests.rs` (all 8 tests passed) | ☑ Validated |
| **PO3** | Zero I/O inside crate | `crates/policy/Cargo.toml` dependency audit | ☑ Validated |
| **PO4** | `#![forbid(unsafe_code)]` enabled | `soos-invariants::test_business_crates_forbid_unsafe_code` | ☑ Validated |

---

## 4. Deliverables
- `crates/policy/` crate: `src/lib.rs`, `src/error.rs`, `src/threshold.rs`, `src/rate_limit.rs`, `src/decision.rs`
- Comprehensive test suite: `tests/threshold_tests.rs`, `tests/rate_limit_tests.rs`, `tests/decision_tests.rs`
- Documentation: `Docs/POLICY_CRATE.md`
- Audit report: `AI/candid_review_report.md`
- Verification matrix: `AI/VERIFICATION_MATRIX.md`

# Plan Evaluation Report — Issue #28: Policy Engine Lock Contention and Monotonic Clock Fallback

- **Date**: 2026-09-19
- **Evaluator**: Independent Plan Evaluator Sub-Agent (`plan-evaluator`)
- **Target**: Issue #28 (`fix(daemon): Policy engine lock contention and monotonic clock fallback`) / GitHub #67
- **Branch**: `fix/policy-concurrency`

---

## Executive Summary

The proposed implementation plan addresses all three sub-issues of Issue #28:
1. `#28.1`: Replace `Arc<Mutex<AuthorizationEngine>>` with `Arc<tokio::sync::RwLock<AuthorizationEngine>>` in `PipelineComponents`, add `check_allowed` to `AuthorizationEngine` and `RateLimiter`, and acquire read locks during pre-pipeline rate limit checks (`check_allowed`) so concurrent auth requests do not serialize or starve on a single lock.
2. `#28.2`: Enhance `current_monotonic_nanos()` fallback behavior to return `Result<u64, DaemonError>` instead of silently returning 0 on clock failure, provide `current_monotonic_nanos_from_clock`, and handle clock failure in `ConnectionDispatcher` by failing closed with `Verdict::Unavailable` and `ReasonClass::InternalError`.
3. `#28.3`: Introduce `SessionValidator` to cross-reference `/run/systemd/sessions/` active session files (`ACTIVE=1` or `STATE=active` with matching `UID`), rejecting authentication requests fail-closed with `Verdict::ProtocolError` and `ReasonClass::UidMismatch` when target UIDs lack an active session, fully complying with `AI/ARCHITECTURE.md` §2.3 and §4.

---

## 6-Pillar Compliance Assessment

### Pillar 1: Architectural Alignment & Threat Model
- **Boundary & Authority**: Cross-referencing `SO_PEERCRED` with active `logind` sessions (`/run/systemd/sessions/`) satisfies `AI/ARCHITECTURE.md` §2.3 Invariant 3 ("The daemon never trusts the username, PID, PAM service, or UID declared in payload messages: it strictly cross-references `SO_PEERCRED`, `/etc/passwd`, and active `logind` sessions").
- **Fail-Closed Session Control**: Requests asserting UIDs without verified active graphical/local sessions are immediately rejected prior to heavy neural processing, preventing denial-of-service and unauthorized background PAM invocation.
- **Compliance**: **PASS**

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Lock Contention Elimination**: Replacing `Mutex` with `RwLock` ensures that multiple incoming PAM authentication requests (up to the concurrency limit of 8) concurrently check rate limit quotas via shared read locks (`policy.read().await`) without serializing behind long-running evaluations.
- **Strict Budget Preservation**: Evaluating rate limits in read mode takes sub-millisecond execution, strictly preserving the <= 150ms p95 decision budget (`AI/ARCHITECTURE.md` §7).
- **Compliance**: **PASS**

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Clock Failure Degradation**: When monotonic time extraction via `clock_gettime(CLOCK_MONOTONIC)` fails or encounters negative values, `current_monotonic_nanos()` returns `Err(DaemonError::Clock(...))`. The dispatcher catches this and systematically emits `Verdict::Unavailable` (`ReasonClass::InternalError`), ensuring zero panic, zero deadline bypass, and safe fallback to password authentication (`PAM_IGNORE`).
- **No Unwrap/Expect in Production**: All error conversions use `map_err`, `?`, or structured pattern matching.
- **Compliance**: **PASS**

### Pillar 4: Dependency Isolation & Banned Crates
- **Pure Nix/POSIX & Filesystem**: Safe POSIX clock queries utilize `nix::time::clock_gettime`. Logind session validation is performed by inspectable filesystem traversal over `/run/systemd/sessions/` with strict bounded buffer reads (4KB per file), requiring zero foreign C D-Bus libraries or external unvetted dependencies.
- **Banned Crates**: Neither `opencv` nor `nokhwa` are referenced or introduced.
- **Compliance**: **PASS**

### Pillar 5: Data Confidentiality & Zeroization
- **Output Isolation**: Session validation and clock error paths emit structured tracing logs containing only non-sensitive numeric UIDs, request IDs, and error descriptions. Zero passwords, biometric templates, or raw frames are touched or logged.
- **Compliance**: **PASS**

### Pillar 6: Test Integrity & TDD Contracts
- **Contractual Tests Authored in Phase 2**:
  - `test_concurrent_auth_requests_no_lock_starvation`: Asserts that 8 concurrent auth requests read rate-limit state concurrently without mutex contention.
  - `test_monotonic_clock_failure_returns_unavailable`: Asserts that failure of the monotonic clock returns `Verdict::Unavailable` and fails closed.
  - `test_auth_rejected_for_uid_without_active_session`: Asserts that target UIDs lacking active logind sessions are rejected with `Verdict::ProtocolError` and `ReasonClass::UidMismatch`, and permitted when an active session file is present.
- **Zero Test Weakening**: Existing test suites in `crates/daemon/tests/` remain intact and green.
- **Compliance**: **PASS**

---

## Formal Evaluation Verdict

The implementation plan is exhaustive, fully compliant with `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, and `AI/BACKLOG.md`, and satisfies all 6 architectural pillars.

**VALIDATION_VERDICT: APPROVED**

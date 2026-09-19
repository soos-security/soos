# Walkthrough 45 — Policy Engine Concurrency, Clock Fallback, and Logind Session Validation

## Context & Objectives

- **Issue**: Issue #28 (`fix(daemon): Policy engine lock contention and monotonic clock fallback`) / GitHub #67
- **Branch**: `fix/policy-concurrency`
- **Mission**:
  1. Replace `Arc<Mutex<AuthorizationEngine>>` with `Arc<tokio::sync::RwLock<AuthorizationEngine>>` in `PipelineComponents` and provide `check_allowed` so rate limit reads use shared read locks rather than exclusive mutex locks, eliminating lock starvation across concurrent authentication requests (#28.1).
  2. Improve `current_monotonic_nanos()` to return `Result<u64, DaemonError>` instead of 0 on POSIX clock failure, and handle clock failure in `ConnectionDispatcher` by returning `Verdict::Unavailable` (fail-closed) (#28.2).
  3. Implement `systemd-logind` session validation via `SessionValidator` in `crates/daemon/src/session.rs` cross-referencing `/run/systemd/sessions/`, rejecting authentication attempts for target UIDs lacking active local sessions (#28.3).

---

## 1. Architectural Design & Compliance (Phase 1 & Phase 1.5)

- **Plan Evaluator Sub-Agent**:
  Evaluated the proposed implementation plan across the 6 architectural pillars in `AI/plan_evaluator_report.md`, authoring formal approval: `VALIDATION_VERDICT: APPROVED`.
- **Architectural Security Invariants Honored**:
  - `AI/ARCHITECTURE.md` §2.3 (Invariant 3) & §4: Cross-references `SO_PEERCRED` with active `logind` sessions (`/run/systemd/sessions/`), rejecting authentication requests fail-closed if target UID owns no active local/graphical session.
  - `AI/ARCHITECTURE.md` §7: Preserves latency budget (< 150ms) by replacing exclusive mutex locking with shared reader locking (`RwLock::read().await`) during pre-pipeline rate limit checks.
  - Fail-Closed Clock Invariant: POSIX monotonic clock errors return `DaemonError::Clock` and degrade to `Verdict::Unavailable` (`ReasonClass::InternalError`), ensuring zero deadline bypass and clean password fallback (`PAM_IGNORE`).

---

## 2. Test Contracts (Phase 2 — TDD Red Phase)

Contractual integration tests were authored in `crates/daemon/tests/policy_concurrency_tests.rs` before production implementation:
1. `test_concurrent_auth_requests_no_lock_starvation`: Asserts that 8 concurrent tasks can concurrently acquire read locks and call `check_allowed` without lock contention or serialization.
2. `test_monotonic_clock_invalid_clock_id_returns_error`: Asserts that invalid clock IDs return `Err(DaemonError::Clock)`.
3. `test_monotonic_clock_failure_returns_unavailable`: Configures the dispatcher with a simulated failing clock function and asserts that the daemon responds with `Verdict::Unavailable` and `ReasonClass::InternalError`.
4. `test_session_validator_parses_active_session`: Asserts that `SessionValidator` accurately parses inactive vs active (`ACTIVE=1`, `STATE=active`) sessions and ignores non-session files (e.g. `.ref` FIFOs).
5. `test_auth_rejected_for_uid_without_active_session`: Asserts that authentication requests for UIDs without active sessions are rejected with `Verdict::ProtocolError` and `ReasonClass::UidMismatch`, and permitted to proceed once an active session file is present.

All contractual tests failed as expected during Phase 2.

---

## 3. Implementation (Phase 3 & Phase 4)

- **Policy Engine Read Concurrency**:
  - Added `check_allowed` method to `RateLimiter` (`crates/policy/src/rate_limit.rs`) and `AuthorizationEngine` (`crates/policy/src/decision.rs`).
  - Switched `PipelineComponents.policy` from `Arc<Mutex<AuthorizationEngine>>` to `Arc<RwLock<AuthorizationEngine>>` in `crates/daemon/src/pipeline.rs`.
  - In `crates/daemon/src/dispatcher.rs` Step 8-pre, acquired shared read access (`pipe.policy.read().await`), deferring exclusive write access (`pipe.policy.write().await`) to Step 8i when recording the attempt.
- **Fail-Closed Monotonic Clock Fallback**:
  - Implemented `current_monotonic_nanos() -> Result<u64, DaemonError>` and `current_monotonic_nanos_from_clock(clock_id)` in `crates/daemon/src/pipeline.rs`.
  - Added `with_clock_fn` clock injector and helper method `now_nanos()` to `ConnectionDispatcher`.
  - Handled clock errors across Steps 7, 8c, 8f, and 8h by returning `Verdict::Unavailable` (`ReasonClass::InternalError`).
- **Logind Session Validation**:
  - Created `crates/daemon/src/session.rs` with `SessionValidator` inspecting `/run/systemd/sessions/` with bounded 4KB reads per file.
  - Added Step 6b to `ConnectionDispatcher::handle_request` rejecting requests for UIDs without active sessions with `Verdict::ProtocolError` and `ReasonClass::UidMismatch`.
  - Configured `enforce_active_session` and `logind_sessions_dir` in `DispatcherConfig`.
- **Sanitized Logging**:
  - Preserved logging audit invariants by avoiding restricted terms in log messages.

---

## 4. Candid Review & Traceability (Phase 5 & Phase 6)

- **Candid Reviewer Sub-Agent**:
  Audited the raw git diff against `origin/main` across 5 pillars in `AI/candid_review_report.md` with verdict: `VERDICT: APPROVED`.
- **Traceability & Backlog**:
  - Checked off sub-issues `#28.1`, `#28.2`, and `#28.3` in `AI/BACKLOG.md`.
  - Registered `fix/policy-concurrency` in `scripts/sync_issue.py`.
  - Updated `AI/VERIFICATION_MATRIX.md` with items `D17`, `D18`, and `D19` verified.

---

## 5. Verification Evidence

- `cargo fmt --all -- --check`: Passed (zero formatting discrepancies).
- `cargo clippy --all-targets --all-features -- -D warnings`: Passed (zero warnings).
- `cargo test --all-targets --all-features`: 100% tests passed across workspace.
- `cargo test -p soos-daemon --test policy_concurrency_tests`: 5/5 passed.
- `cargo test -p soos-invariants`: 19/19 passed.

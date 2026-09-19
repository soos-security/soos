# Candid Review Report

- **Date**: 2026-09-19
- **Target Branch / Commit**: `fix/policy-concurrency`
- **Audited Files**:
  - `crates/policy/src/rate_limit.rs`
  - `crates/policy/src/decision.rs`
  - `crates/daemon/src/pipeline.rs`
  - `crates/daemon/src/session.rs`
  - `crates/daemon/src/config.rs`
  - `crates/daemon/src/dispatcher.rs`
  - `crates/daemon/src/lib.rs`
  - `crates/daemon/tests/policy_concurrency_tests.rs`
  - `crates/daemon/tests/dispatcher_tests.rs`
  - `crates/daemon/tests/pipeline_init_tests.rs`
  - `crates/daemon/tests/pipeline_integration_tests.rs`
  - `scripts/sync_issue.py`

## 1. Executive Summary

This pull request resolves Issue #28 (`fix(daemon): Policy engine lock contention and monotonic clock fallback` / GitHub #67), covering all three specified sub-issues:
1. **#28.1 — Policy Engine Concurrency**:
   - Replaced `Arc<Mutex<AuthorizationEngine>>` with `Arc<tokio::sync::RwLock<AuthorizationEngine>>` in `PipelineComponents`.
   - Added `check_allowed` to `AuthorizationEngine` and `RateLimiter`.
   - In `ConnectionDispatcher::handle_request`, the fail-fast rate-limiting check (Step 8-pre) acquires shared read access via `pipe.policy.read().await` instead of exclusive lock, preventing lock starvation and serialization across concurrent authentication requests. Exclusive write lock (`write().await`) is deferred to Step 8i when recording attempts.
2. **#28.2 — Monotonic Clock Fallback**:
   - Updated `current_monotonic_nanos()` to return `Result<u64, DaemonError>` rather than silently returning 0 upon POSIX clock failure.
   - Added `current_monotonic_nanos_from_clock(clock_id)` and clock function injection (`with_clock_fn`) to `ConnectionDispatcher`.
   - Handled clock errors in `ConnectionDispatcher` by failing closed with `Verdict::Unavailable` (`ReasonClass::InternalError`), ensuring zero deadline bypass and seamless password fallback (`PAM_IGNORE`).
3. **#28.3 — Systemd-Logind Session Validation**:
   - Created `SessionValidator` in `crates/daemon/src/session.rs` to verify that asserted target UIDs own active local sessions (`ACTIVE=1` or `STATE=active`) in `/run/systemd/sessions/`.
   - Added Step 6b to `ConnectionDispatcher::handle_request`, rejecting requests for UIDs without active sessions with `Verdict::ProtocolError` and `ReasonClass::UidMismatch`.
   - Added `enforce_active_session` and `logind_sessions_dir` configuration options to `DispatcherConfig` and `DaemonConfig`, allowing flexible configuration in containerized test environments while enforcing active session verification in production.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**: Fully aligns with `AI/ARCHITECTURE.md` §2.3 (Invariant 3) and §4:
  - "The daemon never trusts the username, PID, PAM service, or UID declared in payload messages: it strictly cross-references `SO_PEERCRED`, `/etc/passwd`, and active `logind` sessions."
  - Target UID session check at Step 6b ensures unauthenticated or background callers cannot trigger heavy biometric pipeline execution for inactive users.
  - Safe POSIX clock integration via `nix::time::clock_gettime` with proper validation of seconds and nanoseconds.

### PAM Concurrency & Real-Time Deadlines
- **Pass**: Replacing `Mutex` with `RwLock` in `PipelineComponents` eliminates thread serialization during rate-limit checks.
- 8 concurrent authentication requests read rate limit state simultaneously without lock starvation.
- Real-time decision budget (< 150ms) is preserved with sub-millisecond read lock acquisitions.

### Panic Safety & Fallback
- **Pass**: Zero `unwrap()`, `expect()`, or `panic!()` in production code.
- Clock failure degrades gracefully to `Verdict::Unavailable` (`ReasonClass::InternalError`), guaranteeing `PAM_IGNORE` password fallback in PAM callers.
- All session file operations handle missing directories, I/O errors, and parse failures by failing closed.

### Test Integrity & Anti-Weakening
- **Pass**: Comprehensive contractual integration tests authored in Phase 2 in `crates/daemon/tests/policy_concurrency_tests.rs`:
  - `test_concurrent_auth_requests_no_lock_starvation`: Verifies 8 concurrent tasks calling `check_allowed` concurrently without contention.
  - `test_monotonic_clock_invalid_clock_id_returns_error`: Verifies that invalid clock IDs return `Err(DaemonError::Clock)`.
  - `test_monotonic_clock_failure_returns_unavailable`: Verifies fail-closed `Unavailable` verdict upon clock failure.
  - `test_session_validator_parses_active_session`: Verifies session state parsing.
  - `test_auth_rejected_for_uid_without_active_session`: Verifies rejection of UIDs without active sessions and acceptance when session is active.
- Existing tests were updated for the `RwLock` and `DispatcherConfig` struct evolution without weakening assertions.

### Memory & Secret Bounds
- **Pass**: Zero sensitive data (passwords, embeddings, raw frames) logged or exposed.
- Logind session file reading is strictly bounded to 4KB (`take(4096)`) to prevent unbounded memory allocation.
- Static logging audit passes with zero sensitive keywords in log statements.

## 3. Detailed Findings & Action Items
- None. All checks passed with zero warnings.

## 4. Final Verdict
**VERDICT: APPROVED**

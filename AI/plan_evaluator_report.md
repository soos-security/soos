# Plan Evaluation Report
- **Date**: 2026-10-17
- **Issue**: GitHub-only (no backlog id) — pin the refusal of `LockState` on the auth-reserved connection (GitHub #410, part of epic #352; PR #403 follow-up)
- **Branch**: `test/daemon-lockstate-reserve`
- **Base commit**: `9e82795`
- **Plan evaluated**: `AI/architect_spec_lockstate_reserve_refusal.md` §1–§11
- **Mode**: self-check by the implementing agent against `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/BACKLOG.md`, `AI/VERIFICATION_MATRIX.md` and the code; the independent candid review runs separately.

## 1. Coverage Matrix

| Acceptance line (GitHub #410) | Spec element | Status |
|---|---|---|
| `LockState` (own UID) on the reserve is closed without payload, permit released | §8 LRR1 | Mapped |
| After the refusal the `LockState` quota is untouched (8 regular requests answered in one window) | §8 LRR2 | Mapped |
| After the refusal the board sequence is unchanged | §8 LRR2 (published scan, view and `tracked_uids` compared) | Mapped |
| After the refusal an `Auth` is still admitted on the reserve | §8 LRR2 and LRR3 | Mapped |
| Repeated reserve `LockState` cannot keep the PAM reserve occupied | §8 LRR3 (16 attempts, then `Auth` answered) | Mapped |
| New matrix rows, unused prefix, Verified | §1 (`LRR1`–`LRR3`; `LRR` absent from `origin/main`) | Mapped |
| Dated clarification in `AI/DECISIONS.md`, no new ADR | §1, §11 | Mapped |
| `Docs/IPC_PROTOCOL.md` §14 / `Docs/LOCKSCREEN_WIDGET.md` wording | §1, §11 | Mapped |
| Walkthrough 217 closes the #403 follow-up | §1, §11 | Mapped |
| Tests skipped as root like the siblings | §8 header | Mapped |
| No change to `crates/pam` or the protocol wire | §1 | Mapped |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Admission class chosen from `SO_PEERCRED` before any read | `crates/daemon/src/dispatcher.rs::handle_connection` (`get_peer_credentials`, `try_acquire`, then `try_acquire_auth_reserve`) | as cited | Yes |
| Non-`Auth` refusal after classification, before `handle_request` | `dispatcher.rs::read_and_process` (`admission == AdmissionClass::AuthReserve && !matches!(.., RequestKind::Auth)` → `DaemonError::Protocol`) | as cited | Yes |
| Reserve closed after one message | `handle_connection`: `if output.one_shot \|\| admission == AdmissionClass::AuthReserve` | as cited | Yes |
| `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` | `crates/daemon/src/limits.rs` | 200 | Yes |
| `LOCK_STATE_MAX_REQUESTS_PER_SEC` / window | `crates/daemon/src/lock_state.rs` | 8 / `1_000_000_000` ns, sliding (`crates/policy/src/rate_limit.rs::check_and_record`) | Yes |
| Quota is recorded only inside `handle_lock_state_request` | `dispatcher.rs::handle_lock_state_request` (`lock_state_limiter.check_and_record`) | as cited | Yes |
| Default dispatcher clock is real | `ConnectionDispatcher::new`: `clock_fn: current_monotonic_nanos` | as cited (the quota is live in `two_held`) | Yes |
| `DEFAULT_MAX_REQUESTS_PER_CONNECTION` | `limits.rs` | 1024 | Yes |
| Board API is public | `lock_state.rs`: `begin_scan`, `ScanGuard::finish`, `view`, `tracked_uids`; `dispatcher.rs::lock_state_board` | public | Yes |
| ADR 2026-10-13 item (5) "the auth-reserved connection still refuses it" | `AI/DECISIONS.md`, entry of 2026-10-13 | present | Yes |
| ADR 2026-10-15 item (3) accepted residual | `AI/DECISIONS.md`, entry of 2026-10-15 | present | Yes |
| Existing reserve test covers Status, PreviewFrame, PasswordFailed only | `peer_limits_tests.rs::auth_reserve::test_rsh_reserve_closes_non_auth_without_payload` | as cited | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: a widget poller at the regular cap repeatedly lands on the reserve and keeps it busy while PAM needs it.
- Result: PASS. The reserve is released after one classified frame (or 200 ms); LRR3 pins that an `Auth` is still answered after repeated attempts. The accept-time alternative is out of scope (it needs an S-3 design change and owner approval, spec §3) and is not invented here.

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: the PR touches `crates/pam` or the reserve window.
- Result: PASS. No production change; PAM untouched.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: the refusal path answers something (a `Response` or `LockStateResponse`) instead of EOF.
- Result: PASS. `closed_promptly` panics on any payload byte.

### Pillar 4 — Dependencies
- Result: PASS. No dependency change.

### Pillar 5 — Data confidentiality
- Failure scenario considered: the reserve answers `LockState` and leaks the board state over the reserve.
- Result: PASS. LRR1 asserts no payload; LRR2 asserts the board is not touched.

### Pillar 6 — Test integrity
- Failure scenario considered: the new tests pass even if the guard is removed (vacuous).
- Result: PASS with a required procedure: the tests pass on `main` by construction, so their power is shown by temporary mutations (guard removed; guard that records the `LockState` quota before refusing). Each mutation must fail at least one new test. LRR2's quota assertion is made non-vacuous by checking, when the requests fit inside the window, that the request beyond the quota is `RateLimited` (the quota is live in this harness). No existing test is edited.

## 4. Findings

- **[MINOR]** The quota-is-live check (request beyond the quota refused) depends on wall-clock time on a starved host; it must be conditional on the elapsed time being inside the window, never a flaky hard assertion. — Adopted in the test design.

## 5. Verdict
VALIDATION_VERDICT: APPROVED

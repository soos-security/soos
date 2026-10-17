# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `test/daemon-lockstate-reserve`
- **Base (merge-base)**: `9e82795`
- **Reviewed-Diff-Fingerprint**: `9780caa7204a16bba771254fc48ba242a3980a94b0079de36cf0fbafddef2d31`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_lockstate_reserve_refusal.md`, `AI/walkthroughs/217_lockstate_reserve_refusal.md`, `Docs/IPC_PROTOCOL.md`, `Docs/LOCKSCREEN_WIDGET.md`, `crates/daemon/tests/peer_limits_tests.rs`

## 1. Executive Summary

The change (GitHub #410, PR #403 follow-up) is test and documentation only: no production code is
touched. It adds a nested module `auth_reserve::lock_state_on_reserve` with three socket tests
(LRR1-LRR3) pinning that a `LockState` request arriving on the auth-reserved connection is refused
at first-frame classification, without payload, without consuming the per-UID `LockState` quota,
without reading or mutating the lock-state board, and that the reserve permit is released so a
following `Auth` is still served. The documentation and the ADR clarification entry describe
exactly what `ConnectionDispatcher::read_and_process` already does (`crates/daemon/src/dispatcher.rs`
lines 632-643: the non-`Auth` guard runs after `decode_client_message` and before `handle_request`,
hence before `handle_lock_state_request` and its `lock_state_limiter.check_and_record`). No
defect found.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `crates/daemon/tests/peer_limits_tests.rs` only (192 added lines, 0 removed).
- Removed or changed `assert`/`#[test]`/`#[tokio::test]`/`proptest!`/`#[should_panic]` lines:
  **none** (the `^-` grep on the frozen patch is empty; `git diff $(git merge-base origin/main HEAD) HEAD -- '*tests*' 'tests/'`
  contains no removed content line).
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: none. One new nested test module `lock_state_on_reserve` inside the
  existing `auth_reserve` module reuses the existing helpers (`two_held`, `permits_back`,
  `closed_promptly`, `exchange`, `auth_request`, `auth_response`) without modifying them.
- No pre-existing assertion was removed or changed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Checked that the third connection really lands on the reserve: `two_held` holds two served
  regular connections (6 free permits of 8) and the per-UID cap is 2, so the next connection is
  `AdmissionClass::AuthReserve` (same precondition as the existing `test_rsh_*` siblings). PASS.
- Checked the claimed ordering against the source: the guard at `dispatcher.rs:634` precedes
  `handle_request`, and the quota check is inside `handle_lock_state_request` (`dispatcher.rs:1209`).
  The docs' statement "not at accept, at first-frame classification" matches `handle_connection`
  choosing the admission class from `SO_PEERCRED` before any read. PASS.
- Constants: `LOCK_STATE_MAX_REQUESTS_PER_SEC = 8` is imported from the source of truth; the
  window `Duration` is a mirrored literal whose source is named in its doc comment. Nonce
  arithmetic stays below 0xFF (maximum 0x6F and 0xCF), no `u8` overflow. PASS.
- Scope: matches #410 (tests plus clarification docs), no scope creep. PASS.

### PAM Concurrency & Deadlines
- `crates/pam` is not touched. Every socket wait in the new tests is bounded (`closed_promptly`
  uses `CLOSE_DEADLINE`, `permits_back` a 2 s deadline). PASS.

### Panic Safety & Fail-Closed
- No production code changed. LRR3 additionally asserts `Verdict::Unavailable` for the reserve
  `Auth` with no pipeline (fail-closed, never `Allow`). PASS.

### Test Integrity & Anti-Weakening
- Discrimination reasoning: if the guard let `LockState` through on the reserve, the daemon would
  answer a `LockStateResponse` and `closed_promptly` panics on the payload (LRR1-LRR3 fail). If
  the guard ran after the quota check, the reserve request would consume a token and the 8th
  regular request in LRR2 (or the quota after 16 attempts in LRR3) would be refused, failing
  `assert_full_quota_available`. If the permit were not released, `permits_back` times out.
- The "beyond quota" check is conditional on the 9 requests fitting in one window; the quota is
  thus proven live on normal hosts, and on a very slow host the remaining assertions still hold.
  Acceptable and documented in the helper. PASS.
- The tests skip as root like their siblings; verified the suite runs as UID 1000 and the sibling
  `test_rsh_reserve_idle_connection_closed_within_window` takes 0.20 s, so the non-root path
  executes.

### Memory, Bounds & Secrets
- Test reads allocate `4 + len` from a daemon-produced frame header in a local test harness;
  acceptable for tests. No secrets, frames or embeddings involved. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. N/A.

### English-Only Policy
- All added code, comments, docs, spec and walkthrough text is in English. PASS.

### Commands re-run by the reviewer
- `cargo test -p soos-daemon --test peer_limits_tests -- --test-threads=3`: 31 passed, 0 failed
  (LRR1-LRR3 included; the LRR subset was re-run three more times, all green).
- `cargo clippy -p soos-daemon --all-targets -- -D warnings`: clean.
- `cargo fmt -p soos-daemon -- --check`: clean.
- All with `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/daemon-lockstate-reserve CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=3`.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/daemon/tests/peer_limits_tests.rs` (`QUOTA_WINDOW`) — the 1 s window is
  a literal mirroring `LOCK_STATE_RATE_WINDOW_NS`; `Duration::from_nanos(LOCK_STATE_RATE_WINDOW_NS)`
  would keep a single source of truth. Optional.

No CRITICAL, MAJOR or MINOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**

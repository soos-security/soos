# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `feat/lockscreen-status`
- **Base (merge-base)**: `6f32ef8`
- **Reviewed-Diff-Fingerprint**: `10fbf20dbd62bff628ca01050faf1cc5a96953fa4f62617f2ad7640de6c82010`
- **Audited Files**: `.github/workflows/fuzz.yml`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_lockscreen_status.md`, `AI/auditor_constraints_lockscreen_status.md`, `AI/walkthroughs/205_lockscreen_status.md`, `Docs/DAEMON.md`, `Docs/IPC_PROTOCOL.md`, `crates/daemon/src/dispatcher.rs`, `crates/daemon/src/lib.rs`, `crates/daemon/src/lock_state.rs`, `crates/daemon/src/main.rs`, `crates/daemon/src/presence/worker.rs`, `crates/daemon/tests/lock_state_board_tests.rs`, `crates/daemon/tests/lock_state_dispatcher_tests.rs`, `crates/daemon/tests/lock_state_presence_tests.rs`, `crates/protocol/fuzz/Cargo.toml`, `crates/protocol/fuzz/README.md`, `crates/protocol/fuzz/fuzz_targets/decode_lock_state.rs`, `crates/protocol/src/lib.rs`, `crates/protocol/src/message.rs`, `crates/protocol/src/types.rs`, `crates/protocol/tests/lock_state_tests.rs`, `tests/invariants/src/lib.rs`, `tests/invariants/src/lock_state_contract.rs`

## 1. Executive Summary

Re-review of GitHub #380 (PR 1 of #352): read-only `RequestKind::LockState` answered with a
12-byte `LockStateResponse { version, state: Idle|Scanning|NotRecognized, sequence }` from a
bounded, clockless, non-biometric `LockStateBoard` fed by the PAM `Auth` path and the presence
worker. The previous review's MAJOR (next presence scan start depended on the previous scan's end)
is fixed: `PresenceWorker::post_scan_hold` holds after every non-unlocking scan until
`attempt_ns + NOT_RECOGNIZED_REVEAL_FLOOR_MS`, so the next tick lands at start + floor + 1 s
whatever the end. Both previous suggestions are addressed (90 ms reveal-floor margin derived from
constants; logind sessions scan moved to `spawn_blocking` under a 250 ms timeout failing to
`{Idle, 0}`). `pam_soos.so` is untouched; PAM `Auth` latency only gains two synchronous,
non-awaiting `watch::Sender` updates. No pre-existing test was modified. No CRITICAL or MAJOR
finding remains.

Local evidence (this review): `cargo test -p soos-daemon --test lock_state_presence_tests
--test lock_state_board_tests --test lock_state_dispatcher_tests` 10+17+15 passed;
`cargo test -p soos-protocol --test lock_state_tests` 13 passed; `cargo test -p soos-invariants
lock_state` 9 passed; `cargo clippy -p soos-daemon -p soos-protocol --all-targets -- -D warnings`
clean.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `crates/daemon/tests/lock_state_{board,dispatcher,presence}_tests.rs`,
  `crates/protocol/tests/lock_state_tests.rs`, `tests/invariants/src/lock_state_contract.rs` (all
  new files), `tests/invariants/src/lib.rs` (+6 lines: a `mod lock_state_contract;` declaration
  only).
- Removed/changed assertions (`^-` with assert/#[test]/proptest/should_panic): **none**.
- New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: **none**.
- Conclusion: purely additive; no contract migration needed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Fix of MAJOR #1 (cadence).** `run()` now sleeps `LOCK_POLL_INTERVAL_MS + post_scan_hold(tick)`;
  for `Scanned(non-Unlocked)` the hold is `(last_scan_started_ns + REVEAL_FLOOR_NS) - now`, capped
  at the floor, so the next tick starts at `start + floor + poll + ε` independent of the end
  (spoof veto at the first capture vs full non-match at the budget). `tracker.due()` is already
  start-anchored (`last_scan_started_ns + scan_interval`) and the tick grid after the hold is now
  start-anchored too, so the residual "end mod poll interval" quantisation that existed before is
  also gone. Unlock and skipped ticks: `Duration::ZERO` (no unlock-latency change). Clock failure
  during the hold computation: full floor (bounded, outcome-independent). `Preempted`/
  `ShuttingDown` also hold — harmless (PAM is never delayed; shutdown is still observed through the
  `select!`). PASS.
- **Visible trace.** Reveal at `max(end, start + floor)`; next scan at `≈ start + floor + 1 s`,
  i.e. ~1 s after reveal, before the 3 s `NotRecognized → Idle` decay, so both failure kinds render
  `Idle0 → Scanning1 → NotRecognized2 → Scanning3`. With a larger `scan_interval` the Idle decay
  and the next scan are both start-anchored, so still identical. PASS.
- **Sequence numbers.** Hidden phase reads `2·scans − 1`, revealed `2·scans`; a superseding scan
  (PAM or presence) moves `2k−1 → 2k+1` identically whether the earlier end existed or not; a
  superseded guard's `finish`/`Drop` is a no-op (`scan_id` check). Abandoned scans (preempted, `?`
  early return in the PAM path) reveal `Idle` at the floor — reveals only "no claim". PASS.
- **Floor derivation.** 1200 (`MAX_CAMERA_WAKE_WAIT_MS`) + 1000 (`PRESENCE_WAKE_SETTLE_MS`) + 900
  (`DECISION_BUDGET_MS`) + 50 (`RESPONSE_WRITE_MARGIN_MS`) + 90 (`DEFAULT_INFERENCE_ESTIMATE_MS` 80 +
  `FRAME_POLL_INTERVAL_MS` 10) = 3240 ms, all `saturating_add` of the source constants (single source
  of truth); matches `scan()` (wake bound, settle `wait_ms ≤ settle_ms`, deadline = settle end +
  budget + margin). Docs/ADR/DAEMON.md agree. PASS (residual on slow hardware: see SUGGESTION 1).
- **Other timing channels searched.** (a) Post-Allow failures (`SessionChanged`, `AccountRefused`,
  `LidClosed`, `UnlockFailed`) can end after the floor only when bounded D-Bus calls stall; they
  require an already matching face, so not a PAD oracle — documented accepted residual in the ADR.
  (b) PAM scans with `timeout_ms > 2500` may reveal at their end; the PAM caller (the locker)
  already sees that timing — documented. (c) `UnlockFailed` starts the logind backoff (pre-existing,
  post-match only). (d) `LockState` answer path: no reason, no timestamp, no other UID's data;
  neutral `{Idle,0}` for clock error, no local seat session, timeout or failed blocking job; quota
  refusal is a standard `Response{ProtocolError, RateLimited}` keyed on peer UID, not on board
  state. No new channel found.
- **Authorization.** `LockState` is dispatched after wire validation and `verify_peer_credentials`
  (unprivileged peer only for its own UID → `UidMismatch` otherwise; root any UID), then per-peer
  quota (8/s, 64 tracked UIDs), then `has_local_seat_session(uid_hint)` (the preview predicate).
  Never records an attempt, wakes the camera, registers PAM priority or unlocks (tests LKS13, board
  purity, dispatcher side-effect test). Auth-reserved connection still admits only `Auth`. PASS.
- **Wiring.** `main.rs` creates one board and hands clones to the dispatcher and the presence
  worker. PASS.

### PAM Concurrency & Deadlines
- `crates/pam` not in the diff; no Tokio, deadlines, `catch_unwind` or `PAM_IGNORE` paths changed.
  Daemon `Auth` path: `begin_scan`/`finish` are synchronous `send_modify`/`send_if_modified` (no
  await, no I/O, no logging); no post-scan hold exists in the dispatcher. PASS.

### Panic Safety & Fail-Closed
- Production hunks: no `unwrap/expect/panic!/todo!/unreachable!`, no indexing; all arithmetic
  saturating; `now < stamp` handled by `saturating_sub` (board test covers it). `spawn_blocking` +
  `timeout`: only `Ok(Ok(true))` serves data; `JoinError`/elapsed/`false` → neutral. The
  `ScanGuard` `Drop` settles to `Idle` (never `NotRecognized`, never a success claim). Only
  `Verdict::Allow` maps to `Recognized`, which reads `Idle` — no state ever reads as success.
  PASS.

### Test Integrity & Anti-Weakening
- LKS19 `test_lks_fast_and_slow_presence_failures_schedule_the_next_scan_identically` drives the
  real `run()` loop: with the old behaviour (`scan_interval` 1000 ms in `fast_presence_config`)
  the fast gap would be ≈1.0 s and the slow ≈1.9 s, failing both the `expected ≈ 4.24 s` window and
  the 250 ms equality bound; it also asserts the identical 4-step visible trace. The
  `post_scan_hold` unit test pins the anchoring to the scan start (±5/50 ms) for both outcomes and
  `ZERO` for unlock and skipped ticks. Mapping tests cover no-match, spoof, camera unavailable,
  post-match refusal, preemption, unlock, and every PAM failure class. No pre-existing test
  touched. PASS.

### Memory, Bounds & Secrets
- Board: `MAX_LOCK_STATE_UIDS` = 32 with LRU eviction; entries hold counters and monotonic stamps
  only. Quota table 64 UIDs. Response ≤ 12-byte payload, encoded via the bounded codec into
  `Zeroizing`. Logs: `debug!` with `peer_uid` only; no frames, embeddings, reasons or credentials.
  Fuzz target `decode_lock_state` asserts no panic, closed state set, size bound, round trip and
  disjointness from `Response`; added to the CI fuzz matrix. PASS.

### Supply Chain & Automation
- `.github/workflows/fuzz.yml`: matrix string only; SHA pins and permissions unchanged. No new
  dependency (`Cargo.toml` change is a fuzz `[[bin]]`). PASS.

### English-Only Policy
- All code, comments and docs in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/daemon/src/lock_state.rs:48` — the 3240 ms floor bounds a regular
  non-match only while inference admission overrun stays within one nominal estimate (80 ms); on
  hardware whose real inference greatly exceeds the estimate a full non-match could end past the
  floor and become distinguishable from an early veto. Consider a `debug!`-level counter (no
  outcome detail) when a non-unlocking presence scan ends after the floor, to detect this in the
  field.
- **[SUGGESTION]** `crates/daemon/src/dispatcher.rs:1232` — on timeout the `spawn_blocking`
  sessions scan keeps running detached and is not tracked by `BlockingTasks`; bounded by the 8/s
  per-UID quota and harmless (read-only `read_dir`), but tracking it would keep shutdown draining
  symmetric with the evidence writes.
- **[SUGGESTION]** `crates/daemon/src/presence/worker.rs:675` — post-Allow failures
  (`SessionChanged`, `AccountRefused`, `LidClosed`, `UnlockFailed`) can be revealed after the floor
  when bounded D-Bus calls stall; accepted and documented in the ADR (needs an already matching
  face), noted here for the owner's confirmation on #352.

## 5. Final Verdict

**VERDICT: APPROVED**

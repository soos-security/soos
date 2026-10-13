# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `test/deflake-rwp-delivery`
- **Base (merge-base)**: `6f32ef8`
- **Reviewed-Diff-Fingerprint**: `d9dd8b629c43b27f029c539511e490660ca65b9acd5de395d6e70fa304ab1371`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/204_deflake_rwp_delivery.md`, `crates/remote/tests/android_push_tests.rs`, `crates/remote/tests/common/push.rs`, `crates/remote/tests/push_server_tests.rs`

## 1. Executive Summary

The diff deflakes `test_rwp_delivery_outcomes` (GitHub #371) by changing only the test harness. `FakeTransport` now records the scripted answer of each call. A new `settle_gone` helper, called from `wait_calls` in both push test files, waits in real time with a bound and without moving the paused clock, until the dispatcher has consumed the blocking-pool removal of the latest `Gone` subscription. The diff touches no production file under `crates/remote/src`. No assertion, virtual bound, step, timeout or panic message was changed or removed, and no `#[ignore]` or retry was added. The root-cause analysis is consistent with the code: `remove_gone` awaits `with_store_lock(remove_endpoint)` on the blocking pool, and with a frozen clock, virtual time moves only through `advance`. I found no CRITICAL or MAJOR issue.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `crates/remote/tests/android_push_tests.rs`, `crates/remote/tests/common/push.rs`, `crates/remote/tests/push_server_tests.rs`.
- Removed or changed checks (`^-` lines with assert/#[test]/#[tokio::test]/proptest!/#[should_panic]): **none**. The only removed lines in the whole patch are a doc comment (`common/push.rs`, "One recorded call ...") and the single `calls.push(Call { at_ms, request })` statement, which moved below the queue pop so that the record carries `answer`. `at_ms` is still computed first, so the recorded timestamps do not change.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none** in code. The only match is prose in walkthrough 204 stating that none were added.
- Inline `mod tests` changes: **none**.
- Justification: amendments CA-1 to CA-3 (walkthrough 204 §4) only add synchronisation and recording, with every assertion preserved.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: the dispatcher never removes the `Gone` endpoint (regression in `remove_gone`).
  - **Block 1** (two subscriptions, first answer `Gone`): `settle_gone` exits once the second call is recorded (`count > index + 1`). The unchanged assertions that follow still fail: `subscriptions == 1`, store length 1, the remaining endpoint differs from the gone one, and exactly one `push subscription removed` audit line.
  - **Block 2** (single subscription): neither exit condition can hold, so the `GONE_WAIT` assertion panics after 10 s.
  - **Android** test: the unchanged `wait_view(subscriptions == 3)` and the `only the gone endpoint is removed` set equality still fail.
  - The wait therefore cannot mask that regression → PASS.
- Scenario: the dispatcher removes the endpoint but never records `gone`. The store check alone is not enough to exit; exit requires a later call or `last_delivery == "gone"`. Otherwise the wait panics → PASS.
- Scenario: a stale exit in a later `wait_calls` on the same transport. `last_gone` returns an earlier index for which `count > index + 1` already holds, so the helper exits at once without hiding anything. The re-subscribe assumption is documented, and no test re-subscribes a gone endpoint → PASS.
- Scenario: `store.load()` racing the atomic temp-file plus rename write. A read error yields "not stored", but exiting also requires the view to report `gone`, which `record_gone` sets only after the join → PASS.
- Scenario: lock ordering in `FakeTransport::deliver`. The queue and default locks are released at the end of their statement before the calls lock is taken, and `last_gone` takes only the calls lock. There is no new deadlock → PASS.
- `wait_calls` still returns virtual `elapsed`. `settle_gone` never calls `advance`, so virtual-time measurements (for example `let waited = wait_calls(...)`) are unaffected → PASS.

### PAM Concurrency & Deadlines
- Not applicable: the diff does not touch `crates/pam`. The real-time wait is bounded (`GONE_WAIT` 10 s per wait, 1 ms sleeps), below the frozen clock's 20 s `WALL_CLOCK_FALLBACK` → PASS.

### Panic Safety & Fail-Closed
- The new `unwrap()` and `assert!` calls are confined to integration-test code, and production is untouched. No path to `Allow` or `PAM_SUCCESS` → PASS.

### Test Integrity & Anti-Weakening
- I checked that the virtual bounds and steps (`10_000`/`100`, `PUSH_COALESCE_MS + JOURNAL_IDLE_TICK_MS + 10_000`) are unchanged, as are the panic messages and every assertion. The new helper only adds a stricter, bounded precondition → PASS.
- `fmt --check` is clean. `cargo clippy --locked -p soos-remote --all-targets --all-features -D warnings` is clean. `push_server_tests` (22 tests) and `android_push_tests` (5 tests) pass → PASS.

### Memory, Bounds & Secrets
- Polling is bounded in time. Recording `Scripted` (a `Copy` value) per call adds no secret. No logging was added → PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change → PASS.

### English-Only Policy
- All code, comments, the walkthrough and the matrix rows are in English → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/remote/tests/android_push_tests.rs:316` / `crates/remote/tests/push_server_tests.rs:471`: `settle_gone`, `GONE_WAIT` and `GONE_WAIT_SLEEP` are duplicated byte for byte. This is acknowledged in walkthrough 204 §3 and follows the existing `wait_view` precedent. A future change could move them into `common/push.rs` behind a small `push_view` callback.
- **[SUGGESTION]** `crates/remote/tests/android_push_tests.rs:294` and `crates/remote/tests/push_server_tests.rs:448`: the doc comments say "the clock waits" where the helper is what waits, since the clock never moves. Consider "the helper first waits, in real time, ...".
- **[SUGGESTION]** `AI/walkthroughs/204_deflake_rwp_delivery.md:24`: the residual window (two consecutive `Gone` answers that end a round) is documented but not covered by any test. That is acceptable while no test scripts it.

## 5. Final Verdict

**VERDICT: APPROVED**

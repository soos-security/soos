# Walkthrough 204 — Deflake the Web Push Delivery Tests (Gone Removal Race)

- **Date**: 2026-10-13
- **Issue**: GitHub-only #371, no backlog id. **Branch**: `test/deflake-rwp-delivery`. It is not registered in `scripts/sync_issue.py`; the commit message carries `Closes #371`.
- **Matrix criterion**: RWPF1 (component `rwp-delivery-deflake`)
- **Numbering**: 196 to 203 are reserved by other open branches.

## 1. Context & Objectives

`crates/remote/tests/push_server_tests.rs` `test_rwp_delivery_outcomes` failed once in CI (PR #368, run 38371004266) with `2 calls not reached within 10000 ms (1 so far)` in the `wait_calls` helper. The PR changed no Rust file of `soos-remote`, the test passed 15 times out of 15 locally, and the re-run of the job passed. The goal was to find the real cause and fix the synchronisation, without raising a bound or weakening an assertion.

## 2. Root Cause

The failing block is the first one of the test: two subscriptions, and the first answer is `Gone` (404, then 410).

- **Production path** (`crates/remote/src/push.rs`): `send_summary` delivers to each subscription in turn. A `Gone` answer calls `remove_gone`, which runs `with_store_lock(remove_endpoint)` on the blocking pool (`spawn_blocking`: lock, load, write a temp file, `fsync`, rename). The dispatcher awaits that join before it delivers to the next subscription. This is the only blocking-pool job the dispatcher awaits: the summary, retry and test rounds read the store inline.
- **Harness**: `wait_calls` repeats `pump()` (64 `yield_now`) and `tokio::time::advance(100 ms)`. The frozen clock (a held blocking task) prevents auto-advance, so virtual time moves only through `advance`. The 100 steps of the 10 000 ms budget take a few wall-clock milliseconds.
- **The race**: when the real `fsync` lasts longer than those few milliseconds, the virtual budget runs out while the dispatcher is still waiting for the store write. The second call has not happened yet, and the assertion fires.
- **Why it passed locally**: `/tmp` is a tmpfs on the development machine, so `fsync` is free. GitHub runners keep the `TempDir` on a disk-backed filesystem, where `fsync` can take tens of milliseconds or more under load.

This is not a production bug. Delivering to the next subscription only after the removal is the specified behaviour. Writing the store inline would stall the current-thread runtime, which the candid review of 2026-10-06 rejected. Detaching the write would break the ordering that keeps `GET /api/push` consistent.

## 3. Fix

- **`crates/remote/tests/common/push.rs`**: each `Call` now records the scripted `answer` it was given. `Call::answered_gone()` and `FakeTransport::last_gone()` (index and endpoint of the latest `Gone` call) are new. The answer is taken before the call is recorded, both inside the same synchronous `deliver`.
- **`settle_gone`** (in `push_server_tests.rs` and `android_push_tests.rs`): `wait_calls` calls it after each `pump()`, before it checks the count or moves the clock. It waits in real time (1 ms sleeps between pumps, never moving the clock) until the dispatcher has consumed the removal of the latest `Gone` call. One of two things must hold:
  - a later transport call was recorded, or
  - the endpoint left the store file and `GET /api/push` reports `last_delivery: "gone"`. `record_gone` runs on the runtime thread right after the join, so the view proves the dispatcher resumed.
- **Bound**: the new real-time bound `GONE_WAIT` is 10 s per wait, shorter than the frozen clock's 20 s `WALL_CLOCK_FALLBACK` (which counts from `FrozenClock::hold` for the whole harness). Reaching it means a broken test, not a slow runner. A call index already settled is skipped within one `wait_calls`. The helper assumes a gone endpoint is not subscribed again before a later `wait_calls` on the same transport; no test does that.
- **Duplication**: `settle_gone`, `GONE_WAIT` and `GONE_WAIT_SLEEP` are copied into both test files, like the existing `wait_calls` and `wait_view`, because each file has its own `Harness` setup and `push_view`.
- **Sibling**: `android_push_tests.rs` `test_ran_endpoints_subscribe_store_and_dispatch` has the same hazard (the first of four answers is `Gone`, then `wait_calls(4, ...)`) and gets the same helper. The second `Gone` block of `test_rwp_delivery_outcomes` already used the real-time `wait_view` (2026-10-07 migration). No other test scripts `Outcome::Gone`. The retry, unavailable and timeout cases do no blocking-pool work in the dispatcher.
- **Residual**: two consecutive `Gone` answers, where the second ends the round, could be seen as settled after the second store write but a few instructions before the join wakes the dispatcher. No test scripts that case, and the virtual budget still covers that window.

## 4. Contract Amendments (harness only, every assertion preserved)

| ID | Change | Justification |
|---|---|---|
| CA-1 | `push_server_tests.rs`: `wait_calls` calls the new `settle_gone` (`GONE_WAIT`, `GONE_WAIT_SLEEP`) | Synchronises with real blocking-pool work the helper ignored; the virtual bound, step and panic message are unchanged |
| CA-2 | `android_push_tests.rs`: the same change to its `wait_calls` | Same hazard after the first `Gone` answer |
| CA-3 | `common/push.rs`: `Call.answer`, `Call::answered_gone`, `FakeTransport::last_gone` | Lets the harness know which call triggers the store removal |

No assertion, timeout, virtual bound or step was changed or removed. No `#[ignore]` and no retry was added. Production code is unchanged.

## 5. Phases

- **Plan** (condensed cycle): root cause, fix and amendments as above.
- **Plan evaluator**: `VALIDATION_VERDICT: APPROVED` (`AI/plan_evaluator_report.md`), five MINOR findings: the doc comments now state the re-subscribe assumption and the per-wait scope of `GONE_WAIT`, and §3 records the duplication and the residual.
- **Auditor**: `Clearance: CLEARED`.

## 6. Reproduction and Proof

Stress loop: the test binary filtered with `--exact --test-threads=1`, 8 instances in parallel pinned with `taskset -c 0-1`, 4 busy loops on the same CPUs, 3 `dd ... conv=fsync` writers, and `TMPDIR` on ext4.

| Test | Before | After |
|---|---|---|
| `test_rwp_delivery_outcomes` | 26 failures in 40 runs, all with the CI message at the `wait_calls` assertion | 0 failures in 200 runs |
| `test_ran_endpoints_subscribe_store_and_dispatch` | 0 in 40 (larger virtual budget) | 0 in 40 |

With `TMPDIR` on tmpfs, the unfixed test passed 40 runs out of 40 under the same CPU load, which confirms that `fsync` latency is the cause.

## 7. Quality Gates

- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo test --locked --workspace --all-targets --all-features`
- `./scripts/candid_review.sh` (layer 1)

All four pass on the final tree. Layer 2 of the candid review runs in the release step.

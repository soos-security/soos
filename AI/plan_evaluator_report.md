# Plan Evaluation Report
- **Date**: 2026-10-13
- **Issue**: GitHub #371 — deflake `test_rwp_delivery_outcomes` (no backlog id; condensed cycle for a test-harness fix)
- **Branch**: `test/deflake-rwp-delivery`
- **Base commit**: `6f32ef8`
- **Plan**: `plan_371.md` (session scratchpad); implementation evaluated from the working-tree diff of
  `crates/remote/tests/common/push.rs`, `crates/remote/tests/push_server_tests.rs`,
  `crates/remote/tests/android_push_tests.rs`. Production code (`crates/remote/src/push.rs`) is unchanged.

## 1. Coverage Matrix
| Acceptance line / TDD test | Spec element | Status |
|---|---|---|
| #371: `test_rwp_delivery_outcomes` must not fail with `2 calls not reached within 10000 ms` on CI | `settle_gone` called from `wait_calls` (push_server_tests.rs) | Covered |
| Sibling hazard: `test_ran_endpoints_subscribe_store_and_dispatch` (first of 4 answers `Gone`, then `wait_calls(4, ...)`) | Same helper in android_push_tests.rs | Covered |
| Second gone block of `test_rwp_delivery_outcomes` (`set_default(Gone)`, `wait_calls(1)` then `wait_view`) | Already real-time via `wait_view` (2026-10-07 migration); now also settled in `wait_calls` | Covered |
| Zero Test Weakening (AGENTS.md) | Contract amendments CA-1..CA-3, harness-only; no assertion, bound or step changed | Covered (verified in diff) |
| Every other `Outcome::Gone` script in `crates/remote/tests` | grep: only push_server_tests.rs:1211, :1231 and android_push_tests.rs:484 | Covered |

## 2. Facts Verified Against Code
| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `Gone` -> `remove_gone` -> `with_store_lock` -> `spawn_blocking(remove_endpoint)`, awaited before next subscription | push.rs:1629-1636, :537-556, :1718-1745 (`send_summary` loop; `deliver_alert` awaits `remove_gone`) | as cited | Yes |
| `record_gone` runs on the runtime thread right after the join | push.rs:1635 (after `with_store_lock(..).await`) | as cited; also called when the removal fails | Yes |
| Dispatcher rounds read the store inline (no other awaited blocking job) | push.rs `send_summary`/`send_retry`/`send_test` use `self.store.load()` | as cited | Yes |
| `PushStore::load` takes no lock (a test-side read cannot cause `Locked`) | push.rs:294-309 (no `try_lock_file`) | no lock | Yes (load-bearing: a `Locked` would make `with_store_lock` sleep in virtual time, which `settle_gone` never advances) |
| `GET /api/push` reads the store first, the status after | push.rs:1393-1430 (`view`) | as cited | Yes |
| `WALL_CLOCK_FALLBACK` = 20 s, `GONE_WAIT` = 10 s | tests/common/harness.rs:55; diff | 20 s / 10 s | Yes |
| `FrozenClock` blocks auto-advance | harness.rs:670-690 | blocking task held until drop or 20 s | Yes |
| Answer popped before the call is recorded, same synchronous `deliver` | common/push.rs diff | as cited | Yes |

## 3. Pillar Analysis
### Pillar 1 — Architecture & threat model
- Failure scenario considered: the fix moves the store write inline or detaches it, changing production ordering.
- Result: PASS. Production untouched; the plan rejects both production alternatives with reasons consistent with the 2026-10-06 candid review and the view-consistency ordering of `view()`.

### Pillar 2 — Deadline & concurrency (here: harness virtual-time semantics, hang risk)
- Scenario A: `settle_gone` holds the clock while the dispatcher needs virtual time to finish the removal. Only possible via the `Locked` retry `sleep` of `with_store_lock`; no concurrent flock holder exists in these tests (`load()` and the `GET /api/push` route do not lock), so excluded. If it ever happened, the helper fails loudly after 10 s rather than hanging.
- Scenario B: a stale gone index in a later `wait_calls` keeps the clock frozen (e.g. the same endpoint re-subscribed after its `Gone`). `settled` is per call, so a later `wait_calls` re-checks the old gone; it breaks immediately when `count > index + 1`, or via `!stored && last_delivery == "gone"`. Every gone script is in a block with a fresh harness/transport and no re-subscribe, so excluded today; see finding 2.
- Scenario C: the virtual time at which calls happen changes. `settle_gone` never advances the clock, so `at_ms` and the returned `elapsed` can only become more deterministic (the next call always happens at the virtual step it gets on a fast disk). No assertion relied on a step that a slow fsync used to skip.
- Scenario D: `std::thread::sleep` on the current-thread runtime starves the blocking pool. Blocking-pool threads are separate; same established pattern as `wait_view`.
- Result: PASS with MINOR observations.

### Pillar 3 — Panic safety & fail-closed
- Scenario: the production removal fails (`Io`), the endpoint stays stored, `record_gone` still runs. `settle_gone` spins 10 s and panics with "the removal of a gone subscription never completed" — a failing test, as the old code would also produce (subscriptions == 1 assertion). Test-only code; `unwrap` in the harness is acceptable.
- Result: PASS.

### Pillar 4 — Dependencies
- No new crate, no Cargo change. PASS.

### Pillar 5 — Data confidentiality
- `Call` now stores the scripted answer (outcome, status, retry-after only); nothing logged. PASS.

### Pillar 6 — Test integrity
- The diff is additive in the helpers; `wait_calls` keeps its `max_ms`, `step`, panic message and return value; no assertion, `#[ignore]`, retry or bound changed. Amendments are listed (CA-1..CA-3) and justified as synchronisation with real blocking-pool work.
- Power: if production stopped removing gone subscriptions or stopped delivering to the next subscription, the tests still fail (settle panic, or the unchanged count/store/audit assertions).
- Real synchronisation, not a raised bound: the waiter observes completion of the exact awaited job (store file without the endpoint plus `last_delivery == "gone"`, written only after the join) or the next call.
- Evaluator reproduction: `push_server_tests` 22/22 and `android_push_tests` 5/5 pass; 40 parallel runs (8 at a time, `taskset -c 0-1`, `TMPDIR` on ext4) of `test_rwp_delivery_outcomes`: 0 failures. The plan's before/after numbers (26/40 -> 0/200) were not re-measured here.
- Result: PASS.

## 4. Findings
1. **[MINOR]** `settle_gone`, `GONE_WAIT` and `GONE_WAIT_SLEEP` are duplicated verbatim in `push_server_tests.rs` and `android_push_tests.rs`. Acceptable since each file owns its `Harness`/`push_view`; mention the duplication in the walkthrough so a future edit updates both.
2. **[MINOR]** `settle_gone` assumes a gone endpoint is not re-subscribed before a later `wait_calls` on the same transport (otherwise it waits for a removal that already happened and panics after 10 s). No current test does this; state the precondition in the helper doc comment.
3. **[MINOR]** `GONE_WAIT` (10 s) is below `WALL_CLOCK_FALLBACK` (20 s) per wait, but the fallback is measured from `FrozenClock::hold` for the whole harness; the doc comment should say it is a per-wait bound, not a guarantee on the cumulative harness budget.
4. **[MINOR]** The documented residual (a gone answer ending the round observed as settled a few instructions before the join wakes the dispatcher) is correctly bounded by the next `pump()`/advance and does not affect the virtual budget. No change required.
5. **[MINOR]** Traceability: `AI/walkthroughs/204_deflake_rwp_delivery.md` and an `AI/VERIFICATION_MATRIX.md` edit already exist in the working tree; confirm 196–203 are really reserved so numbering does not collide on merge.

No CRITICAL or MAJOR finding.

## 5. Verdict
VALIDATION_VERDICT: APPROVED

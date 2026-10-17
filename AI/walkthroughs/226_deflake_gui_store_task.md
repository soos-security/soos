# Walkthrough 226 — Deflake the Held-Store-Lock Enroll Task Test

- **Date**: 2026-10-18
- **Issue**: GitHub #425, sub-issue of #291 (GitHub-only, no backlog id) — **Branch**: `test/deflake-gui-store-task`
- **Matrix criteria**: DGS1, DGS2 (new). SGU1 and SGU2 keep the same tests and status, with a note on the hardened wait helper.

## 1. Context & Objectives

`crates/gui/tests/store_task_tests.rs::test_sgu_enroll_submit_never_blocks_on_a_held_store_lock` failed once in CI (run 39062359937, attempt 1; attempt 2 passed) with `no store task outcome within 10 s`. The goal was to tell a production defect from a test timing problem, and to fix a test timing problem in the test's synchronisation only, without changing any assertion.

## 2. Architect Design

### Evidence from the failing run

- The `store_task_tests` binary started at 19:29:08.651. The enroll test panicked at 19:29:20.889, about 12.2 s in.
- `test_sgu_delete_submit_never_blocks_and_keeps_the_template_when_busy` ran concurrently with its own tempdir and runner. It reported `ok` 2.7 ms after that panic, after about 12.2 s instead of its usual ~1 s.
- The first phase of the enroll test (the 600 ms `LOCK_WAIT` plus lock polls) puts the second `wait_outcome` deadline at about 19:29:19.3. A loop that sleeps 5 ms between polls should panic within a few milliseconds of it. It panicked about 1.6 s late, at the same instant the unrelated test finished.

Two independent tests stalling together and resuming together fits the whole test process being descheduled or starved (CPU or I/O) across the deadline. It does not fit a worker that never sends an outcome.

### Production path (unchanged, bounded)

- `StoreTaskRunner::submit` only spawns a thread.
- The worker runs the job under `catch_unwind`, always calls `tx.send`, then `notify`.
- `poll` drains `try_iter` and synthesizes a failure for a finished worker that sent nothing.
- `BiometricStore::lock_store` gives up after `lock_timeout`.
- The only unbounded step is `fsync` inside `write_template` / `delete`, which the GUI does not control. The test wants a real write, so that is expected.

### Root cause

The previous helper was:

```rust
while Instant::now() < deadline { poll; sleep(5 ms) }
panic!("no store task outcome within 10 s");
```

It checks the deadline before polling. After a stall that carries the clock past the deadline, the loop exits without one last poll, even when the outcome is already queued. The unit-test helper `worker_failure_tests::wait_outcome` in `crates/gui/src/store_tasks.rs` already uses the correct order (receive first, then check the deadline).

### Design

- A generic private helper `poll_until(deadline_passed, poll, pause) -> Option<T>` polls first and checks the deadline after, so there is always a final poll after the deadline.
- The deadline is an injected predicate. `after(timeout)` builds the real-clock one; the DGS1 test injects a flag that its `pause` sets, so the simulated stall does not depend on host timing.
- `wait_outcome` calls it with `after(10 s)` and the same 5 ms pause. It keeps the `one outcome per task` assertion and the `no store task outcome within 10 s` panic text.
- The optional notify-driven wait (a channel fed by the `notify` closure of `setup()`) was not adopted. It would change the `setup()` tuple and every test that destructures it, and the 5 ms pause is not what failed.

Invariants touched: none. No production, PAM, IPC, `unsafe`, logging or Cargo change.

### Limitation

The fix removes the missing final poll. It does not make a run pass when the worker itself really needs more than 10 s (for example an `fsync` stalled for over 10 s). Such a run still fails at the bound, which is intended.

## 3. Tester Contract (Red)

Two new tests in `crates/gui/tests/store_task_tests.rs`:

- `test_dgs_wait_polls_once_more_after_a_stall_past_the_deadline` (DGS1): a `pause` that marks the deadline as passed (the process wakes up after the deadline) and a source that yields on its second poll. The test expects `Some(2)`. It uses no real clock, so it cannot itself flake under load. (A first draft used a 20 ms real timeout and a 60 ms sleep; it was dropped because a stall before the first deadline check would have failed it.)
- `test_dgs_wait_still_gives_up_after_the_timeout` (DGS2): with the real-clock `after(30 ms)` predicate, a source that never yields gives up with `None` after at least the 30 ms timeout and well under 10 s.

Red run, with `poll_until` first written in the old deadline-first order:

```
test test_dgs_wait_polls_once_more_after_a_stall_past_the_deadline ... FAILED
assertion `left == right` failed: the outcome queued during the stall must be returned by a final poll
  left: None
 right: Some(2)
```

DGS2 passed in both orders, as expected for a boundedness guard.

## 4. Auditor Constraints

1. No change to any assertion, message or `#[test]` of the SGU tests. The only removed lines are the old `wait_outcome` body; its `one outcome per task` assertion and its panic text are kept verbatim in the new body.
2. No production code change (`crates/gui/src/store_tasks.rs`, `crates/biometric-store`).
3. The wait stays bounded at 10 s.
4. The new tests must not be load-sensitive themselves: DGS1 uses no clock at all, and DGS2 only asserts lower bounds that hold by construction plus a 10 s upper bound.

## 5. Developer (Green) & Validation

- `poll_until` now uses `loop { poll; if deadline passed return None; pause }`.
- `cargo fmt`, `cargo clippy -p soos-gui --all-targets -- -D warnings` and `cargo test -p soos-gui --test store_task_tests -- --test-threads=3` are clean (8 passed).
- Load run: 50 runs of the `store_task_tests` binary, pinned to CPU 0 at `nice 19` next to four busy loops on CPU 0 and a looping `dd bs=1M count=64 conv=fsync` writer: 50/50 passed (53 s total). The harness did not reproduce the CI stall itself, so the evidence for the fix is the deterministic DGS1 Red/Green pair.

## 6. Changed Tests

- `crates/gui/tests/store_task_tests.rs`: `wait_outcome` (helper) rewritten on top of the new `poll_until` helper; new tests `test_dgs_wait_polls_once_more_after_a_stall_past_the_deadline` and `test_dgs_wait_still_gives_up_after_the_timeout`. The SGU tests are unchanged.

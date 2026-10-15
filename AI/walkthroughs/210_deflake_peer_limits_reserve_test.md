# Walkthrough 210 — Deflake the Auth-Reserve Peer-Limit Socket Test

- **Date**: 2026-10-15
- **Issue**: GitHub #387 (GitHub-only, no backlog id) — **Branch**: `test/deflake-peer-limits`
- **Matrix criteria**: PLF1 (new). RSH5 stays as it is and keeps citing the same test.

## 1. Context & Objectives

`crates/daemon/tests/peer_limits_tests.rs::auth_reserve::test_rsh_auth_admitted_on_reserve_while_two_connections_held` failed once during a local `save.sh` run while several cargo builds were running in parallel. The failure was `Capped connection must not receive a payload, got 1 byte(s)`. Run alone, it passed. The goal was to find the root cause, which was either a race in the test's connection timing or a real race in the daemon peer cap. A daemon bug would be fixed in the daemon. A test timing problem would be fixed in the test's synchronisation, without weakening or removing any assertion.

## 2. Architect Design

### Reproduction

Busy loops spread over all 16 CPUs did not reproduce the failure (0/300). A starvation harness did:
- the test binary is pinned to CPU 0 with `taskset -c 0` and runs at `nice 19`;
- four busy loops run on CPU 0 next to it.

Under this harness the test failed 48/100 runs, always at `closed_promptly` (`peer_limits_tests.rs:137`).

### Diagnosis

A temporary diagnostic build (reverted, never committed) printed two values just before `second` connected:
- `dispatcher.available_permits()`;
- the wall time since `held_reserve` connected.

In 40 out of 40 runs, the held reserve had already been admitted (5 free permits). What separated the outcomes was the gap:
- every failure had a gap of at least 203 ms;
- every pass had a gap of at most 192 ms.

### Root cause

The test is at fault; the daemon is correct.

1. The daemon closes an idle auth-reserved connection after `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` = 200 ms. The constant is in `crates/daemon/src/limits.rs`, and `read_and_process` in `crates/daemon/src/dispatcher.rs` enforces it.
2. Under starvation, the test's `sleep(20 ms)` plus the second `connect` overran that window.
3. The held reserve was therefore released, and `second` was legitimately admitted on the freed reserve and answered.
4. In other words, the test's precondition ("while one reserve is held") silently stopped being true.

`PeerConnectionLimiter::try_acquire_auth_reserve` performs a single mutex-guarded check-and-set, so two concurrent reserves for one UID cannot exist.

### Design (round 2)

A new private helper, `connect_back_to_back`, is added in `mod auth_reserve`. It works in two steps:
1. It opens the two connections with `std::os::unix::net::UnixStream::connect`. A Unix-socket `connect` to a listener that has backlog space completes synchronously, with no `.await`.
2. It then registers both streams with `tokio::net::UnixStream::from_std`.

The test replaces the `connect` / `sleep(20 ms)` / `connect` sequence with a call to this helper. Ordering is then guaranteed by the scheduler instead of the wall clock:
- `#[tokio::test]` uses the current-thread runtime, so the server's accept loop cannot run while the test is between the two connects.
- The accept loop takes both connections in one poll.
- It spawns the two handlers in connect order onto the FIFO local run queue.
- The `held_reserve` handler takes the reserve on its first poll, and the `second` handler is refused right after.
- If the held reserve's timer fires during a stall, its wake is queued behind the already-queued `second` handler.

Invariants touched: none. There is no production, PAM, IPC, `unsafe`, logging or Cargo change.

### Rejected design (round 1)

Round 1 proposed `tokio::time::pause()` around the section, with the `test-util` feature for `tokio`. With that patch the test failed 100/100, even on an idle host. In tokio 1.53.1, `runtime/time/mod.rs::park_internal` auto-advances virtual time whenever `!did_wake()`. `did_wake` is set only by an unpark of the driver, never by I/O events dispatched during `park_timeout(0)`. The first idle park therefore jumped straight to the held reserve's 200 ms deadline. The design was dropped and the Cargo change reverted.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`, round 2: **VALIDATION_VERDICT: APPROVED**.

- **R1-MISS (MAJOR, resolved).** The round-1 report approved the paused-clock design based on a wrong reading of `did_wake`. The empirical run caught it, and the evaluator confirmed the miss against the tokio source.
- **FIFO and single-poll accept claims (verified).** The evaluator checked both against the tokio source:
  - `tokio::spawn` from inside a task calls `push_back`, and the scheduler takes tasks with `pop_front`;
  - `UnixListener::accept` clears readiness only on `WouldBlock`, so it takes both connections in one poll.
- **Minor findings:**
  - Changing the test to the multi-thread flavor, or making the accept loop concurrent, would reopen #387. This is documented here and in the test comment.
  - A negligible residual race remains: the accept loop would need two polls, an `event_interval` park in between, and a stall over 200 ms.

## 4. Tester Contract

This change adds no new test and migrates no contract. It is a setup and synchronisation change only:
- every `assert!` / `assert_eq!`, every message, every helper body and every constant in the file is byte-identical;
- the removed lines are the two `UnixStream::connect(..).await.expect(..)` calls and the `sleep(20 ms)`.

Red evidence: the starvation harness failed 48/100 before the fix.

## 5. Auditor Constraints

1. No production code changes. Met: the diff is limited to the test file and traceability documents.
2. No assertion is weakened, removed or relaxed. Met: verified on the diff.
3. The helper must not leak descriptors or block indefinitely. Met:
   - a Unix `connect` on a bound listener returns immediately, or fails with `ECONNREFUSED` / `EAGAIN` and the test fails loudly;
   - both streams are owned by the test and dropped at scope end.
4. The test must still fail against a wrong daemon. Met:
   - with a mutation setting `AUTH_RESERVED_CONNECTIONS_PER_UID = 2` (reverted), the test fails with `Capped connection must not receive a payload`;
   - a daemon that never closes the refused connection fails `closed_promptly` after 500 ms;
   - a permit leak fails `permits_back(6)`.
5. No sensitive data is logged. Met: nothing is logged.

`Clearance: CLEARED`.

## 6. Implementation

- `crates/daemon/tests/peer_limits_tests.rs`:
  - new helper `auth_reserve::connect_back_to_back`;
  - the connect/sleep/connect sequence in `test_rsh_auth_admitted_on_reserve_while_two_connections_held` is replaced by one helper call, with a comment that cites #387 and explains the ordering.
- `AI/VERIFICATION_MATRIX.md`: new component section `deflake-peer-limits` with row PLF1.
- `AI/plan_evaluator_report.md`: round-2 report.

## 7. Candid Review

Recorded in `AI/candid_review_report.md`, which is bound to the diff fingerprint of this branch. An independent reviewer started from a fresh context with `./scripts/candid_subagent.sh --prepare`.

## 8. Verification Results

- **Starvation harness, test binary before the fix:** 48/100 failures.
- **Starvation harness, round-1 paused-clock patch:** 100/100 failures (unloaded runs fail too).
- **Starvation harness, round-2 fix, 4 busy loops:** 0/100 failures.
- **Starvation harness, round-2 fix, 8 busy loops:** 0/300 failures.
- **Mutation `AUTH_RESERVED_CONNECTIONS_PER_UID = 2`:** the test fails as required.
- **`cargo test --locked -p soos-daemon --all-features --test peer_limits_tests -- --test-threads=4`:** 28 passed.
- **`cargo test --locked -p soos-daemon --all-targets --all-features -- --test-threads=4`:** all passed.
- **`cargo fmt --all -- --check`:** clean.
- **`cargo clippy --locked -p soos-daemon --all-targets --all-features -- -D warnings`:** clean.
- **`save.sh` workspace gates:** run before the push.

## 9. Known Limitations / Follow-ups

- The determinism relies on two things:
  - the default current-thread `#[tokio::test]` flavor;
  - the sequential accept loop in `spawn_server`.

  Changing either would reopen #387.
- The other `auth_reserve` socket tests, and the c1/c2 2000 ms `connection_timeout` idle window, still measure wall-clock bounds by design: they check the 200 ms and 500 ms limits themselves. Under extreme starvation they could in principle exceed those bounds. None of them failed in the runs above, and they are out of the scope of #387.

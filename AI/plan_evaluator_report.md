# Plan Evaluation Report — Round 2
- **Date**: 2026-10-15
- **Issue**: GitHub-only #387 — test(daemon): deflake `test_rsh_auth_admitted_on_reserve_while_two_connections_held` (no `AI/BACKLOG.md` entry)
- **Branch**: `test/deflake-peer-limits`
- **Base commit**: `6f32ef8`
- **Plan evaluated**: architect spec for #387, round 2 (session scratchpad), plus the uncommitted working-tree diff of `crates/daemon/tests/peer_limits_tests.rs` (new private helper `connect_back_to_back`, one call site). `crates/daemon/Cargo.toml` is back to the base (no diff).

## 0. Round-1 Retrospective
Round 1 approved a paused Tokio clock (`tokio::time::pause()` plus the `test-util` dev feature). That design failed 100/100, even on an idle host. Round 1's Pillar 2 analysis was wrong. It claimed that I/O readiness collected by `park_timeout(0)` sets `did_wake` and so blocks the auto-advance. Re-reading tokio 1.53.1 shows otherwise:
- `did_wake` is stored only in `runtime/time/handle.rs::Handle::unpark`, which is reached only through `runtime/driver.rs::Handle::unpark`.
- On the current-thread scheduler, a wake issued from the runtime thread never calls `driver.unpark()`:
  - `Schedule::schedule` pushes to the local queue (`core.push_task`) when the scheduler context is set, which is the case inside `park_internal` because it runs under `self.enter(core, ..)`.
  - `wake_by_ref` for the `block_on` future only sets `woken` when called on the runtime thread.
- So every I/O event dispatched during the zero-length park leaves `did_wake` false. `park_thread_timeout` then advances virtual time to the earliest timer, which was the held reserve's 200 ms window.

Round 1 inferred the wake path from the comment and the branch condition and did not follow `did_wake` to its only writer. This is recorded as finding R1-MISS below.

## 1. Coverage Matrix
| Acceptance line / TDD test | Spec element | Status |
|---|---|---|
| #387: find the root cause (race in the test's connection timing or a real race in the daemon peer cap) | "Root cause (evidence-based)": 48/100 under CPU starvation; failures only when the connect gap exceeds 200 ms; limiter check-and-set under one mutex | Covered |
| #387: without weakening any assertion | Change §4; diff verified (section 2 below) | Covered |
| The test still fails against a wrong daemon | "Why the test can still fail against a wrong daemon" (4 scenarios, one mutation-checked with `AUTH_RESERVED_CONNECTIONS_PER_UID = 2`) | Covered |
| Real-clock coverage of the 200 ms window is kept | Untouched `test_rsh_reserve_idle_connection_closed_within_window` and `test_rsh_reserve_stalled_frame_holds_the_reserve_for_the_window` | Covered by existing tests |
| Deflake evidence | "Evidence": 0/100 under the same starvation harness, 12 `auth_reserve` tests green unloaded | Covered (to be reproduced in the walkthrough) |
| Traceability: matrix row PLF1 (prefix unused), walkthrough 210 (reserved by the orchestrator), `Closes #387`, no `scripts/sync_issue.py` entry | "Invariants touched" | Covered |

## 2. Facts Verified Against Code
| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Reserve window 200 ms, enforced by a Tokio timer in `read_and_process` | `crates/daemon/src/limits.rs:57`; `crates/daemon/src/dispatcher.rs:553-570` | `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS = 200`; `timeout(..., read_client_frame(stream))` | Yes |
| Admission is synchronous in the handler's first poll | `crates/daemon/src/dispatcher.rs:426-453` | `get_peer_credentials` (`SO_PEERCRED`), `try_acquire`, `try_acquire_auth_reserve` all run before the first `.await` | Yes |
| Reserve check-and-set is atomic | `crates/daemon/src/limits.rs:262-293` | one `shared.lock()` guard | Yes |
| The server accept loop spawns one handler per accepted stream | `crates/daemon/tests/peer_limits_tests.rs:99-109` | `while let Ok((stream, _)) = listener.accept().await { tokio::spawn(..handle_connection..) }` | Yes |
| `tokio::spawn` from inside a task is FIFO on the local queue | tokio 1.53.1 `runtime/scheduler/current_thread/mod.rs` `schedule` / `push_task` / `next_local_task` | in-context schedule -> `core.tasks.push_back`; `next_task` -> `tasks.pop_front()` (the remote inject queue is empty here) | Yes |
| Accept takes both connections in one poll | tokio 1.53.1 `net/unix/listener.rs::accept` -> `registration().async_io(READABLE, accept)` | readiness is cleared only on `WouldBlock`, so the second accept succeeds immediately; two I/O ops are far below the 128-unit coop budget per poll | Yes |
| Timer expiries are processed only when the scheduler parks or yields to the driver | tokio 1.53.1 `current_thread/mod.rs` `block_on` loop | the driver (timers and I/O) runs in `park` (local queue empty) or `park_yield` (every `event_interval` = 61 ticks); a fired timer wakes `H1` with `push_back`, behind an already-queued `H2` | Yes |
| Std Unix connect completes synchronously with backlog space | `connect_back_to_back` | two blocking `std::os::unix::net::UnixStream::connect` calls with no `.await` between them; the backlog holds 2 | Yes |
| `from_std` requires non-blocking mode and a runtime context | diff | `set_nonblocking(true)` before `UnixStream::from_std`; called inside the `#[tokio::test]` body | Yes |
| Current-thread flavor | test attribute has no `flavor` | default `#[tokio::test]` | Yes |
| `Cargo.toml` reverted | `git diff -- crates/` | only `crates/daemon/tests/peer_limits_tests.rs` changed | Yes |
| Assertions unchanged | diff | Removed: two `.expect("Connect reserve" / "Connect second reserve")` setup unwraps and the 20 ms sleep. Added: helper `.expect` setup unwraps. Every `assert!`, `assert_eq!`, `closed_promptly` / `permits_back` / `status_roundtrip` call, message, helper body and constant is byte-identical. | Yes |

## 3. Pillar Analysis
### Pillar 1 — Architecture & threat model
- Failure scenario considered: the deflake masks a real daemon race that admits two auth-reserved permits for one UID.
- Result: PASS. The limiter is a single mutex-guarded check-and-set. The mutation run with `AUTH_RESERVED_CONNECTIONS_PER_UID = 2` makes the test fail with the original message, which proves the assertion still detects a double reserve. No production code, socket mode or `SO_PEERCRED` path changes.

### Pillar 2 — PAM deadline & concurrency (test synchronisation)
- Failure scenario considered: the host suspends the test thread for more than 200 ms after `H1` (the `held_reserve` handler) registers its window and before `H2` (the `second` handler) is polled.
- Result: PASS. Both streams are already in the listener backlog before the test body first yields. The accept task takes both in one poll and pushes `H1` then `H2` onto the FIFO local queue. `H1` takes the reserve in its first poll. Even if its 200 ms timer expires during a stall, the expiry is only observed at the next park or `park_yield`, and the woken `H1` is queued behind `H2`. `H2` is therefore always admitted while the reserve is held, independently of wall time.
- Failure scenario considered: the accept task needs two polls (coop exhaustion or no cached readiness), and an `event_interval` boundary plus a 200 ms stall falls between `H1`'s first poll and the spawn of `H2`.
- Result: residual, negligible (MINOR-2). Cached readiness and a 128-unit budget make a two-poll accept effectively impossible for two connections.
- Failure scenario considered: c1/c2 regular connections time out (2000 ms `connection_timeout`) before the final `status_roundtrip`.
- Result: unchanged from the base (pre-existing wall-clock dependency, acknowledged in "Residual dependencies"); the new section shortens the path, removing the 20 ms sleep.
- No PAM code, no Tokio in `crates/pam`.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: the helper panics or blocks indefinitely (std `connect` blocks if the backlog is full; `from_std` panics without a reactor).
- Result: PASS. The backlog has room for 2. The call is inside the runtime. Failures panic loudly (setup `expect`, allowed by the file-level lint allowance) and never pass the test. No production error path changes.

### Pillar 4 — Dependencies
- Failure scenario considered: a manifest or feature change trips an invariant or `deny.toml`.
- Result: PASS. No manifest change remains. No invariant in `tests/invariants/src` references `peer_limits_tests`.

### Pillar 5 — Data confidentiality
- Result: PASS. Test-only change; no logging, credentials, embeddings or frames.

### Pillar 6 — Test integrity
- Failure scenario considered: the precondition "one reserve is held" is no longer explicitly checked. Round 1's `permits_back(&dispatcher, 5)` is gone, so a daemon that refuses `held_reserve` might go unnoticed.
- Result: PASS. If `held_reserve` were refused, `second` would find the reserve free, be admitted and answered, and `closed_promptly` would panic. A daemon that never closes a refused connection fails the 500 ms `closed_promptly` assertion. A leaked permit fails `permits_back(6)`. An explicit `available_permits() == 5` check after `closed_promptly(second)` would itself be wall-clock dependent (the 200 ms window may legitimately expire by then), so not adding one is correct.

## 4. Findings
- **[MAJOR, round 1, resolved] R1-MISS**: the round-1 evaluation approved a paused-clock design on a misreading of tokio's auto-advance guard. It assumed I/O dispatched during `park_timeout(0)` sets `did_wake`; in tokio 1.53.1 only an off-thread `driver.unpark()` sets it. The design failed 100/100. Round 2 drops it entirely (no `test-util`, no `pause`). Process lesson for future evaluations: when a plan relies on runtime internals, trace each flag to its writers rather than reading the guarding comment, and require an empirical run before approval.
- **[MINOR-1]** The ordering guarantee depends on the default current-thread flavor and on the server accept loop spawning handlers in accept order. The in-test comment states the current-thread reliance; the walkthrough should also state that converting this test to `flavor = "multi_thread"` or changing `spawn_server` to accept concurrently would reopen #387.
- **[MINOR-2]** Residual: if the accept task ever needed two polls (cooperative budget exhaustion), an `event_interval` park could, together with a stall of more than 200 ms, queue `H1`'s timeout before `H2` is spawned. With two connections and cached readiness this is not reachable in practice; mention it only in the walkthrough.
- **[MINOR-3]** The helper's `expect` messages are generic ("Connect first" / "Connect second") where the base said "Connect reserve" / "Connect second reserve". These are setup unwraps, not assertions, so this is not a weakening. Optional: keep the old wording for continuity.
- **[MINOR-4]** Record the evidence in the walkthrough and matrix row PLF1:
  - 48/100 failures before, 0/100 after under the starvation harness;
  - the round-1 failure (100/100);
  - the mutation run.

## 5. Verdict
Round 2 has no open CRITICAL or MAJOR finding (R1-MISS is fixed by dropping the round-1 design). The root cause is consistent with the code. The FIFO ordering argument checks out against the tokio 1.53.1 current-thread scheduler and listener. No assertion is changed or weakened, and no manifest or invariant is touched.

VALIDATION_VERDICT: APPROVED

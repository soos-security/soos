# Walkthrough 156 — Mock Daemon Socket Race, Expired-Window Margin and Store Poll Test

- **Date**: 2026-10-01
- **Issue**: GitHub #293 ("Tests" items; no backlog id) — **Branch**: `test/p3fu4-flaky-and-gui-poll`
- **Matrix criteria**: FGP1–FGP3 (component `mock-daemon-flake-and-store-poll-test`)

---

## 1. Context & Objectives

#293 lists the residual follow-ups of #291. This batch takes its two "Tests" items:

| # | Item | State on `main` (`ccf37c1`) |
|---|---|---|
| 1 | `distro_matrix::test_mock_daemon_malformed_modes_put_one_defect_on_the_wire` | Failed once in a full `soos-invariants` run during the #291 candid review, passed in reruns; cause unknown |
| 2 | `StoreTaskRunner::poll` lost-outcome branch (`crates/gui/src/store_tasks.rs`) | Only `PendingTask::failed` and the panic path were tested; `poll` itself never ran the branch |

The "Hardening (optional)" and hardware items of #293 are out of scope.

## 2. Architect Design (Phase 1)

**Item 1 — reproduction first.** The test was run 200 times (then 400 more) under CPU load: 16
busy loops on a 16-CPU host with 4 test processes in parallel. Two failure signatures came out:

```text
panicked at tests/invariants/src/distro_matrix.rs:1158:53:
connect mock: Os { code: 111, kind: ConnectionRefused, message: "Connection refused" }

panicked at tests/invariants/src/distro_matrix.rs:1324:5:
expired must send a window that closed before the request
```

The stressed full suite showed the same two signatures in the sibling test
`pam_response_expiry_contract::test_pre_mock_daemon_stamps_from_the_monotonic_clock`:

```text
panicked at tests/invariants/src/pam_response_expiry_contract.rs:130:53:
connect mock: Os { code: 111, kind: ConnectionRefused, message: "Connection refused" }

panicked at tests/invariants/src/pam_response_expiry_contract.rs:196:5:
expired mode must send a window that closed before the request (17201832994967 >= 17201831072210)
```

Root causes, both in `tests/docker/mock_daemon.py`:

1. **Bound-but-not-listening socket.** The mock called `bind(sock_path)`, then `chown`, `chmod`
   and finally `listen`. The socket path exists from `bind` on, and every harness (the two above,
   `review_followups_contract`, the Docker scripts) waits for the *path* before it connects. A
   connect that lands between `bind` and `listen` is refused. Under load the Python process is
   descheduled in that window often enough to fail about 1 run in 100.
2. **Expired window too close to "now".** `--mode expired` stamps `expires = now - 1 s`, read at
   send time. The test reads `before` (one `python3` process) before it spawns the mock, so the
   gap between `before` and the send covers the mock's interpreter start-up and the socket poll.
   Under load that gap exceeded 1 s (2.9 s in the captured message), so `expires >= before`.

The stamp and one-defect assertions are correct; only the mock's synchronisation and margin were
wrong. Design:

- The mock binds a private staging name (`<socket dir>/.mock-<pid>`, removed first if it exists)
  under the same `0o117` umask, applies the group and mode, listens, and only then
  `os.rename`s it to `--socket`. A Unix socket keeps its listening inode across a rename, so the
  path appears atomically in a ready state. `cleanup` removes both names.
- `EXPIRED_AGE_NS` becomes 60 s, above the 10 s socket wait plus the 10 s read timeout of the
  harnesses. The `max(..., 2)` / `max(..., 1)` clamps keep a consistent, non-zero window on a host
  whose CLOCK_MONOTONIC is still below 60 s.

**Item 2.** The unit-test module `worker_failure_tests` is a child of `store_tasks`, so it can
install an in-flight worker directly (private fields `tx`, `in_flight`): a thread that drops a
clone of the outcome sender without sending. No production seam or API change is needed.

## 3. Plan Evaluation

Condensed with the spec (test-only batch). Checked: no assertion of any existing test changes;
the mock still refuses "other" permission bits and is never briefly world-accessible (the staging
name is bound under the same umask and receives the mode before it is published); production code
of `soos-gui` is untouched.

## 4. Tester Contract (Phase 2)

| Test (path::name) | Matrix | Red evidence |
|---|---|---|
| `tests/invariants/src/distro_matrix.rs::test_mock_daemon_publishes_its_socket_only_once_listening` | FGP1, FGP2 | Against the `main` mock: "mock_daemon.py must never bind the published socket path directly" |
| `crates/gui/src/store_tasks.rs::worker_failure_tests::test_fgp_poll_reports_a_lost_delete_worker_as_one_failure` | FGP3 | Mutation (synthesized outcome removed from `poll`): "the runner stays busy until it reports" |
| `crates/gui/src/store_tasks.rs::worker_failure_tests::test_fgp_poll_reports_a_lost_enroll_worker_as_one_failure` | FGP3 | Same mutation, same message |

The `poll` branch already exists on `main`, so the new GUI tests are green against it; red was
shown by temporarily removing `outcomes.push(pending.failed(WORKER_LOST_MESSAGE))` (restored before
the commit). The flaky test itself is the contract for item 1, proven by the stress loop (§8).

### Migrated existing tests

None. No existing test file was edited except by appending new tests: `distro_matrix.rs` gains one
test function, `store_tasks.rs` gains two helpers and two tests inside `worker_failure_tests`.

### Flakiness check

`test_fgp_` tests run 10 times in a row: 10/10 green.

## 5. Auditor Constraints (Phase 3)

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No existing assertion modified, weakened or deleted | all test files | `git diff` (additions only in test files) |
| 2 | The mock socket is never accessible to "other", including the staging name | `mock_daemon.py` | `distro_matrix::test_mock_daemon_socket_is_group_restricted` |
| 3 | No production code change, no `unwrap`/`expect`/`panic` outside `#[cfg(test)]` | `crates/gui/src/store_tasks.rs` | diff, clippy `-D warnings` |
| 4 | Every harness wait stays bounded | mock + harnesses | unchanged 10 s socket wait and 10 s read timeout; new GUI poll bounded at 5 s |

Clearance: CLEARED.

## 6. Implementation (Phase 4)

- `tests/docker/mock_daemon.py`: staging-name bind, `listen`, then atomic `os.rename` to the
  published path; `cleanup` removes both names; `EXPIRED_AGE_NS = 60_000_000_000` with its rationale.
- `tests/invariants/src/distro_matrix.rs`: `test_mock_daemon_publishes_its_socket_only_once_listening`.
- `crates/gui/src/store_tasks.rs`: `runner_with_lost_worker`, `poll_until_outcome` and the two
  `test_fgp_` tests in `worker_failure_tests`.
- `Docs/PAM_DOCKER_TEST_MATRIX.md`: the socket publication order and the 60 s expired window.
- `AI/VERIFICATION_MATRIX.md`: component `mock-daemon-flake-and-store-poll-test` (FGP1–FGP3).

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) green. The independent layer 2 review runs at release time
(not part of this batch).

## 8. Verification Results

Stress loop (`--exact distro_matrix::test_mock_daemon_malformed_modes_put_one_defect_on_the_wire`,
16 busy loops, 4 runs in parallel; the test binary reads the mock at run time, so the same binary
was used before and after):

| Mock | Runs | Failures |
|---|---|---|
| `main` | 200 + 400 | 2 + 4 (5 `ConnectionRefused`, 1 `expired` window) |
| fixed | 400 + 200 | 0 + 0 |

Full `soos-invariants` binary, 40 runs, 2 in parallel, 16 busy loops: `main` mock 2 failures
(both in `test_pre_mock_daemon_stamps_from_the_monotonic_clock`, one per signature), fixed mock 0.

```bash
cargo test --locked --all-features -p soos-invariants -p soos-gui   # all green
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -p soos-invariants -p soos-gui -- -D warnings
python3 -m py_compile tests/docker/mock_daemon.py
./scripts/candid_review.sh
```

## 9. Known Limitations / Follow-ups

- The Docker PAM matrix (`./run_tests.sh`) was not run in this batch; the mock change is
  behaviour-preserving for it (same modes, same socket mode and group) and removes the same race
  for its socket waits.
- #293 "Hardening (optional)" and hardware items remain open.

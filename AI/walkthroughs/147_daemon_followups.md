# Walkthrough 147 — Daemon Follow-ups (GitHub #287)

- **Date**: 2026-10-01
- **Issue**: GitHub #287 "Daemon" section (no backlog id) — **Branch**: `fix/p3fu-daemon`
- **Matrix criteria**: DFU1–DFU6 (component `daemon-followups`)
- **ADR**: 2026-10-01 "Debug-Only Panic Messages, Accept Backoff and Tracked Evidence Writes"

## 1. Context & Objectives

GitHub #287 collects the non-blocking findings left after the P3 batch reviews. This branch
handles six items of its "Daemon" section, following walkthroughs 135 (logging, stamping,
bounded shutdown drain) and 140 (systemd readiness, `memory_locked`):

1. Panic hook: log the panic message in debug builds only (owner decision), release unchanged.
2. `accept()` errors such as `EMFILE` loop without backoff.
3. Spoof evidence is written detached; a write in flight at shutdown is lost.
4. `packaging/soos-daemon.service`: a start that always times out takes about 62 s, more than
   `StartLimitIntervalSec=60`, so `StartLimitBurst=5` is never reached.
5. Show `memory_locked` in `soos-admin status`.
6. `test_clock_failure_yields_expired_non_allow_response` does not reach the downgrade branch.

Out of scope: measuring `TimeoutStartSec` on the slowest target CPU (owner hardware check).

## 2. Architect Design

- `crates/daemon/src/shutdown.rs`
  - `PanicMessagePolicy { Withhold, LogMessage }` with `for_debug_assertions(bool)` and
    `for_build()` (`cfg!(debug_assertions)`); `install_panic_hook_with(policy)`;
    `install_panic_hook()` stays the payload-free release hook (now `with(Withhold)`).
    `MAX_DEBUG_PANIC_MESSAGE_CHARS` = 512 bounds the debug message.
  - `AcceptBackoff` (`new(initial, max)`, `on_error() -> Duration`, `on_success()`,
    `Default`), `ACCEPT_BACKOFF_INITIAL` = 5 ms, `ACCEPT_BACKOFF_MAX` = 1 s; the initial delay is
    floored at 1 ms and `max` raised to `initial`, so a delay is never zero.
  - `accept_with_backoff(accept, dispatcher, tasks, backoff, shutdown)`: the accept loop with an
    injectable `FnMut() -> Future<Output = io::Result<UnixStream>>` seam;
    `accept_until_shutdown` keeps its signature and delegates to it.
  - `BlockingTasks` (`spawn_blocking`, `in_flight`, `drain(budget) -> DrainReport`): a
    `std::sync::Mutex<JoinSet<()>>` of blocking jobs, reaped on every spawn.
- `crates/daemon/src/dispatcher.rs`: `ConnectionDispatcher::evidence_writes()`; the pure
  `stamp_response(request_id, verdict, reason_class, clock_reading) -> Response` now holds the
  stamping and downgrade logic that `build_response` used inline.
- `crates/daemon/src/main.rs`: hook chosen by `PanicMessagePolicy::for_build()`; evidence writes
  drained after the connection drain with `drain_budget.saturating_sub(elapsed)`.
- `crates/admin-cli/src/status.rs`: `DaemonStatusReport::memory_locked: Option<bool>`, shown as
  "Swap Protection" (`LOCKED`, `NOT LOCKED (memory may be swapped out)`, `N/A`).
- Invariants touched: fail-closed stamping (unchanged behaviour), bounded shutdown (total bound
  unchanged: one `connection_timeout`), no sensitive data in release logs.

## 3. Plan Evaluation

Condensed into this single-agent run (no separate plan-evaluator report). Two conflicts with
existing contracts were found while checking the plan against the tests:

- `graceful_shutdown_tests::test_production_main_wires_tracked_handlers_and_bounded_drain`
  requires `install_panic_hook();` in `main.rs`, and `panic_hook_tests` (built with debug
  assertions) requires `install_panic_hook()` to withhold the message. Resolution: the release
  hook keeps its name and behaviour; `main.rs` calls it on the `Withhold` branch and
  `install_panic_hook_with` on the debug branch. Both tests pass unchanged.
- Item 4 (start limit): `installer_templates_contract::test_daemon_unit_requires_deployed_models_and_bounds_restarts`
  asserts the literal line `StartLimitIntervalSec=60`. Setting it to 320 breaks that assertion,
  and existing assertions could not be modified without approval. Lowering `TimeoutStartSec` to
  fit 60 s instead (10 s + 2 s) would change start behaviour on slow hardware, which is the
  owner's out-of-scope check. Item 4 was first held back (DFU6 Pending). **The owner approved the
  assertion change on 2026-10-01** (only that literal, 60 → 320), and item 4 was then delivered.

## 4. Tester Contract

| Test (path::name) | Matrix | Red evidence |
|---|---|---|
| `panic_hook_build_policy_tests::test_panic_message_policy_follows_debug_assertions` | DFU1 | E0432 unresolved `PanicMessagePolicy`, `MAX_DEBUG_PANIC_MESSAGE_CHARS` |
| `panic_hook_build_policy_tests::test_debug_policy_logs_message_and_release_policy_withholds_it` | DFU1 | E0432 unresolved `install_panic_hook_with` |
| `panic_hook_build_policy_tests::test_production_main_selects_panic_policy_from_build_profile` | DFU1 | same compile failure (file-level) |
| `daemon_followups_tests::test_accept_backoff_doubles_from_initial_and_caps_at_max`, `test_accept_backoff_defaults_and_degenerate_bounds` | DFU2 | E0432 unresolved `AcceptBackoff`, `ACCEPT_BACKOFF_INITIAL`, `ACCEPT_BACKOFF_MAX` |
| `daemon_followups_tests::test_accept_errors_back_off_without_stopping_the_loop`, `test_shutdown_interrupts_accept_backoff_promptly`, `test_accept_until_shutdown_still_serves_a_real_listener` | DFU2 | E0432 unresolved `accept_with_backoff` |
| `daemon_followups_tests::test_blocking_tasks_drain_waits_for_in_flight_write`, `test_blocking_tasks_drain_is_bounded_by_budget`, `test_blocking_tasks_reap_finished_jobs_on_spawn`, `test_blocking_task_panic_is_counted_not_propagated` | DFU3 | E0432 unresolved `BlockingTasks` |
| `daemon_followups_tests::test_spoof_evidence_write_is_tracked_and_drained_at_shutdown` | DFU3 | E0599 no method `evidence_writes` |
| `daemon_followups_tests::test_production_main_drains_evidence_writes_within_the_shutdown_budget` | DFU3 | compile failure (file-level); asserts no `drop(tokio::task::spawn_blocking` remains |
| `daemon_followups_tests::test_clock_failure_downgrades_allow_to_unavailable_internal_error`, `test_zero_clock_reading_downgrades_allow`, `test_clock_failure_never_yields_allow_for_any_verdict`, `test_working_clock_keeps_verdict_and_stamps_validity_window`, `test_build_response_renders_through_stamp_response` | DFU4 | E0432 unresolved `stamp_response` |
| `status_memory_locked_tests::test_status_reports_memory_locked_from_daemon`, `test_status_reports_memory_unlocked_from_daemon`, `test_offline_status_never_claims_memory_locked` | DFU5 | E0609 no field `memory_locked` on `DaemonStatusReport` |
| `installer_templates_contract::test_daemon_unit_start_limit_interval_covers_burst_of_timed_out_starts` | DFU6 | assertion failure on the unchanged unit: `StartLimitIntervalSec=60 must be >= StartLimitBurst * (TimeoutStartSec + RestartSec) = 5 * (60 + 2) = 310` |

Every Red failure was on exactly the specified API or the asserted unit value.

### Migrated existing tests

Setup-only edits allowed for item 5 (a new struct field makes the existing literals fail to
compile), and one owner-approved literal migration for item 4:

| File | Edit |
|---|---|
| `crates/admin-cli/tests/status_tests.rs` (`test_status_report_json_serialization`) | added `memory_locked: Some(true),` to the `DaemonStatusReport` literal |
| `crates/admin-cli/tests/cli_deadline_json_tests.rs` (`test_status_report_json_escapes_socket_path_and_unit`) | added `memory_locked: None,` to the `DaemonStatusReport` literal |
| `tests/invariants/src/installer_templates_contract.rs` (`test_daemon_unit_requires_deployed_models_and_bounds_restarts`) | literal `StartLimitIntervalSec=60` → `StartLimitIntervalSec=320`, owner approval 2026-10-01; nothing else in the test changed (its message still reads "5 in 60 s") |

`test_clock_failure_yields_expired_non_allow_response` is untouched; the new DFU4 tests are added
next to it in a separate file.

### Flakiness check

`daemon_followups_tests` and `panic_hook_build_policy_tests` run 10 times in a row: 10/10 green.

## 5. Auditor Constraints

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No `unwrap`/`expect`/`panic!`/indexing in production code; mutex poisoning handled with `PoisonError::into_inner` | `shutdown.rs`, `dispatcher.rs`, `status.rs` | workspace clippy lints `-D warnings` |
| 2 | Release builds never log the panic message; the debug message is bounded (512 chars) | `install_panic_hook_with`, `main.rs` | DFU1 tests, pre-existing `panic_hook_tests` |
| 3 | A backoff delay is never zero (no hot loop) and never exceeds 1 s; no arithmetic overflow (`saturating_mul`) | `AcceptBackoff` | `test_accept_backoff_defaults_and_degenerate_bounds` |
| 4 | Shutdown is never delayed by a backoff; the accept loop never exits on an error | `accept_with_backoff` | `test_shutdown_interrupts_accept_backoff_promptly`, `test_accept_errors_back_off_without_stopping_the_loop` |
| 5 | The total shutdown wait stays one `connection_timeout`; evidence writes use only the remainder | `main.rs` | `test_production_main_drains_evidence_writes_within_the_shutdown_budget` |
| 6 | The tracked set is bounded (reaped on spawn); no lock is held across `.await` | `BlockingTasks` | `test_blocking_tasks_reap_finished_jobs_on_spawn`; review of `drain` (`mem::take` before awaiting) |
| 7 | A job panic is counted and logged without its payload (the `JoinError` is never formatted) | `classify_blocking` | `test_blocking_task_panic_is_counted_not_propagated`; logging audit test |
| 8 | Fail-closed stamping unchanged: no clock means no `Allow` | `stamp_response` | DFU4 tests, pre-existing `response_timestamp_tests` |
| 9 | An unreachable daemon never reports memory as locked | `query_status` | `test_offline_status_never_claims_memory_locked` |
| 10 | No new log field carries frames, embeddings, nonces or credentials | new `error!`/`info!`/`warn!` calls | `logging_audit_test`, `request_id_logging_tests` |

No new dependency (`tokio::task::JoinSet` is already in use). Clearance: CLEARED.

## 6. Implementation

- `crates/daemon/src/shutdown.rs`: `PanicMessagePolicy`, `install_panic_hook_with`,
  `AcceptBackoff`, `accept_with_backoff` (backoff sleep raced against shutdown and the handler
  reaper), `BlockingTasks`, `classify_blocking`.
- `crates/daemon/src/dispatcher.rs`: `evidence_writes` field and accessor; `capture_spoof_evidence`
  spawns through it instead of `drop(tokio::task::spawn_blocking(..))`; `stamp_response`
  extracted from `build_response`.
- `crates/daemon/src/main.rs`: build-dependent hook; `Arc::clone(&dispatcher)` passed to the
  accept loop so the dispatcher stays available for the evidence drain.
- `crates/admin-cli/src/status.rs`: `memory_locked` field, "Swap Protection" table line.
- `packaging/soos-daemon.service`: `StartLimitIntervalSec=320` (5 × (60 + 2) = 310 ≤ 320) with
  the comment explaining the bound; `Docs/PACKAGING_AND_PROVISIONING.md` updated; matrix IDT6
  criterion text updated to 320 s.
- Note: a running blocking write cannot be cancelled; a write past the budget is detached and
  reported as abandoned, so the bound holds even with a stuck filesystem.

## 7. Candid Review

Not run on this branch: the sub-agent review is run later on the combined #287 batch. Layer 1
(`./scripts/candid_review.sh`) was run, see §8.

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=4
cargo fmt --all -- --check                                                   # clean
cargo clippy --locked --all-targets --all-features -p soos-daemon -p soos-admin-cli -p soos-invariants -- -D warnings   # clean
cargo test --locked --all-features -p soos-daemon -p soos-admin-cli -p soos-invariants  # 763 passed, 0 failed
./scripts/candid_review.sh                                                   # layer 1 passed
```

## 9. Known Limitations / Follow-ups

- Item 4 (DFU6) was delivered after the owner approved the literal migration on 2026-10-01.
  The failure message of `test_daemon_unit_requires_deployed_models_and_bounds_restarts` still
  says "5 in 60 s" because only the literal was allowed to change; it can be reworded later.
  A real-systemd check of the start-limit state still needs a systemd container.
- The `PasswordFailed` event snapshot is still written inline on the event path (not detached,
  so not lost at shutdown); it blocks that connection task during the write, as before.
- Debug builds now log panic messages, which may contain request data; debug builds are
  developer-only and never packaged.

# Tester Contract — GitHub #329 (install readiness race, foreign-owned build directory, presence wake settle)

- **Branch**: `fix/install-readiness-and-presence-warmup` (base `origin/main` `b16f567`)
- **Spec**: `AI/architect_spec_install_presence_warmup.md` revision 3 (`AI/plan_evaluator_report.md`, round 3 APPROVED)
- **Matrix**: new component `install-readiness-presence-warmup`, rows IWP1–IWP13 (spec §6)
- **Tests added**: 20 (11 invariants, 5 daemon worker integration, 4 daemon consensus integration); no existing test or fixture changed
- **Red state**: 17 tests red on the base (assertion, or compile error on exactly the specified API); 3 pass on purpose (characterisation / no-false-positive tests whose power is stated below)

Run (CI command form):
`cargo test --locked -p soos-invariants --all-features install_presence_warmup`,
`cargo test --locked -p soos-daemon --all-features --test presence_wake_settle_tests`,
`cargo test --locked -p soos-daemon --all-features --test presence_wake_settle_consensus_tests`.

## Contract

### `tests/invariants/src/install_presence_warmup_contract.rs` (compiles on the base; assertion red)

| Test | Matrix | Red evidence (base `b16f567`) |
|---|---|---|
| `test_iwp_readiness_polls_until_status_is_healthy` | IWP1 | `a daemon that turns healthy on the third status query is ready (attempts: 1); output: {"attempt":1,"is_healthy":false}` |
| `test_iwp_readiness_prints_only_the_final_status` | IWP2 | `the healthy report is printed; output: {"attempt":1,"is_healthy":false}` |
| `test_iwp_readiness_times_out_after_polling_with_last_status` | IWP2 | `the status must be polled during the whole --timeout (only 1 attempt(s))` |
| `test_iwp_readiness_attempt_is_bounded` | IWP3 | `left: Some(0), right: Some(1)` after 30 s (the hanging status ran unbounded and `{"partial":` was printed) |
| `test_iwp_install_keeps_exit_70_on_readiness_failure` | IWP4 | passes on the base (characterisation: exit 70 and the helper's exit-code contract must survive the change) |
| `test_iwp_build_refuses_foreign_owned_target_dir` | IWP5 | `the failure must print the exact fix 'sudo chown -R nobody: /tmp/soos_iwp_foreign_target_<pid>'` (the base ran `runuser`, then failed on missing artifacts) |
| `test_iwp_dry_run_build_reports_foreign_owned_target_dir` | IWP5 | `left: Some(0), right: Some(2)` (the dry run passed its preflight) |
| `test_iwp_build_refuses_non_writable_target_dir` | IWP6 | `the failure must print the exact fix 'chmod -R u+w /tmp/soos_iwp_ro_target_<pid>'` |
| `test_iwp_build_proceeds_with_an_owned_target_dir` | IWP7 | passes on the base (no-false-positive test); killed by a check that flags any existing target directory or ignores the build user |
| `test_iwp_settle_is_presence_only` | IWP11 | `presence/mod.rs must define PRESENCE_WAKE_SETTLE_MS = 1000 (single source)` |
| `test_iwp_docs_and_adr_describe_the_changes` | IWP13 | ``Docs/PACKAGING_AND_PROVISIONING.md must document `sudo chown -R` `` |

### `crates/daemon/tests/presence_wake_settle_tests.rs` (compiles on the base; assertion red)

| Test | Matrix | Red evidence (base `b16f567`) |
|---|---|---|
| `test_iwp_woken_scan_skips_captures_before_the_settle` | IWP8 | `no PAD evaluation may happen before the 1000 ms settle after a wake (first PAD call seen 8 ms after the wake)` |
| `test_iwp_woken_scan_unlocks_after_spoof_looking_wake_frames` | IWP8 | `left: Scanned(SpoofVetoed), right: Scanned(Unlocked { session: SessionId("2"), uid: 1000 })` — the 2026-10-03 trace reproduced |
| `test_iwp_spoof_after_the_settle_still_vetoes` | IWP9 | `the vetoing capture is a settled one (first PAD call 8 ms after the wake)` (the veto itself already holds on the base and must keep holding) |
| `test_iwp_constant_spoof_never_unlocks_a_woken_scan` | IWP9 | `the woken scan evaluates only settled captures (first PAD call 8 ms)` |
| `test_iwp_scan_of_a_streaming_camera_has_no_settle` | IWP10 | passes on the base (no settle exists); killed by an implementation that applies the settle to every scan |

### `crates/daemon/tests/presence_wake_settle_consensus_tests.rs` (compile red on exactly the specified API)

| Test | Matrix | Red evidence (base `b16f567`) |
|---|---|---|
| `test_iwp_settle_constant_value` | IWP11 | `E0432 unresolved import soos_daemon::presence::PRESENCE_WAKE_SETTLE_MS` |
| `test_iwp_not_before_zero_is_the_pam_consensus` | IWP11 | `E0599 no method named not_before_ns / with_not_before found for struct RequestDeadline` |
| `test_iwp_not_before_skips_earlier_captures` | IWP12 | same compile error |
| `test_iwp_not_before_past_the_deadline_fails_closed` | IWP12 | same compile error |

Existing contracts that already cover the remaining rows and must stay green: `installer_templates_contract::test_wait_daemon_ready_succeeds_when_socket_and_status_answer`, `installer_templates_contract::test_wait_daemon_ready_fails_closed_and_bounded` (IWP4), `packaging_ownership_contract::test_install_build_as_root_drops_to_invoking_user_or_refuses`, `arch_faillock_ci_contract::test_install_build_exports_the_prefix_bindir` (IWP7, frozen strings of spec §4.3), `presence_unlock_contract::test_pau_pad_consensus_is_built_only_in_consensus_rs` (both callers keep `run_face_consensus(`), `warmup_default_tests::test_daemon_default_warmup_frames_constant_is_zero` (IWP11), the whole `presence_worker_tests` / `presence_consensus_tests` suites (camera ready at scan start ⇒ no settle).

## Test design notes for the developer

- **Woken camera**: `MockCameraManager::set_starved(true)` before the scan (re-applied after 100 ms to clear a frame the capture thread may race in), and the spy camera's `on_wake` hook — run inside `notify_activity` — calls `set_starved(false)`. The worker must sample `camera.is_ready()` **before** `notify_activity()` (spec §5.3); sampling after it always sees a ready mock camera.
- **PAD observer**: a thread started in the `on_wake` hook records the first instant `MockPadDetector::call_count()` rises above its value at the wake (`build_pipeline` already evaluated one frame). Observation can only be late, never early, so `>= 1000 ms` cannot fail spuriously on a correct implementation; IWP10's `< 1000 ms` is a wall-clock bound above the 900 ms product budget.
- **Clock domain**: `SpyCamera` restamps every capture with the test clock (`CLOCK_MONOTONIC + offset`), the same clock as the worker; `not_before_ns` must be computed from the worker's `clock_fn`, never `Instant` or a raw `CLOCK_MONOTONIC` read.
- **Build check**: the foreign-owner tests run `install.sh` under `root_shims` (`id -u` → 0, logging `runuser`/`cargo`) with `SUDO_USER=nobody`; they skip when `nobody` does not exist or is the runner. IWP6 and IWP7 skip on a root runner. The exact strings asserted are `sudo chown -R nobody: <target>`, `not owned by the build user 'nobody'`, `nothing was modified`, `chmod -R u+w <target>` (the scratch paths contain no shell-special character, so `printf %q` prints them verbatim).
- **Readiness**: the stderr text `did not report healthy within <T> s` and `journalctl` are asserted; `IWP3` needs coreutils `timeout` on `PATH` (skips otherwise) and a hung attempt's partial stdout must not be printed.
- **Accepted residual (plan-evaluator P5)**: IWP12 restamps captures at read time, so it cannot tell a timestamp filter from "sleep until the bound, then evaluate"; the worker tests (IWP8/IWP9) prove the observable contract (no PAD call before the settle).

### Migrated existing tests

None. No existing assertion, test or shared fixture (`crates/daemon/tests/common/mod.rs`) was modified.

### Validation against a throwaway reference

A reference implementation of spec revision 3 (readiness poll, `check_build_target_dir`, `RequestDeadline::with_not_before`, consensus filter, worker settle, doc/ADR stubs) was written in a scratch copy of the worktree only (never in the repository). Against it:
- the 20 new tests pass;
- the complete `soos-daemon` and `soos-invariants` suites (`--all-targets --all-features`) pass, including `presence_unlock_contract` (this run found plan finding P6, fixed by spec revision 3 before the tests were finalised);
- `cargo clippy --locked -p soos-daemon -p soos-invariants --all-targets --all-features -- -D warnings` is clean and `cargo fmt --all -- --check` passes.

In the worktree: `cargo fmt --all -- --check` passes; `cargo clippy -p soos-invariants --all-targets --all-features -- -D warnings` and `cargo clippy -p soos-daemon --all-features --test presence_wake_settle_tests -- -D warnings` are clean (the consensus test file is compile-red by design until the API exists).

### Flakiness check

Against the reference implementation, 10 consecutive runs each, all green:
- `cargo test --locked -p soos-daemon --all-features --test presence_wake_settle_tests --test presence_wake_settle_consensus_tests` — 10/10;
- `cargo test --locked -p soos-invariants --all-features install_presence_warmup` — 10/10.

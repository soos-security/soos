# Walkthrough 153 — Biometric Store Lock, Never-Create Migration Open and Bounded Daemon Exit

- **Date**: 2026-10-01
- **Issue**: GitHub #289 ("[P3-FU2] Non-blocking follow-ups from the #287 batch", storage and
  daemon items) — **Branch**: `fix/p3fu2-storage-daemon`
- **Matrix criteria**: MLS1–MLS8 (new component `migration-lock-and-shutdown-bound`)
- **ADRs**: 2026-10-01 "Biometric Store Lock and Migration Identity Check", 2026-10-01 "Daemon
  Exit Bounded by the Shutdown Drain Budget"
- **Context**: walkthrough 147 (daemon follow-ups: tracked evidence writes, bounded drain),
  walkthrough 150 (operator-run legacy storage migration)

---

## 1. Context & Objectives

Four items of GitHub #289:

1. `BiometricStore::migrate_template` read a legacy template, then rewrote it through `enroll()`
   with no check in between. A concurrent `delete` of the same UID could be undone (the deleted
   template resurrected) and a concurrent re-enrollment overwritten with the older legacy
   content. `soos-enroll migrate` was also not locked against `enroll` / `import` (documented
   only).
2. `open_template_store_checked` (`soos-enroll migrate`) checked that the biometrics directory
   exists, then called `BiometricStore::new`, which creates a missing directory: a directory
   removed in between would be recreated, against "migrate never creates anything".
3. `soos-daemon` reported evidence writes still running at the end of the drain budget as
   "abandoned", but `#[tokio::main]` drops the runtime at the end of `main`, and a runtime drop
   waits without bound for every `spawn_blocking` job: the drain budget did not bound exit.
4. The failure message of
   `installer_templates_contract::test_daemon_unit_requires_deployed_models_and_bounds_restarts`
   still said "5 in 60 s" while the asserted condition is `StartLimitIntervalSec=320`.

Out of scope (other agents): the `daemon.toml` readers in `crates/enrollment-cli/src/service.rs`
and `crates/admin-cli/src/daemon_config.rs`.

## 2. Phase 1 — Architect Design

### `soos-biometric-store`

- `pub const STORE_LOCK_TIMEOUT: Duration = 5 s` (re-exported); private
  `STORE_LOCK_POLL_INTERVAL = 10 ms`.
- `BiometricStore` gains `lock_timeout: Duration` (default `STORE_LOCK_TIMEOUT`) and
  `with_lock_timeout(self, Duration) -> Self` (`Duration::ZERO` = one attempt, no wait).
- Private `lock_store(&self) -> Result<Flock<File>, _>`: `flock(LOCK_EX | LOCK_NB)` on the store
  directory itself, opened `O_RDONLY | O_DIRECTORY | O_NOFOLLOW`; `EAGAIN` / `EINTR` retried every
  10 ms until the timeout, then `LockTimeout`; any other errno is `Io`. The guard releases the lock
  on drop (every error path included). Deviation from the suggested "lock file inside the store
  dir": a directory lock needs no safe creation (no `create_new` / mode / owner race), adds no
  entry to the template directory, cannot be left behind, and is released by the kernel if the
  process dies (ADR).
- Taken by `enroll` and `delete` (so by `soos-enroll enroll` / `import` / `delete` and by the GUI,
  which all call them) and by a real `migrate_template` from its read to its rewrite. Reads
  (`get`, `get_metadata`, `template_format`, `list_enrolled`) and dry runs never take it. The CLI
  `enroll` captures frames first and calls `store.enroll` afterwards, so the lock never covers a
  camera capture.
- Private `FileIdentity { dev, ino, len, mtime, mtime_nsec }` recorded from the read handle of
  the migration (`read_decrypted_with_identity`). `enroll` becomes `lock_store` +
  `write_template(template, None)`; the migration calls `write_template(template,
  Some(&identity))`, which refuses when the write handle on the previous inode does not match
  the identity, or when `symlink_metadata(dest)` no longer matches just before `rename`; the
  temporary file is removed and the file is left as found.
- New error variants: `BiometricStoreError::LockTimeout(String)` ("biometric store '<dir>' is
  locked by another enroll, delete, import or migrate operation; gave up after N ms") and
  `BiometricStoreError::ChangedConcurrently(String)` ("template of UID N changed concurrently
  during migration; left as found"). No consumer matches the enum exhaustively.
- `BiometricStore::open_existing(dir, key)`: `symlink_metadata` then the validation of `new`
  (shared `from_validated`); a missing directory is `Io(NotFound)`, nothing is created.
- Private test seam `migrate_template_with_hook(uid, dry_run, &mut dyn FnMut())`: the hook runs
  between the read and the rewrite; `migrate_template` passes a no-op. It does not read
  `cfg!(test)` and has no production caller other than `migrate_template`.

### `soos-enrollment-cli`

`open_template_store_checked` calls `BiometricStore::open_existing` and maps `Io(NotFound)` to the
existing "no biometrics directory found" skip reason (the separate existence check is gone).

### `soos-daemon`

- `shutdown::remaining_budget(started, budget, now) -> Duration`:
  `budget.saturating_sub(now.saturating_duration_since(started))`.
- `shutdown::shutdown_runtime(runtime, remaining)`: logs `budget_ms` and calls
  `Runtime::shutdown_timeout(remaining)`.
- `main.rs`: `async fn run() -> Result<Duration, _>` (the former body) returns
  `remaining_budget(drain_started, drain_budget, Instant::now())`; a sync `fn main` builds the
  multi-thread runtime explicitly, `block_on(run())`, then `shutdown_runtime(runtime, remaining)`
  (zero budget after a startup failure). Exit is bounded by `connection_timeout` plus the 100 ms
  abort reap of `ConnectionTasks::drain`.

### Invariants touched

Templates `0600`, atomic, symlink-safe (unchanged write path); bounded waits (lock timeout,
runtime shutdown budget); fail closed (lock timeout and identity mismatch change nothing); no
embedding, key or plaintext in any error or log. No PAM, protocol or socket code changes.

## 3. Plan Evaluation

Condensed in this single-agent batch (orchestrator instruction): the plan was checked against
ADR 2026-10-01 "Operator-Run Migration of Legacy v1 Storage Envelopes" (its "no lock ... documented,
not enforced" sentence is now marked superseded) and against the existing tests that inspect
`crates/daemon/src/main.rs` as text (`graceful_shutdown_tests`, `daemon_followups_tests`,
`panic_hook_build_policy_tests`, `systemd_readiness_tests`, `template_model_binding_tests`); every
string they look for is kept. Verdict: approved.

## 4. Phase 2 — Tester Contract (Red)

New tests only (no existing assertion changed):

| Test | Matrix | Red evidence (stubs: `open_existing` delegating to `new`, unused `lock_timeout`, hook without lock or identity check, `remaining_budget` returning the budget, `shutdown_runtime` dropping the runtime) |
|---|---|---|
| `concurrency_tests::test_mls_concurrent_delete_during_migrate_does_not_resurrect` | MLS1 | "a deleted template must never be resurrected by a migration" |
| `concurrency_tests::test_mls_concurrent_enroll_during_migrate_is_not_overwritten` | MLS1 | "the newer enrollment must never be overwritten ...", left 1, right 2 |
| `concurrency_tests::test_mls_unlocked_removal_during_migrate_is_refused_not_resurrected` | MLS2 | "expected ChangedConcurrently, got Ok(Migrated)" |
| `concurrency_tests::test_mls_unlocked_replace_during_migrate_is_refused_and_newer_file_kept` | MLS2 | failed `matches!(.., ChangedConcurrently)` |
| `concurrency_tests::test_mls_unlocked_in_place_rewrite_during_migrate_is_refused` | MLS2 | failed `matches!(.., ChangedConcurrently)` |
| `store_lock_tests::test_mls_enroll_times_out_on_a_held_lock_and_writes_nothing` | MLS3 | "expected LockTimeout, got Ok(())" |
| `store_lock_tests::test_mls_delete_times_out_on_a_held_lock_and_keeps_the_template` | MLS3 | "expected LockTimeout, got Ok(true)" |
| `store_lock_tests::test_mls_migrate_times_out_on_a_held_lock_and_keeps_the_legacy_file` | MLS3 | "expected LockTimeout, got Ok(Migrated)" |
| `store_lock_tests::test_mls_default_lock_timeout_is_bounded` | MLS3 | passes on the stub (constant contract) |
| `store_lock_tests::test_mls_dry_run_and_reads_do_not_wait_for_the_lock` | MLS4 | passes on the stub (non-regression: reads never wait) |
| `store_lock_tests::test_mls_lock_is_released_and_leaves_no_file` | MLS4 | passes on the stub (non-regression: no lock file) |
| `store_lock_tests::test_mls_open_existing_never_creates_the_directory` | MLS5 | "expected NotFound, got Ok(BiometricStore { .. })" |
| `store_lock_tests::test_mls_open_existing_validates_like_new` | MLS5 | passes on the stub (validation parity) |
| `migrate_open_existing_tests::test_mls_migrate_opens_the_template_store_without_creating_it` | MLS6 | "migrate must open the store with the never-create constructor" |
| `migrate_open_existing_tests::test_mls_migrate_skips_a_missing_dir_under_a_missing_parent` | MLS6 | passes (non-regression) |
| `migrate_open_existing_tests::test_mls_migrate_refuses_a_symlinked_biometrics_dir_and_creates_nothing` | MLS6 | passes (non-regression) |
| `shutdown_exit_bound_tests::test_mls_remaining_budget_is_what_is_left_of_the_budget` | MLS7 | left 1s, right 600ms |
| `shutdown_exit_bound_tests::test_mls_remaining_budget_never_exceeds_the_budget` | MLS7 | passes on the stub |
| `shutdown_exit_bound_tests::test_mls_shutdown_runtime_does_not_wait_for_an_abandoned_blocking_write` | MLS7 | "process exit must be bounded by the remaining budget, took 3.89s" |
| `shutdown_exit_bound_tests::test_mls_shutdown_runtime_lets_a_fast_blocking_job_finish` | MLS7 | passes (non-regression) |
| `shutdown_exit_bound_tests::test_mls_production_main_bounds_runtime_shutdown` | MLS8 | "#[tokio::main] drops the runtime ..." |

The race tests use the private hook: the racing thread is started inside the read-to-rewrite
window and given 300 ms to reach the store; the "unlocked" variants simulate a writer that bypasses
the lock with raw `remove_file` / `rename` / in-place `write`. Lock contention is created
deterministically by the test holding `flock` on the store directory (the documented on-disk
protocol). The race item of `open_template_store_checked` (directory removed between check and
open) is not hermetically reproducible; MLS6 checks the constructor used and the missing / symlink
behaviour instead.

### Migrated existing tests

| Test | Change | Mandate |
|---|---|---|
| `installer_templates_contract::test_daemon_unit_requires_deployed_models_and_bounds_restarts` | failure message only: "[Unit] must bound restarts to 5 in 60 s" → "... 5 in 320 s"; the asserted condition (`StartLimitIntervalSec=320` and `StartLimitBurst=5`) is byte-identical | GitHub #289 item, **owner approval 2026-10-01** (relayed by the orchestrator) |

### Flakiness check

`cargo test -p soos-biometric-store mls` and `cargo test -p soos-daemon --test
shutdown_exit_bound_tests`, 10 consecutive runs: 10/10 green.

## 5. Phase 3 — Auditor Constraints

1. No `unwrap` / `expect` / `panic` / indexing in production code: `?`, `is_ok_and`,
   `try_from(..).unwrap_or(u64::MAX)` only (lint-checked with clippy `-D warnings`).
2. Every wait is bounded: lock retries end at `lock_timeout` (sleep clamped to the time left),
   runtime shutdown at `remaining_budget` (saturating; never above the drain budget).
3. The lock fd is opened `O_DIRECTORY | O_NOFOLLOW`: a store directory swapped for a symlink is
   refused; no lock file is created, so no permission or creation race exists.
4. Fail closed: `LockTimeout` and `ChangedConcurrently` leave the store unchanged (temporary file
   removed); in bulk migration they are per-file failures and the exit status is 1.
5. The lock never spans a camera capture (`EnrollmentService::enroll` captures before
   `store.enroll`); reads and dry runs never take it, so the daemon is never blocked.
6. Error messages carry the store path, UID and elapsed milliseconds only; no embedding, key or
   plaintext. The decrypted CBOR stays in `Zeroizing` and is dropped before the rewrite.
7. `#![forbid(unsafe_code)]` kept in `soos-biometric-store`, `soos-enrollment-cli` and the daemon
   `main.rs`; no new dependency (`nix` with `fs` was already a dependency of the store).
8. The daemon log wording stays accurate: "abandoned" now means not awaited by the drain nor by the
   runtime shutdown; the new `shutdown_runtime` log says blocking jobs past the budget are not
   awaited.

Clearance: CLEARED.

## 6. Phase 4 — Implementation

- `crates/biometric-store/src/store.rs`: lock, identity check, `open_existing`,
  `with_lock_timeout`, `STORE_LOCK_TIMEOUT`; `error.rs`: two variants; `lib.rs`: re-export.
- `crates/enrollment-cli/src/service.rs`: `open_template_store_checked` uses `open_existing`.
- `crates/daemon/src/shutdown.rs`: `remaining_budget`, `shutdown_runtime`, doc wording;
  `crates/daemon/src/main.rs`: explicit runtime, `run()` returning the remaining budget.
- `tests/invariants/src/installer_templates_contract.rs`: message-only change (item 4).
- Docs: `Docs/BIOMETRIC_STORE_CRATE.md`, `Docs/ENROLLMENT_CLI.md` (the "do not run it while ..."
  caveat replaced by the concurrency guarantees), `Docs/DAEMON.md` (bounded exit),
  `AI/DECISIONS.md` (two ADRs; the "no lock" sentence of the migration ADR marked superseded),
  `AI/VERIFICATION_MATRIX.md` (MLS1–MLS8).

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) passes. The layer 2 sub-agent review is run by the
orchestrator for the combined batch (not run in this agent, by instruction).

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=5
cargo test --locked --all-features -p soos-biometric-store -p soos-evidence-store \
  -p soos-enrollment-cli -p soos-daemon -p soos-invariants      # 1003 passed, 0 failed
cargo clippy --locked --all-targets --all-features -p soos-biometric-store \
  -p soos-enrollment-cli -p soos-daemon -p soos-invariants -p soos-gui -- -D warnings  # clean
cargo fmt --all -- --check                                      # clean
./scripts/candid_review.sh                                      # pass
```

## 9. Known Limitations / Follow-ups

- The CLI `import` duplicate check (`AlreadyEnrolled` without `--yes`) still runs before the
  locked write; closing it needs an `enroll_if_absent` store call (enrollment-cli import code, not
  in this batch's scope).
- A writer that ignores the lock can still race in the window between the final identity check and
  `rename(2)`; every soos writer takes the lock.
- `soos-enroll migrate` keeps processing other templates when one hits `LockTimeout`; that template
  is reported as failed and can be migrated by a second run.

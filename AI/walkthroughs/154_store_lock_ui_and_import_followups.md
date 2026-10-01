# Walkthrough 154 — Store Lock Off the GUI Thread, Atomic Import Check, Single Config Read and Orphaned Temporary File Sweep

- **Date**: 2026-10-01
- **Issue**: GitHub #291 ("[P3-FU3] Residual follow-ups after #289", storage and GUI items); no
  backlog id — **Branch**: `fix/p3fu3-storage-gui`
- **Matrix criteria**: SGU1–SGU7 (new component `store-lock-ui-and-import-followups`)
- **ADR**: 2026-10-01 "Bounded Startup Sweep of Orphaned Store Temporary Files; Atomic Import
  Duplicate Check; GUI Store Writes Off the UI Thread"
- **Context**: walkthrough 153 (store lock, bounded daemon exit), walkthrough 151 (shared
  `daemon.toml` reader)

---

## 1. Context & Objectives

| #291 item | Outcome | Rows |
|---|---|---|
| GUI direct mode calls `store.enroll` / `store.delete` on the UI thread (freeze up to `STORE_LOCK_TIMEOUT`, 5 s, while `soos-enroll` holds the lock) | Done: `soos_gui::store_tasks::StoreTaskRunner` runs them on a background thread; a lock timeout shows "another soos operation is using the template store; try again" | SGU1, SGU2 |
| `soos-enroll` `warn_camera_config_notes` reads `daemon.toml` separately from `build_full_service` | Done: `build_full_service_with_notes` resolves once through `resolve_camera_device_from_config_reported` and reports that call's notes | SGU6 |
| `soos-enroll import` without `--yes` checks `AlreadyEnrolled` before the locked write | Done: `BiometricStore::enroll_if_absent` (check and write under one lock), used by import | SGU3 |
| Temporary files orphaned by an abandoned blocking write are never swept | Done: bounded, symlink-safe `sweep_orphaned_temp_files` in both store crates, called once at daemon startup | SGU4, SGU5, SGU7 |
| Document the `EBADF` of the store `flock` on NFS | Done: `Docs/BIOMETRIC_STORE_CRATE.md` (local filesystem required) | — (docs) |

Out of scope (other agent / later): `crates/camera-v4l`, `crates/admin-cli`, the hardware checks.
`crates/daemon/src/main.rs` is not touched: the startup call lives in `initialize_pipeline`.

## 2. Phase 1 — Architect Design

### `soos-biometric-store`

```rust
pub const TEMP_SWEEP_MIN_AGE: Duration = Duration::from_secs(60);
pub const MAX_TEMP_SWEEP_REMOVALS: usize = 256;
const MAX_TEMP_SWEEP_SCANNED_ENTRIES: usize = 65_536;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TempSweepReport { pub removed: usize, pub kept_recent: usize, pub limit_reached: bool, pub lock_busy: bool }
impl BiometricStore {
    pub fn enroll_if_absent(&self, template: &BiometricTemplate) -> Result<(), BiometricStoreError>;
    pub fn sweep_orphaned_temp_files(&self) -> Result<TempSweepReport, BiometricStoreError>;
}
// BiometricStoreError::AlreadyEnrolled(u32): "UID N is already enrolled; the existing template was left unchanged"
```

- `enroll_if_absent`: `lock_store()` (bounded by the lock timeout), `template_path` (symlink
  refused), then any entry at the path (`symlink_metadata` succeeds) is `AlreadyEnrolled`;
  otherwise `write_template(template, None)` (the unchanged atomic `0600` write).
- Sweep candidates: exactly `<uid>.tmp.<pid>.<suffix>` (5 dot-separated fields are refused),
  each number canonical (`parse` then `to_string` must round-trip: no sign, no leading zero,
  no overflow) as `u32`, `u32`, `u64`. Removal rule: `fstatat(dirfd, name,
  AT_SYMLINK_NOFOLLOW)` shows `S_IFREG`, `st_nlink == 1`, `st_uid` 0 or euid, and
  `now - mtime >= TEMP_SWEEP_MIN_AGE` (future = recent, pre-epoch = old); then
  `unlinkat(dirfd, name, NoRemoveDir)` (`ENOENT` ignored). The directory descriptor is opened
  `O_RDONLY | O_DIRECTORY | O_NOFOLLOW`, its `(dev, ino)` compared with `symlink_metadata` of the
  path (mismatch: `InvalidPath`), and flocked `LOCK_EX | LOCK_NB` (`EAGAIN` / `EINTR`:
  `lock_busy`, nothing examined). Bounds: 256 removals, 65 536 entries; the directory is
  `fsync`ed after a removal.

### `soos-evidence-store`

Same report type, constants and rule (own copy: the crates are independent). Candidates, only in
real `YYYY-MM-DD` partitions (`DirEntry::file_type` no-follow + `parse_date`):
`.tmp.<uuid>.<pid>.<16 hex>`, `.tmp.migrate.<uuid>.<pid>.<16 hex>`,
`.tmp.daily_count.<uid>.<pid>.<16 hex>` (UUID lowercase `8-4-4-4-12`, uid canonical and
`<= MAX_VALID_UID`, pid canonical `u32`, salt exactly 16 lowercase hex). Exclusion: the
`daily_counts` mutex (in-process writers) then a non-blocking `flock` of the base directory
(retention, migration). A disabled store or missing base returns an empty report; a symlinked
or non-directory base is `InvalidPath`. The partition is opened by path with `O_NOFOLLOW` and
the checks / unlink are relative to its descriptor.

### `soos-daemon`

`pipeline::sweep_orphaned_store_temp_files(&BiometricStore, &EvidenceStore)` runs both sweeps,
logs counts (`info`, only when something happened; `warn` on error) and returns nothing: the
sweep never fails the startup. `initialize_pipeline` calls it once right after both stores are
opened.

### `soos-enrollment-cli`

- `build_full_service_with_notes(cli, config_path, &mut dyn FnMut(&[String]))`: the former
  body of `build_full_service`, with one `resolve_camera_device_from_config_reported` call whose
  notes are handed to the callback exactly once, before `validate_camera_device_path`.
  `build_full_service(cli)` delegates with the default path and a no-op callback (signature
  unchanged).
- `main.rs`: `warn_camera_config_notes` is replaced by `build_camera_service`, which prints the
  callback's notes as `[WARN] ...` (enroll, verify, debug-vision).
- `store_imported(args, bytes, allow_overwrite)`: without `allow_overwrite` it calls
  `enroll_if_absent` and maps `AlreadyEnrolled(uid)` to `EnrollmentCliError::AlreadyEnrolled`;
  the early `refuse_unconfirmed_overwrite` stays (a refused stdin import never consumes its
  input, GitHub #237). `import_with_overwrite_from_reader` now reads the input and calls
  `store_imported` directly; `import_from_reader` (library, no gating) keeps replacing.

### `soos-gui`

New module `store_tasks`: `StoreTask { Enroll(BiometricTemplate), Delete { uid } }` (manual
`Debug`: UID and dimension only), `StoreTaskOutcome { Enrolled { uid, result }, Deleted { uid,
result } }`, `StoreTaskError { Busy, Failed(String) }` (`From<&BiometricStoreError>`:
`LockTimeout` is `Busy`), `StoreTaskSubmitError { Busy, Spawn }`, `STORE_BUSY_MESSAGE`, and
`StoreTaskRunner::{new(store, notify), submit, is_busy, poll}` with the semantics of
`TaskRunner`. `SoosApp` holds `Option<StoreTaskRunner>` (only with a local store), submits
instead of calling the store, and applies outcomes in `handle_store_task_outcomes` each frame.

Design choice: a dedicated runner instead of new `PrivilegedAction` variants. The existing
`responsiveness_tests::GateExecutor` matches `PrivilegedAction` exhaustively, so new variants
would force a change to an existing test; a local save would also queue behind a pending Polkit
dialog (Pause/Resume) on the shared runner.

### Invariants touched

Templates and evidence stay `0600`, atomic, symlink-safe (write paths unchanged); every wait is
bounded (lock timeout; sweeps never wait); fail closed (`AlreadyEnrolled`, `Busy` and a lock
timeout change nothing); no embedding, key or plaintext in any message or log (counts, UIDs and
paths only). No PAM, protocol or socket change.

## 3. Plan Evaluation

Condensed in this single-agent batch (orchestrator instruction). Checked against ADR 2026-10-01
"Biometric Store Lock and Migration Identity Check" (the lock protocol is reused, unchanged),
"Daemon Exit Bounded by the Shutdown Drain Budget" (the reason orphans exist) and "One Shared
`daemon.toml` Camera Reader" (the `_reported` resolver is reused), and against the existing tests
that read `crates/enrollment-cli/src/main.rs`, `crates/gui/src/app.rs` and
`crates/daemon/src/pipeline.rs` as text: no string they look for is removed. Verdict: approved.

## 4. Phase 2 — Tester Contract (Red)

Stubs: `enroll_if_absent` delegating to `enroll`, both sweeps returning an empty report,
`build_full_service_with_notes` ignoring its path and callback, an inline (blocking)
`StoreTaskRunner` mapping every error to `Failed`, an empty `sweep_orphaned_store_temp_files`.

| Test | Rows | Red evidence |
|---|---|---|
| `enroll_if_absent_tests::test_sgu_enroll_if_absent_refuses_an_enrolled_uid_and_keeps_it_byte_for_byte` | SGU3 | "expected AlreadyEnrolled(1000), got Ok(())" |
| `enroll_if_absent_tests::test_sgu_enroll_if_absent_checks_under_the_lock_against_a_concurrent_enroll` | SGU3 | "expected AlreadyEnrolled(1000), got Ok(())" |
| `enroll_if_absent_tests::test_sgu_concurrent_enroll_if_absent_has_exactly_one_winner` | SGU3 | left 8, right 1 |
| `enroll_if_absent_tests::test_sgu_enroll_if_absent_writes_a_missing_template`, `test_sgu_already_enrolled_error_names_the_uid_only` | SGU3 | pass on the stub (nominal path, message) |
| `import_enroll_if_absent_tests::test_sgu_import_without_yes_never_replaces_a_concurrent_enrollment` | SGU3 | "the import must be refused atomically, got Ok(..)" |
| `import_enroll_if_absent_tests::test_sgu_import_with_yes_still_replaces_and_reports_it` | SGU3 | passes (non-regression) |
| `temp_sweep_tests::test_sgu_sweep_removes_old_orphaned_template_temp_files`, `test_sgu_sweep_keeps_recent_and_future_dated_temp_files`, `test_sgu_sweep_is_capped_per_call`, `test_sgu_sweep_touches_nothing_while_the_store_lock_is_held` (biometric) | SGU4 | removed 0 vs 3 / kept_recent 0 vs 3 / 0 vs 256 / `lock_busy` false |
| `temp_sweep_tests::test_sgu_sweep_keeps_every_decoy_and_the_templates`, `test_sgu_sweep_never_follows_or_removes_non_regular_entries`, `test_sgu_sweep_minimum_age_and_cap_are_bounded` | SGU4 | pass on the stub (nothing may be removed; constants) |
| `temp_sweep_tests::test_sgu_evidence_sweep_*` (6 of 8) | SGU5 | removed 0 vs 4 / 0 vs 256 / `lock_busy` false / no `InvalidPath`; the decoy and disabled-store tests pass on the stub |
| `store_temp_sweep_tests::test_sgu_*` (3) | SGU7 | orphan still present; no `sweep_orphaned_store_temp_files(` call in `initialize_pipeline` |
| `full_service_config_notes_tests::test_sgu_*` (4) | SGU6 | callback never called (0 vs 1); `main.rs` still calls `camera_config_notes(` |
| `store_task_tests::test_sgu_enroll_submit_never_blocks_on_a_held_store_lock`, `test_sgu_delete_submit_never_blocks_and_keeps_the_template_when_busy`, `test_sgu_only_one_store_task_runs_at_a_time` | SGU1 | `submit` took the whole lock wait |
| `store_task_tests::test_sgu_store_task_errors_map_lock_timeout_to_the_busy_message` | SGU2 | left `Failed("Biometric store is busy: held")`, right `Busy` |
| `store_task_tests::test_sgu_app_never_mutates_the_store_on_the_ui_thread` | SGU1 | "`app.rs` (UI thread) must not call `store.enroll(`" |
| `store_task_tests::test_sgu_store_task_debug_never_prints_embedding_values` | SGU2 | passes on the stub (the derived `Debug` delegates to the manual, value-free `Debug` of `BiometricTemplate`; kept as a non-regression) |

Contention is deterministic: the tests hold `flock` on the store directory (the documented
on-disk protocol); the import test's reader signals once the early check has passed, and the
concurrent enrollment lands while the import waits for the lock. Old files are made with
`File::set_modified`; a FIFO with `mkfifo`. Owner filtering (root or euid) cannot be exercised
without root (a non-root test cannot create a foreign-owned file); it is a one-line predicate
checked by review.

During Phase 4 two defects of the new (uncommitted) evidence test were fixed: an unused
variable, and `test_sgu_evidence_sweep_only_looks_inside_date_partitions` expected 5 base entries
where the fixture creates 4; the count was replaced by the exact expected name list (stronger).

### Migrated existing tests

None. No existing test file was modified.

### Flakiness check

`enroll_if_absent_tests`, `import_enroll_if_absent_tests`, `store_task_tests` and both
`temp_sweep_tests`, 10 consecutive runs: 10/10 green.

## 5. Phase 3 — Auditor Constraints

1. No `unwrap` / `expect` / `panic` / indexing in production code (`?`, `is_some_and`,
   `checked_add`, `try_from`, `saturating_add`); clippy `-D warnings` clean.
2. Every wait is bounded: `enroll_if_absent` by the lock timeout; both sweeps try their lock once
   (`LOCK_NB`) and stop after 256 removals / 65 536 entries; the GUI never waits (worker thread).
3. Symlink safety: directory descriptors opened `O_DIRECTORY | O_NOFOLLOW`; `fstatat` with
   `AT_SYMLINK_NOFOLLOW` and `unlinkat` relative to them; only `S_IFREG`, single-link, root/euid
   files; symlinked partitions skipped; base / store path identity checked against the
   descriptor.
4. Never delete anything else: exact name grammar with canonical numbers and lowercase hex /
   UUID; minimum age 60 s against a write in progress; future modification times never old;
   decoy tests cover near-miss names.
5. Fail closed: `AlreadyEnrolled`, `Busy`, `LockTimeout` change nothing; the sweep is
   housekeeping and its failure never makes the daemon serve anything differently.
6. No secret in logs or messages: `StoreTask` `Debug` prints UID and dimension only; sweep logs
   carry counts and the store name; `AlreadyEnrolled` carries the UID. The daemon log audit
   (`logging_audit_test`) passes.
7. `#![forbid(unsafe_code)]` kept in every touched crate; `as_raw_fd` is safe; no new
   dependency (`nix` `fs` was already enabled workspace-wide).
8. Storage-only commands still open no camera: `import` uses `build_store_only`; only camera
   commands use `build_full_service_with_notes`.

Clearance: CLEARED.

## 6. Phase 4 — Implementation

- `crates/biometric-store/src/store.rs`: `enroll_if_absent`, `sweep_orphaned_temp_files`,
  constants, `TempSweepReport`, name / age / candidate helpers; `error.rs`: `AlreadyEnrolled`;
  `lib.rs`: re-exports.
- `crates/evidence-store/src/store.rs`: `sweep_orphaned_temp_files`, constants,
  `TempSweepReport`, helpers; `lib.rs`: re-exports.
- `crates/daemon/src/pipeline.rs`: `sweep_orphaned_store_temp_files`, call in
  `initialize_pipeline`.
- `crates/enrollment-cli/src/service.rs`: `build_full_service_with_notes`, `store_imported`
  with `allow_overwrite`; `lib.rs`: re-export; `main.rs`: `build_camera_service`.
- `crates/gui/src/store_tasks.rs` (new), `lib.rs`, `app.rs` (submit + outcome handler).
- Docs: `Docs/BIOMETRIC_STORE_CRATE.md` (NFS / local filesystem, `enroll_if_absent`, sweep,
  rows), `Docs/EVIDENCE_STORE_CRATE.md` (§4.6), `Docs/ENROLLMENT_CLI.md` (import, notes),
  `Docs/GUI_APPLICATION.md` (store worker), `Docs/DAEMON.md` (startup sweep);
  `AI/DECISIONS.md` (ADR), `AI/VERIFICATION_MATRIX.md` (SGU1–SGU7).

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) passes. The layer 2 sub-agent review is run by the
orchestrator for the combined batch (not run in this agent, by instruction).

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=6
cargo test --locked --all-features -p soos-biometric-store -p soos-evidence-store \
  -p soos-enrollment-cli -p soos-gui -p soos-daemon -p soos-invariants   # 1151 passed, 0 failed
cargo clippy --locked --all-targets --all-features -p soos-biometric-store \
  -p soos-evidence-store -p soos-enrollment-cli -p soos-gui -p soos-daemon \
  -p soos-invariants -- -D warnings                                      # clean
cargo fmt --all -- --check                                               # clean
./scripts/candid_review.sh                                               # pass
```

## 9. Known Limitations / Follow-ups

- The store lock does not work on NFS (`EBADF`, fail closed); documented, not worked around.
- The owner filter of the sweeps is not covered by an automated test (needs a foreign-owned file,
  i.e. root).
- Master-key temporary files (`master.key.tmp.*`, `evidence.key.tmp.*`) are not swept; they live
  outside the store directories and are only created once.
- A GUI save shows "Saving the template..." for up to the lock timeout (5 s) when `soos-enroll`
  holds the lock; the window stays responsive meanwhile.
- The sweep runs at daemon startup only (not at the start of `soos-enroll migrate`).

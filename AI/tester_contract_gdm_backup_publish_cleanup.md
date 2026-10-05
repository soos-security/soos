# Tester Contract — GDM Backup Publication Cleanup (GitHub #335)

- **Branch**: `fix/gdm-backup-publish-cleanup`
- **Spec**: `AI/architect_spec_gdm_backup_publish_cleanup.md` (Revision 1 supersedes §2.2/§3)
- **Plan evaluation**: `AI/plan_evaluator_report.md` round 2 (APPROVED; MINOR 1, 3 and 4 encoded below)
- **Location**: new test module `backup_publish_tests` in `crates/admin-cli/src/gdm.rs` (sibling of
  `enable_race_tests`; no existing test modified)
- **Red command**: `cargo test -p soos-admin-cli --lib --locked --all-features`
  → `52 passed; 7 failed` (every failure is an assertion, no compile error)

## GBP id → test → observed red reason

| ID | Test | Red reason (observed) |
|---|---|---|
| GBP1 | `test_gbp1_stale_backup_temp_file_does_not_block_enable` (cases: legacy `.<backup>.soos-tmp-<pid>` alone, `-<pid>-0` alone, both; PID from `std::process::id()`) | legacy case: `enable must succeed: ... Failed to write PAM file atomically '<dir>/gdm-password.soos-backup': File exists (os error 17)` |
| GBP1 (exhaustion) | `test_gbp1_exhausted_backup_temp_names_fail_without_removing_anything` (16 stale `-<pid>-0..15`) | `16 taken temporary names must fail the enable: ()` (current code uses the legacy name and succeeds) |
| GBP2 | `test_gbp2_stale_pam_temp_file_does_not_block_enable` (same three cases on `.gdm-password.soos-tmp-…`) | legacy case: `Failed to write PAM file atomically '<dir>/gdm-password': File exists (os error 17)` |
| GBP3 | `test_gbp3_post_link_failure_removes_created_backup` (cases: hook removes its tmp / hook does NOT remove its tmp, plan-eval MINOR 1) | `the post-link hook runs once` left 0 right 1 (stub never calls the hook) |
| GBP3 (guard) | `test_gbp3_preexisting_backup_never_reaches_post_link_hook` | Passes today by design (regression guard: fast path, hook 0 calls, existing backup same inode/bytes) |
| GBP3b | `test_gbp3b_post_link_failure_keeps_backup_when_file_holds_managed_block` | post-link calls left 0 right 1 |
| GBP4 | `test_gbp4_post_link_failure_leaves_a_replaced_backup_and_reports_it` | post-link calls left 0 right 1 |
| GBP5 | `test_gbp5_stale_pam_temp_file_does_not_block_restore` (three cases) | legacy case: `restore must succeed: ... Failed to write PAM file atomically '<dir>/gdm-password': File exists (os error 17)` |

## Pinned assertions (developer must satisfy)

- Stale files are byte-identical **and same inode**; the directory listing equals exactly the
  expected set (no temporary file of this run remains, nothing extra).
- Exhaustion: error starts with `GDM configuration error: Failed to write PAM file atomically '<backup>': `
  and contains the `AlreadyExists` I/O text (`File exists`); PAM file = PRISTINE; no backup.
- GBP3/GBP3b: error is exactly
  `GDM configuration error: Failed to write PAM file atomically '<backup>': injected post-link failure`.
- GBP3/3b/4: post-link hook called exactly once, `before_rename` called **0 times** (plan-eval
  MINOR 3). Inside the hook (GBP3): `dir` == PAM directory, `tmp` is in that directory, named
  `.gdm-password.soos-backup.soos-tmp-…`, still present, holds PRISTINE and shares the backup's
  inode (hook runs right after the hard link).
- GBP3 with a hook that does not remove its tmp: the temporary file is still removed
  (best-effort removal of the helper-returned path after a hook error, `NotFound` ignored).
- GBP4: message starts with the GBP3 error and contains `could not be removed: it was replaced`.

## Production stubs added (Phase 2)

- `crates/admin-cli/src/gdm.rs`: new private
  `fn ensure_gdm_pam_line_with_hooks(pam_file, before_rename: &mut dyn FnMut(), post_link: &mut dyn FnMut(&Path, &Path) -> std::io::Result<()>) -> Result<(), AdminCliError>`
  holding the former body of `ensure_gdm_pam_line_with`; **the stub ignores `post_link`**
  (`let _ = post_link;`). `ensure_gdm_pam_line_with` keeps its exact signature and delegates with
  a no-op post-link closure — the developer replaces it with the production steps (remove own
  temporary file, fsync directory) and wires `post_link` into `publish_backup`.
- No other production change (`create_temp_sibling` is not stubbed; it is private and exercised
  only through `enable`/`restore`).

## Non-test change

- `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs` GHF7 doc comment: now reads
  "through `sd_notify::notify_ready()` (the stamped variant since OA-4)" (spec §2.3; comment only,
  `cargo test -p soos-invariants` 448 passed).

## Gate status at hand-off

- `cargo fmt` applied; `cargo clippy -p soos-admin-cli --all-targets --all-features --locked -- -D warnings` clean.
- All pre-existing admin-cli lib tests pass (52); only the 7 new GBP tests fail.

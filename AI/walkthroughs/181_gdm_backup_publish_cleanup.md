# Walkthrough 181 — GDM Backup Publication Cleanup and Collision-Resistant Temporary Names

- **Date**: 2026-10-05
- **Issue**: GitHub #335 (GitHub-only follow-up of the candid review of #333 / PR #334, no backlog id;
  not registered in `scripts/sync_issue.py`, the squash commit carries `Closes #335`) — **Branch**:
  `fix/gdm-backup-publish-cleanup`
- **Matrix criteria**: GBP1, GBP2, GBP3, GBP3b, GBP4, GBP5 (component `gdm-backup-publish-cleanup`)

## 1. Context & Objectives

The candid review of PR #334 (walkthrough 180) left three items:

1. `publish_backup` hard-linked the backup, then removed its temporary file and fsynced the directory;
   a failure in either step returned an error **after** the backup was published, so a failed
   `gdm enable` could leave a new backup behind.
2. Both temporary-file sites (`publish_backup`, `write_atomic_checked`) used `.<file>.soos-tmp-<pid>`.
   A file left by a crashed run with a reused PID made `create_new` fail with `AlreadyExists`, and both
   sites then removed that name although this run had not created it.
3. The GHF7 doc comment in `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs` still named
   `notify_ready_stamped`.

## 2. Architect Design

Spec: `AI/architect_spec_gdm_backup_publish_cleanup.md` (Revision 1 supersedes §2.2/§3).

- `const MAX_TEMP_NAME_ATTEMPTS: u32 = 16` and
  `fn create_temp_sibling(dir, file_name) -> io::Result<(PathBuf, fs::File)>`: `create_new` + `0600`
  on `.<file>.soos-tmp-<pid>-<n>`, `n` in `0..16`, retry only on `AlreadyExists`.
- `fn ensure_gdm_pam_line_with_hooks(pam_file, before_rename, post_link)`: the former body of
  `ensure_gdm_pam_line_with`, which keeps its signature and delegates with the production post-link
  steps.
- A post-link failure goes through the unchanged `discard_created_backup` (managed block or unreadable
  file ⇒ keep; same `(dev, ino)` ⇒ remove; replaced ⇒ note).
- Invariants: fail closed, never remove or modify a file this run did not create, no PAM change, no
  `unwrap`/`expect`, no `unsafe`, no new dependency.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: round 1 REVISION_REQUIRED (2 MAJOR: custom guard without the
managed-block re-read, seam unreachable from `enable`); round 2 APPROVED with four MINOR
clarifications, all applied: best-effort re-removal of the own temporary file after a hook error
(MINOR 1), ADR wording with the managed-block / unreadable exception (MINOR 2), `before_rename` called
0 times on a post-link failure (MINOR 3), legacy `-<pid>` stale name planted alone (MINOR 4).

## 4. Tester Contract

`AI/tester_contract_gdm_backup_publish_cleanup.md`, new module `gdm::backup_publish_tests` (no
existing test modified). Red run: `cargo test -p soos-admin-cli --lib --locked --all-features` →
52 passed, 7 failed (assertion failures only).

| ID | Test |
|---|---|
| GBP1 | `test_gbp1_stale_backup_temp_file_does_not_block_enable`, `test_gbp1_exhausted_backup_temp_names_fail_without_removing_anything` |
| GBP2 | `test_gbp2_stale_pam_temp_file_does_not_block_enable` |
| GBP3 | `test_gbp3_post_link_failure_removes_created_backup`, `test_gbp3_preexisting_backup_never_reaches_post_link_hook` (regression guard, green by design) |
| GBP3b | `test_gbp3b_post_link_failure_keeps_backup_when_file_holds_managed_block` |
| GBP4 | `test_gbp4_post_link_failure_leaves_a_replaced_backup_and_reports_it` |
| GBP5 | `test_gbp5_stale_pam_temp_file_does_not_block_restore` |

## 5. Auditor Constraints

`AI/auditor_constraints_gdm_backup_publish_cleanup.md` (CLEARED, 15 constraints):

1. No panic path or unchecked arithmetic: the retry is a `0..16` range; exhaustion returns the last
   `AlreadyExists` error.
2. `create_temp_sibling` uses `write(true).create_new(true).mode(0o600)`, retries only on
   `AlreadyExists`.
3. Both sites remove only the helper-returned path; nothing is removed when creation fails.
4. Helper errors keep `Failed to write PAM file atomically '<path>': <io>`.
5. `post_link(tmp, dir)` runs once, only after a successful hard link; never on the fast path or on a
   link-time `AlreadyExists` (there only the own temporary file is removed).
6. Production steps `backup_post_link_steps` are today's (remove ignoring `NotFound`, fsync the
   directory); the stub closure and the "TDD RED STUB" paragraph are gone.
7. On a hook error: own temporary file removed again, `discard_created_backup` with the atomic-write
   error, `Err` returned; the PAM-file write is never reached.
8. `discard_created_backup` is byte-identical.
9. Implemented: the post-hook re-removal (`remove_own_file`) unlinks only while `symlink_metadata(tmp)`
   is a regular file with the created `(dev, ino)`.
10. The `_hooks` body still reads only through `read_pam_file_snapshot`.
11. `.soos-backup`, `sync_all`, `fs::rename`, `soos-tmp` substrings kept; no new `custom_flags`.
12. Doc comments of `publish_backup`, `write_atomic_checked`, `ensure_gdm_pam_line_with_hooks` updated.
13. No test line modified.
14. ADR, `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 and matrix rows added (this walkthrough's Phase 6).
15. Gates run (section 8).

## 6. Implementation

`crates/admin-cli/src/gdm.rs` only (production):

- `create_temp_sibling`, `MAX_TEMP_NAME_ATTEMPTS`, `remove_own_file`, `backup_post_link_steps`.
- `publish_backup` takes `pam_file` and `post_link`; `link_new_file` and `write_and_rename` receive the
  already-created `fs::File` (`write_and_rename` derives the directory from `target` with `parent_dir`,
  keeping its argument count within the Clippy limit).
- `write_atomic_checked` creates its temporary file through the helper and, on failure, removes only
  that path.

Non-production: the GHF7 doc comment (comment only) was updated by the tester phase.

## 7. Candid Review

Pending at the time of writing (Phase 5 runs on the final diff and writes
`AI/candid_review_report.md`).

## 8. Verification Results

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets --locked --all-features -- -D warnings`: clean.
- `cargo test -p soos-admin-cli --lib --locked --all-features`: 59 passed, 0 failed (run 10 times,
  10/10 green).
- `cargo test --workspace --locked --all-features`: all green.
- `python3 scripts/sync_issue.py --check`: passes (branch is GitHub-only).

## 9. Known Limitations / Follow-ups

- The GHF2 invariant needle scans only `fn ensure_gdm_pam_line_with(` (now a delegator); the real body
  lives in `ensure_gdm_pam_line_with_hooks` and is kept free of path-based reads by review (auditor
  constraint 10). A follow-up may extend the needle.
- The production post-link removal of the temporary file is unguarded by name (unchanged from #333);
  a root-only replacement of that name between the link and the unlink stays a residual.
- A stale symlink at a temporary name and the link-time `AlreadyExists` race are covered by review,
  not by tests (`O_EXCL` semantics are identical to a regular stale file).

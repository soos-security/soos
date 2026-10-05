# Audit Constraints — GDM Backup Publication Cleanup (GitHub #335)

- **Branch**: `fix/gdm-backup-publish-cleanup` (base `7d98021`)
- **Inputs**: `AI/architect_spec_gdm_backup_publish_cleanup.md` (§1–§5 + Revision 1),
  `AI/plan_evaluator_report.md` (round 2 APPROVED, MINOR 1–4),
  `AI/tester_contract_gdm_backup_publish_cleanup.md`, uncommitted diff.
- **Scope audited**: `crates/admin-cli/src/gdm.rs` (production: `ensure_gdm_pam_line_with*`,
  `publish_backup`, `link_new_file`, `discard_created_backup`, `write_atomic_checked`,
  `write_and_rename`, `restore_gdm_pam_file_with`), new module `backup_publish_tests`,
  `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs` (GHF7 comment).

## Checklist results

| Item | Result |
|---|---|
| 1. Panic paths | Production part of `gdm.rs` (lines 1–1095): 0 `unwrap`/`expect`/`panic!`/`todo!`/`unreachable!`. New tests carry a scoped `#[allow(..., reason = ...)]`. |
| 2. Unsafe | None in `gdm.rs`; `soos-admin-cli` keeps `#![forbid(unsafe_code)]`. No change needed. |
| 3. Output isolation | No print added; PAM crate untouched. |
| 4. Bounded I/O / loops | New retry loop is specified as `0..16` (`MAX_TEMP_NAME_ATTEMPTS`); no unbounded loop. |
| 5. Arithmetic | Only a range iterator is needed; no `+ 1` on counters in production. |
| 6. Filesystem safety | `create_new(true)` (`O_CREAT|O_EXCL`) never opens, truncates or follows an existing name (a symlink included), so a stale or hostile temp name only consumes one attempt. Today `publish_backup` and `write_atomic_checked` remove the computed temp name even when `create_new` failed on a stale file (deleting a file this run did not create) — fixed by design (helper-returned path only). |
| 7. Secrets | No new logging; error texts carry paths and I/O errors only. |
| 8. Fail-closed | Every new path returns `Err`; a post-link failure must still fail `enable` even when the backup is removed; no PAM code touched. |
| 9. Supply chain | No new dependency. |
| 10. CI/scripts | Not touched. |
| Test integrity | `git diff -U0` on `gdm.rs` shows 0 removed lines (stub + appended tests only). Invariants file: GHF7 doc comment only, no assertion or needle changed. Red run: `52 passed; 7 failed`, all assertion failures (confirmed). |

## Audit Constraints — GitHub #335

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| 1 | No `unwrap`/`expect`/`panic!`/indexing/unchecked arithmetic in production code; the exhaustion path returns the last `AlreadyExists` `io::Error` (or `io::Error::from(ErrorKind::AlreadyExists)`) without `Option::unwrap`. | `gdm.rs::create_temp_sibling`, `publish_backup`, `write_atomic_checked` | `cargo clippy -p soos-admin-cli --all-targets --all-features --locked -- -D warnings`; grep on lines before the first `#[cfg(test)]` |
| 2 | `create_temp_sibling(dir, file_name)` tries exactly `n in 0..MAX_TEMP_NAME_ATTEMPTS` (`const MAX_TEMP_NAME_ATTEMPTS: u32 = 16`), name `.{file_name}.soos-tmp-{pid}-{n}`, `OpenOptions::new().write(true).create_new(true).mode(0o600)`; retries **only** on `ErrorKind::AlreadyExists`, returns any other error at once; returns `(PathBuf, fs::File)` of the file it created. Never `create(true)`, `truncate(true)` or a pre-open `remove_file`. | `gdm.rs::create_temp_sibling` | GBP1, GBP1-exhaustion, GBP2, GBP5; code review |
| 3 | Both temp sites use the helper; a temp path is removed **only** after the helper returned it (never the computed name when creation failed). No glob/prefix cleanup of other `soos-tmp` files. | `gdm.rs::publish_backup`/`link_new_file`, `write_atomic_checked`/`write_and_rename` | GBP1/GBP2/GBP5 (stale same inode + bytes, exact dir listing), exhaustion test; review |
| 4 | Helper errors map to the existing wording: `Failed to write PAM file atomically '<backup>': <io>` for the backup site and `... '<target>': <io>` for the PAM-file site (exhaustion text contains `File exists`). | `publish_backup`, `write_atomic_checked` | `test_gbp1_exhausted_backup_temp_names_fail_without_removing_anything` |
| 5 | `post_link(tmp, dir)` is invoked exactly once and **only** when `hard_link` succeeded (identity `(dev, ino)` known); never on the fast path (`symlink_metadata(backup)` found a name) nor on link-time `AlreadyExists` (`Ok(None)`), where only this run's own temp file is removed (non-`NotFound` error still returned, nothing created). `tmp` passed to the hook is the helper-returned path; `dir` is `parent_dir(backup)`. | `publish_backup` (or its `_with` variant) | GBP3 (`assert_hook_args`, `post_calls == 1`), `test_gbp3_preexisting_backup_never_reaches_post_link_hook` |
| 6 | Production post-link steps (passed by `ensure_gdm_pam_line_with`) are exactly today's: remove `tmp` ignoring `NotFound`, then `fs::File::open(dir)?.sync_all()`. `ensure_gdm_pam_line_with` keeps its signature; the stub closure `|_tmp, _dir| Ok(())`, `let _ = post_link;` and the "TDD RED STUB" doc paragraph are removed. | `ensure_gdm_pam_line_with`, `ensure_gdm_pam_line_with_hooks` | grep `TDD RED STUB\|let _ = post_link` = 0; existing 52 tests stay green |
| 7 | On a post-link hook error: (a) best-effort remove this run's temp path again, ignoring every error (`NotFound` included) — plan-eval MINOR 1; (b) hand the identity to the **unchanged** `discard_created_backup(pam_file, &backup, identity, err)` with `err = gdm_error("Failed to write PAM file atomically", backup, &hook_err)`; (c) return its result as `Err`. Never return `Ok` after a post-link failure; never call `write_atomic_checked` / `before_rename` afterwards. Signature choice (pass `pam_file`, or return `(err, Some(identity))`) is free. | `publish_backup`, `ensure_gdm_pam_line_with_hooks` | GBP3 (both cases, exact message, `before_calls == 0`, listing `{gdm-password}`), GBP3b, GBP4 |
| 8 | `discard_created_backup` and its note texts stay byte-identical (no custom guard, no new suffix; Rev1 item 1/3). | `discard_created_backup` | GBP4 contains `could not be removed: it was replaced`; `git diff` of that fn empty |
| 9 | Recommended (not test-pinned): the post-hook best-effort temp removal of constraint 7(a) only unlinks when `symlink_metadata(tmp)` is a regular file with the created `(dev, ino)`; if the developer keeps an unguarded unlink, the residual (root-only replacement of a name in the PAM directory between link and unlink) must be stated in the `publish_backup` doc comment next to the existing #333 residual. | `publish_backup` | review / candid reviewer |
| 10 | GHF2 strength preservation: the invariant scans only `fn ensure_gdm_pam_line_with(` (now a one-line delegator). The moved body in `ensure_gdm_pam_line_with_hooks` MUST still contain none of `read_bounded_utf8(`, `symlink_metadata(pam_file`, `fs::metadata(pam_file`, and must keep reading through `read_pam_file_snapshot`. | `ensure_gdm_pam_line_with_hooks` | `grep` on the `_hooks` body = 0 hits; `cargo test -p soos-invariants` |
| 11 | Keep the substrings other invariants rely on: `.soos-backup`, `sync_all`, `fs::rename` (lib.rs:3538-3549), `pub const GDM_PAM_LINE`, `soos-tmp` in temp names (existing `leftover_temp_files` filters), and `custom_flags(` lines with `O_NOFOLLOW` also carrying `O_NOCTTY`/`O_NONBLOCK` (GHF2) — do not add a new `custom_flags(O_NOFOLLOW ...)` without them. | `gdm.rs` | `cargo test -p soos-invariants --locked` |
| 12 | Doc comments updated to the new behaviour: `publish_backup` ("the temporary name is always removed" → only this run's own temp file; failed post-link step removes the created backup via `discard_created_backup`), `write_atomic_checked` (removes only the temp file it created), `ensure_gdm_pam_line_with_hooks` (production steps). | `gdm.rs` | review |
| 13 | Zero test weakening: no line of an existing test or of `backup_publish_tests` is modified, removed or `#[ignore]`d; the invariants file keeps only the GHF7 comment change. | all tests | `git diff -U0 -- crates/admin-cli/src/gdm.rs` shows removals only inside production functions; `git diff tests/invariants` = comment only |
| 14 | Traceability (Phase 6): ADR amendment uses plan-eval MINOR 2 wording ("...leaves no new backup unless the re-read PAM file holds a soos managed block or cannot be re-read"); `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 sentence (Rev1 item 6); matrix rows GBP1–GBP5 incl. GBP3b. | `AI/DECISIONS.md`, `Docs/`, `AI/VERIFICATION_MATRIX.md` | traceability-agent / candid reviewer |
| 15 | Gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test -p soos-admin-cli --lib --locked --all-features` (59 passed), `cargo test -p soos-invariants --locked`. | workspace | CI commands |

## Test strength notes

- GBP3 is robust: it fails against the current code, a "swallow and continue" implementation
  (`before_calls == 0`, exact error), an unguarded backup removal (GBP3b, GBP4), and a missing
  re-removal of the temp file (hook-does-not-remove case, exact listing).
- GBP1/GBP2/GBP5 pin stale files by inode and bytes plus exact directory listings; the legacy
  `-<pid>` case is the red case, `-0` catches a non-retrying implementation, exhaustion catches an
  unbounded or non-retrying one.
- Not covered by tests (accepted, review-enforced by constraints 3, 5, 9): stale **symlink** at a
  temp name (semantics identical to a regular stale file under `O_EXCL`), link-time
  `AlreadyExists` race (hook not called), PAM-file temp-name exhaustion during `enable` (backup
  then discarded by the existing #333 path).
- Tests in one process share the PID but use separate temp directories: no cross-test collision.

### Pre-existing violations found (not introduced by this change)

- `publish_backup` and `write_atomic_checked` delete the computed temp name even when
  `create_new` failed on a stale file left by another run (fixed by this change).
- GHF2 needle covers only the function named `ensure_gdm_pam_line_with`; after the delegation it
  no longer covers the real body (mitigated by constraint 10; a follow-up may extend the needle to
  `ensure_gdm_pam_line_with_hooks`).
- Cosmetic: the new GHF7 doc comment's second line exceeds 100 columns (rustfmt does not wrap
  comments; may be rewrapped, comment-only).

### Clearance: CLEARED

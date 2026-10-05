# Architect Spec — GDM Backup Publication Cleanup (GitHub #335)

Branch: `fix/gdm-backup-publish-cleanup` (GitHub-only, not registered in `scripts/sync_issue.py`).
Source: MINOR findings and SUGGESTION of the candid review of #334 (GitHub #333).
Matrix prefix: `GBP` (GBP1–GBP4). Walkthrough: next free number (181 expected).

## 1. Scope

`crates/admin-cli/src/gdm.rs` only (production), plus one comment in
`tests/invariants/src/gdm_hardening_flaky_tests_contract.rs`, the ADR amendment, matrix rows and
the walkthrough. No PAM, protocol, daemon, packaging or script change.

## 2. Design

### 2.1 Collision-resistant temporary names (issue item 2)

Both temporary-file sites (`publish_backup` and `write_atomic_checked`) name the temporary file
`.<file>.soos-tmp-<pid>`. After a crash, a leftover file with a reused PID makes `create_new`
fail with `AlreadyExists`, so the next run fails once.

New private helper:

```rust
/// Upper bound on fresh temporary names tried before giving up.
const MAX_TEMP_NAME_ATTEMPTS: u32 = 16;

/// Creates a new `0600` file `<dir>/.<file_name>.soos-tmp-<pid>-<n>` with `create_new`
/// (never opens or truncates an existing file, never follows a symlink at that name),
/// trying n = 0..MAX_TEMP_NAME_ATTEMPTS on `AlreadyExists`; returns the path actually
/// created and its handle. Any other error is returned at once; after the last attempt the
/// `AlreadyExists` error is returned.
fn create_temp_sibling(dir: &Path, file_name: &str) -> std::io::Result<(PathBuf, fs::File)>;
```

- Names keep the `soos-tmp` substring (existing tests and cleanup expectations rely on it).
- Callers remove only the path returned by the helper; a pre-existing stale file is never
  removed or modified (today `write_atomic_checked` removes `tmp` on any failure, including
  the case where `create_new` failed because a stale file existed — that deletion of a file this
  run did not create goes away).
- Bounded: at most 16 `open` calls; no loop without bound; `n` is a `u32` counter with no
  overflow (range `0..16`).

### 2.2 Consistent failure after the hard link (issue item 1)

`publish_backup` today: `link_new_file` hard-links the temporary file to the backup name, then
removes the temporary file and fsyncs the directory; if either of those fails, the run returns
an error **after** publishing the backup, so a failed `enable` leaves a new backup behind.

New rule — **a failed `enable` leaves no backup it created**:

- If the hard link succeeded (identity `(dev, ino)` known) and removing the temporary file or
  the directory fsync then fails, `publish_backup` removes the backup it just created under the
  same guard as `discard_created_backup`: only when the backup name is still a regular file
  (`symlink_metadata`, no follow) whose `(dev, ino)` equals the created identity. Then it
  best-effort removes its own temporary file again and returns the original error
  (`Failed to write PAM file atomically '<backup>': <err>`).
- If that guarded removal itself fails or the identity no longer matches, the error message
  gets the suffix `; the PAM backup '<backup>' created by this run was left in place` (mismatch
  wording: `...was replaced and left in place`). Same note style as `discard_created_backup`.
- `AlreadyExists` on the link (`Ok(None)`) is unchanged: an existing backup is never touched.
- No change to the success path, to `discard_created_backup`, or to the pre-rename re-check.

To make the post-link failure testable without root or fault injection in the kernel, the
post-link steps go through a test seam:

```rust
/// Post-link steps of [`publish_backup`] (remove the temporary file, fsync the directory);
/// replaced in tests to inject a failure.
type PostLinkHook<'a> = &'a mut dyn FnMut(&Path /* tmp */, &Path /* dir */) -> std::io::Result<()>;
fn publish_backup_with(backup, bytes, mode, owner, post_link: PostLinkHook) -> Result<Option<(u64, u64)>, AdminCliError>;
```

`publish_backup` calls `publish_backup_with` with the production steps. `pub(crate)` or private
with the tests in the `enable_race_tests` module.

### 2.3 Stale comment (issue item 3)

`tests/invariants/src/gdm_hardening_flaky_tests_contract.rs` GHF7 doc comment: replace
"through `notify_ready_stamped`" with "through `sd_notify::notify_ready()` (the stamped variant
since OA-4)". Comment-only; no assertion or needle changes.

## 3. Acceptance Criteria → Matrix

| ID | Criterion | Test |
|---|---|---|
| GBP1 | A stale `.<backup>.soos-tmp-<pid>-0` (and the legacy `.<file>.soos-tmp-<pid>`) present before `gdm enable` does not make it fail; the stale file is left byte-identical; no new `soos-tmp` file remains after success | `enable_race_tests` (unit, admin-cli lib) |
| GBP2 | Same for the PAM-file temporary name in `write_atomic_checked` (stale name present ⇒ enable succeeds, stale file untouched) | `enable_race_tests` |
| GBP3 | Post-link failure injected through the seam ⇒ `enable` returns `Failed to write PAM file atomically`, the backup created by this run is removed, the PAM file is byte-identical, no temporary file of this run remains; a pre-existing backup is never removed (link returned `AlreadyExists`, seam not reached) | `enable_race_tests` |
| GBP4 | Post-link failure + backup replaced inside the seam ⇒ the replacement is left byte-identical and the error carries the `was replaced and left in place` suffix | `enable_race_tests` |

`create_temp_sibling` exhaustion (16 stale names) ⇒ error, nothing removed: unit test in the
same module (part of GBP1).

## 4. Invariants

- Fail closed: every new path returns an error; nothing reaches `PAM_SUCCESS`; no PAM code
  touched.
- No `unwrap`/`expect` in production code; no `unsafe`; no new dependency.
- Never remove or modify a file this run did not create (stale temporary files, foreign
  backups).
- Existing tests unchanged (the comment in item 3 is not a test change).

## 5. ADR

Amend ADR 2026-10-05 GitHub #333 entry with: "(GitHub #335) Temporary files are named
`.<file>.soos-tmp-<pid>-<n>` and created with `create_new`, retrying up to 16 fresh names, so a
stale file left by a crashed run with a reused PID never blocks `enable` and is never removed.
A failure after the backup hard link (temporary-file removal or directory fsync) removes the
backup this run created under the `(dev, ino)` guard, so a failed `enable` leaves no new backup."

## Revision 1 (plan evaluator round 1) — supersedes the conflicting parts of §2.1, §2.2, §3

1. **Guard (MAJOR 1).** A post-link failure in `publish_backup` is handled by the existing
   `discard_created_backup(pam_file, backup, identity, err)`, unchanged: the backup is kept when
   the re-read PAM file holds a soos managed block (a concurrent `enable` won and relies on it)
   or cannot be re-read, and is removed only when it is still the regular file with exactly
   `identity`. `publish_backup` therefore takes `pam_file` as an extra argument (or returns the
   identity together with the post-link error so that the caller invokes
   `discard_created_backup`; developer's choice, behaviour identical). The §2.2 custom guard and
   its custom suffix are dropped.
2. **Seam reachable from enable (MAJOR 2).** New private function
   `ensure_gdm_pam_line_with_hooks(pam_file, before_rename: &mut dyn FnMut(), post_link: &mut dyn FnMut(&Path, &Path) -> std::io::Result<()>)`.
   `ensure_gdm_pam_line_with(pam_file, before_rename)` keeps its exact signature and delegates
   with the production post-link steps (remove own temporary file, fsync the directory), so
   `enable_with` and every GHF1 test are unchanged. The post-link hook receives
   `(own_tmp_path, dir)` and runs instead of the production steps; tests may perform those steps
   themselves and then return an injected error.
3. **Error text (minor 3).** No new suffix: GBP4 asserts the existing
   `discard_created_backup` note `could not be removed: it was replaced`.
4. **Stale temporary files (minor 4, 7).** Today both `publish_backup` (it removes `tmp` after
   any link outcome, including after `create_new` failed on a stale file) and
   `write_atomic_checked` can delete a stale file they did not create. With
   `create_temp_sibling`, both remove only the path the helper returned. The `AlreadyExists`
   fast path (`symlink_metadata(backup)` finds a backup) is unchanged and returns before any
   temporary file is created; the link-time `AlreadyExists` (race) removes this run's own
   temporary file only.
5. **`gdm restore` (minor 5).** `restore` also uses `write_atomic_checked`; add GBP5: a stale
   `.<file>.soos-tmp-<pid>` / `-0` present ⇒ `restore` succeeds and leaves the stale file
   byte-identical.
6. **Docs (minor 6).** One sentence in `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 next to the #333
   backup text: temporary names, stale files never removed, a failed `enable` leaves no new
   backup unless a concurrent `enable` relies on it.
7. **Exhaustion test (minor 8).** With 16 stale names (`-0`..`-15`) present, the operation
   fails with `Failed to write PAM file atomically` (I/O `AlreadyExists` in the message), the
   PAM file and all 16 stale files are byte-identical, and no backup is created.

### Revised acceptance table

| ID | Criterion |
|---|---|
| GBP1 | Stale legacy `.<backup-name>.soos-tmp-<pid>` and `-<pid>-0` present ⇒ `enable` succeeds; stale files byte-identical; no `soos-tmp` file of this run remains. Exhaustion (16 stale backup temp names) ⇒ error, nothing removed, no backup. |
| GBP2 | Same for the PAM-file temporary name (`write_atomic_checked`) during `enable`. |
| GBP3 | Post-link hook returns an error (no managed block in the PAM file) ⇒ `enable` returns `Failed to write PAM file atomically`, the backup created by this run is removed, the PAM file is byte-identical, no temporary file of this run remains. |
| GBP3b | Post-link hook writes a soos managed block into the PAM file, then returns an error ⇒ the backup is kept byte-identical (R2-2 rule). |
| GBP4 | Post-link hook replaces the backup, then returns an error ⇒ the replacement is byte-identical and the error contains `could not be removed: it was replaced`. |
| GBP5 | Stale temporary names present ⇒ `gdm restore` succeeds; stale files byte-identical. |

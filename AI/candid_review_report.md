# Candid Review Report

- **Date**: 2026-10-05
- **Target Branch**: `fix/gdm-backup-publish-cleanup`
- **Base (merge-base)**: `7d98021`
- **Reviewed-Diff-Fingerprint**: `22b2dc480b3b8ba0ac4424c9e93c6d00a60cb9007830d972b2204bee675d5d20`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_gdm_backup_publish_cleanup.md`, `AI/auditor_constraints_gdm_backup_publish_cleanup.md`, `AI/tester_contract_gdm_backup_publish_cleanup.md`, `AI/walkthroughs/181_gdm_backup_publish_cleanup.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`, `crates/admin-cli/src/gdm.rs`, `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs`

## 1. Executive Summary

The diff closes the three items of GitHub #335:

1. `publish_backup` post-link failure path (temporary-file removal or directory fsync after the
   hard link) now hands the backup created by this run to the existing `discard_created_backup`
   (dev/ino guard, kept when the PAM file holds a managed block or cannot be re-read), and the
   PAM-file write is never reached.
2. Temporary names are now `.<file>.soos-tmp-<pid>-<n>` created with `create_new` and up to
   `MAX_TEMP_NAME_ATTEMPTS` = 16 attempts (`create_temp_sibling`), used by both `publish_backup`
   and `write_atomic_checked`; stale files are never removed or modified.
3. The GHF7 doc comment in the invariants file now names `sd_notify::notify_ready()`
   (comment-only change).

All changes are confined to the root-only admin CLI (`crates/admin-cli`); no PAM module, IPC or
daemon code is touched. `cargo clippy -p soos-admin-cli --all-targets -- -D warnings`,
`cargo fmt --check`, the 8 new `gdm::backup_publish_tests`, the other `gdm` tests and the
`gdm_hardening` invariants pass locally. One MINOR documentation finding; no CRITICAL or MAJOR
finding.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs` (doc-comment
  lines 223-224 only; no assertion, attribute or code changed; required by #335 item 3).
- Removed/changed assertions (`^-.*assert|#[test]|proptest!|should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): **none**.
- Inline test modules: one new module `gdm::backup_publish_tests` (added only; 8 tests GBP1-GBP5).
  No existing inline test module line is modified or removed.
- Production removals in `gdm.rs` are limited to the bodies/signatures of `publish_backup`,
  `link_new_file`, `write_atomic_checked`, `write_and_rename` (temp-file creation moved into
  `create_temp_sibling`).

## 3. Deep Reasoning Audit

### Logic & Architecture

- Stale legacy `-<pid>` file with reused PID: new names carry a `-<n>` suffix, so no collision
  (GBP1/GBP2/GBP5 plant the legacy name, the `-0` name, and both) → PASS.
- Stale `-<pid>-0`: `create_new` returns `AlreadyExists`, loop moves to `-1`, stale file untouched
  (no open, no truncation, no symlink follow thanks to `O_EXCL`) → PASS.
- All 16 names taken: `AlreadyExists` returned before any write, nothing removed, no backup
  (GBP1 exhaustion test) → PASS; bound is a constant, loop is finite.
- Post-link failure when remove(tmp) fails: `remove_own_file` retries removal only while the name
  is still this run's inode (backup and tmp share the inode after `linkat`), then
  `discard_created_backup` → PASS.
- Post-link failure when dir fsync fails: tmp already gone (NotFound ignored), backup discarded
  under dev/ino guard → PASS.
- Concurrent `enable` winning between link and post-link failure: `discard_created_backup` keeps
  the backup when the re-read file holds the managed block (GBP3b) → PASS.
- Backup replaced concurrently: left alone, error note "it was replaced" (GBP4) → PASS.
- Pre-existing backup: fast-path returns `None` before any temp file, post-link hook not reached
  (GBP3) → PASS. Concurrent backup appearing between fast path and link: `Ok(None)` removes only
  this run's temp name → PASS.
- `write_and_rename` now fsyncs `parent_dir(target)`, which is byte-for-byte the same computation
  as the removed inline `dir` in `write_atomic_checked` → PASS.
- Error message text unchanged ("Failed to write PAM file atomically '<backup>': ...") → PASS.
- No scope creep beyond #335.

### PAM Concurrency & Deadlines

- No file under `crates/pam` changed; the admin CLI path performs a bounded number of syscalls
  (≤ 16 open attempts) → PASS (not applicable to PAM runtime).

### Panic Safety & Fail-Closed

- New production code contains no `unwrap`/`expect`/`panic!`/indexing; `unwrap_or_default` on the
  file name is pre-existing. Every error path returns `Err`; no path converts an error into a
  successful enable (post-link failure now fails before the PAM write) → PASS.

### Test Integrity & Anti-Weakening

- New tests would fail against plausible wrong implementations: keeping the legacy name (GBP1/2/5
  red), removing stale files (`assert_stale_untouched` checks inode and bytes), leaving the backup
  after post-link failure (GBP3), removing a replaced backup (GBP4), removing the backup despite
  a winner's managed block (GBP3b), unbounded retry or removal on exhaustion (GBP1 exhaustion).
- `assert_hook_args` checks the hook runs right after the hard link (same inode) → PASS.
- No existing assertion modified → PASS.

### Memory, Bounds & Secrets

- Temp files created `0600` with `O_EXCL`, then `fchown`/`chmod` on the handle before linking or
  renaming (unchanged ordering). No new log output; no biometric data involved. Removal of
  non-own files is impossible: unguarded `remove_file(&tmp)` calls only target a name this call
  created exclusively in a root-owned directory (same residual as before) → PASS.

### Supply Chain & Automation

- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change → N/A.

### English-Only Policy

- Code, comments, docs and walkthrough are in English → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/VERIFICATION_MATRIX.md` (row `GBP3` of the new `gdm-backup-publish-cleanup`
  table) — the row has only three cells; the `Status` column (`✅ Verified`) is missing, unlike
  every other row. Add the status cell.
- **[SUGGESTION]** `AI/architect_spec_gdm_backup_publish_cleanup.md` / tester contract — they
  still mention an `enable_race_tests` location and a "was replaced and left in place" suffix,
  while the implementation uses `backup_publish_tests` and the existing "could not be removed: it
  was replaced" note. Phase artifacts only; optional alignment.

## 5. Final Verdict

**VERDICT: APPROVED**

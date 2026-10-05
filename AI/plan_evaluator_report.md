# Plan Evaluation Report (round 2)
- **Date**: 2026-10-05
- **Issue**: GitHub #333 — fix: gdm PAM file hardening, enable/status agreement and flaky tests (GitHub-only, no backlog id)
- **Branch**: `fix/gdm-hardening-and-flaky-tests`
- **Base commit**: `3a1abd9`
- **Spec evaluated**: `AI/architect_spec_gdm_hardening_flaky_tests.md` with "Revision 1" (answers to the 9 round-1
  findings). Owner approvals OA-1/2/3 are final and are not re-litigated; they were re-checked for precision and for
  leaving the contract no weaker.

## 1. Coverage Matrix

| Acceptance line (issue #333) | Spec element | Status |
|---|---|---|
| Item 1: re-check before the atomic rename | §4.1 `ensure_gdm_pam_line_with`, `write_atomic_checked`, `ensure_pam_file_unchanged(.., Enable)` (dev, ino, bytes, mode, uid, gid), `linkat` backup, E1/E1b, GHF1 | Mapped (R2-1, R2-2) |
| Item 2: type check and read on one descriptor | §4.2 `O_NOFOLLOW \| O_NOCTTY \| O_NONBLOCK \| O_CLOEXEC`, `fstat`, GHF2 | Mapped |
| Item 3: total include/read budget | §4.3 `ScanBudget`, `MAX_PAM_STACK_READS` = 32, E3, GHF3 | Mapped |
| Item 4: enable/status agreement + `jump_skips_anchor` status test | §4.4 E4, qualified agreement invariant, §7, GHF4 | Mapped (R2-3) |
| Item 5: target of a shared `[success=N]` rule | §4.5 `jump_lands_in_stack` (unknown type ⇒ malformed), E5, OA-1, GHF5 | Mapped |
| Item 6: `%q` in the timeout line | §4.6, GHF6 | Mapped |
| Item 7: systemd journal order flake | §4.7.3 `notify_to_stamped` / `notify_ready_stamped`, value in message text; §4.7.4 SGR strip + anchored parse, OA-2, GHF7 | Mapped |
| Item 8: zero-timeout clamp flake | §4.8 both timeout sites, `None` = accepted + `UnexpectedEof`, bounded retry, OA-3, GHF8 | Mapped |
| Docs / ADR / walkthrough 180 | §8, GHF9, Revision 1 notes | Mapped (R2-3 wording) |

## 2. Round-1 Findings — Resolution Check

| # | Round-1 finding | Revision 1 change | Verified |
|---|---|---|---|
| 1 MAJOR | ANSI styling breaks the field parse | Value moved into the message text (two exact shapes, two `info!` calls); harness strips SGR (`sed -e 's/\x1b\[[0-9;]*m//g'`, GNU sed in the ubuntu image) then parses `(ready_sent_monotonic_us=<digits>)`; `unknown`/missing fails; invariant needles for both message shapes, the parse and the strip | Resolved. The message is written by the default visitor as plain `{:?}` of `fmt::Arguments` (the existing harness already greps message text successfully); field-name styling no longer matters; the strip is a correct defence in depth. Implicit `{us}` capture works in `tracing` macros (`format_args!`). Rust needle escapes (`\\(`, `\\x1b\\[`) match the harness text literally |
| 2 | Env-reading test seam | `notify_to_stamped(Option<&OsStr>, &str)` on the existing `notify_to` seam; clock read after validation and socket set-up, right before `send_to_addr` | Resolved |
| 3 | Backup removal could delete another run's backup | `create_new` temp + `fs::hard_link` (`linkat`, flags 0: never replaces, does not follow a symlink at the destination ⇒ `EEXIST`), `(dev, ino)` from the temp descriptor, removal only of that inode | Resolved for the scenario raised; a reversed-role variant remains (R2-2) |
| 4 | Overclaim, metadata ignored | Residual window stated in §4.1, §5, ADR; enable also compares mode/uid/gid | Resolved, but the mode masking is ambiguous (R2-1) |
| 5 | Agreement false for a symlinked file | §4.4 qualified to a regular, readable UTF-8 file; divergence documented | Resolved in §4.4; ADR wording still says "exactly" (R2-3) |
| 6 | libpam jump mechanism | §4.5.1 / ADR: the jump sets no result; success comes from the target and later rules | Resolved (see observation O-1) |
| 7 | Unknown type keyword | Unknown type ⇒ malformed ⇒ E5 | Resolved |
| 8 | OA-3 precision | Both timeout sites named (`test_pam.rs:378`, `:403`); `None` = accepted then `UnexpectedEof` on prefix or body; probability indicative only | Resolved |
| 9 | `O_NOCTTY` | On the enable open, the snapshot re-read and the backup reader | Resolved |

## 3. Facts Re-verified Against Code

| Fact | Location | Actual | Match |
|---|---|---|---|
| Enable written mode is `st_mode & 0o7755` (group/world write dropped) | `crates/admin-cli/src/gdm.rs:275` | yes | Yes (relevant to R2-1) |
| No existing test runs `gdm enable` on a group- or world-writable GDM file | grep `0o66*`/`0o77*` in `crates/admin-cli/tests/gdm*.rs`, `gdm.rs` | only 0o640 / 0o644 fixtures | — (R2-1 would go undetected) |
| Temp names cannot collide between backup and PAM file | `write_atomic_checked`: `.{target_file_name}.soos-tmp-{pid}` | distinct per target | Yes |
| `notify_to` seam exists | `crates/daemon/src/sd_notify.rs:73` | yes | Yes |
| Harness reads `journalctl -o cat` in the ubuntu container (GNU sed `\x1b`) | `tests/docker/systemd_unit_acceptance_test.sh:347-353` | yes | Yes |
| Invariant needle to replace | `tests/invariants/src/systemd_unit_acceptance_contract.rs:290` | as cited | Yes |
| Clock domains (OA-2) | `pipeline.rs:157` `CLOCK_MONOTONIC`; systemd `ActiveEnterTimestampMonotonic` = `now(CLOCK_MONOTONIC)` µs; Docker: no time namespace by default, PID 1 and daemon share one anyway | same domain, `floor(sent) <= floor(active)` by causality | Yes |

## 4. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Scenario: a symlink or FIFO is planted at `gdm-password` or at the backup path. The single descriptor refuses it on `ELOOP`/`fstat`; `linkat` on a planted backup symlink fails with `EEXIST`, so the run does not count the backup as its own, does not follow the symlink, and `gdm restore` later refuses the symlinked backup. Residual windows (re-check → `rename`, `lstat` → `unlink` of the backup) are stated and root-only.
- Result: PASS (MINOR R2-1, R2-2).

### Pillar 2 — PAM deadline & concurrency
- Scenario: OA-3 masks a never-write regression. Twenty `None` attempts end in a panic; a wrong clamp still fails the unchanged window assertion. `crates/pam` and `test_pam.rs` are untouched.
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Scenario: a 33-include stack, a jump off the file, an unknown type keyword in the jump span, a pre-anchor skip. Each refuses in `enable` and reports `installed: false` in `status`; budget arithmetic is checked. A clock error in the daemon only changes the log text, never the send.
- Result: PASS.

### Pillar 4 — Dependencies
- No new crate (`libc` flags, `fs::hard_link` from std).
- Result: PASS.

### Pillar 5 — Data confidentiality
- One start-up monotonic timestamp in a log message; no request, frame or key data.
- Result: PASS.

### Pillar 6 — Test integrity
- OA-1: the assertion is unchanged and the fixture becomes a stack libpam can run (target `required pam_env.so` in the file); the original bad-jump shape is a new E5 negative test. OA-2: the new needles strengthen the invariant (the parse, the strip and both message shapes are pinned); `sent <= active` fails on a `Type=simple` regression. OA-3: precise and bounded. No other existing test changes.
- Result: PASS (R2-4 lists tests to add so the new behaviour has failing power).

## 5. Findings (round 2)

- **R2-1 [MINOR] Mode comparison vs stored mode is ambiguous.** §4.2 says the snapshot "derives `mode & 0o7755`" while §4.1 compares "full `st_mode & 0o7777`". If the snapshot stores the masked value and the re-read is compared unmasked, every group- or world-writable `gdm-password` (e.g. `0664`) fails with E1 on every `gdm enable`. No existing fixture uses such a mode, so the tests would not catch it. **Required (developer/tester, no re-evaluation needed)**: `PamFileSnapshot.mode` holds `st_mode & 0o7777` from the `fstat`; the `0o7755` mask applies only when the written mode is derived. The tester adds an enable test on a `0o664` file (succeeds, written mode `0o644`) and a hook test where `chmod` in `before_rename` produces E1.
- **R2-2 [MINOR] A reversed-role concurrent-enable variant still loses the backup.** A and B read the same pristine file; A links the backup (`Some`), B gets `EEXIST` (`None`); B renames first (its re-check passes); A's re-check fails (B changed the file) and A removes *its* inode, which is the backup B relies on. B's managed block is then left without a backup. This needs two simultaneous root `gdm enable` runs, and the consequence is availability of `gdm restore` only. Either skip the removal when the re-read PAM file now contains a soos managed block (someone else enabled), or add this case to the stated residual in §4.1/§5/ADR.
- **R2-3 [MINOR] ADR overclaims agreement.** ADR item (3) says "`enable` succeeds exactly when the returned status is installed". §4.4 only guarantees the direction enable `Ok` ⇒ installed, plus the converse for a regular readable file (a symlinked file is `installed: true` in status while enable refuses with E2). Reword to match §4.4. In the same spirit, §8's `Docs/DAEMON.md` line still says "`ready_sent_monotonic_us` field"; it is now message text (the Revision 1 note already says so: align the §8 bullet).
- **R2-4 [MINOR] Test-hook list should cover the new backup and metadata branches.** Add to §7: Insert with a **pre-existing** backup + E1 ⇒ backup byte-identical and same inode (`created_backup = None`); a backup path holding a symlink ⇒ `EEXIST` path, symlink untouched; a backup replaced in the hook ⇒ not removed, error carries the E1b `it was replaced` suffix; and the R2-1 mode cases.
- **O-1 (observation, no change required)**: §4.5.1 says `success=done` "returns at once with the soos success". In libpam `done` stops only when the impression is positive (a failed earlier `required` keeps it negative and evaluation continues). The conclusion "a jump never grants more than `success=done`" is unaffected.

## 6. Verdict
VALIDATION_VERDICT: APPROVED

Round-1 MAJOR finding 1 and MINOR findings 2–9 are resolved. No CRITICAL or MAJOR finding remains. R2-1 to R2-4 are MINOR precision items for the tester and developer phases: R2-1 (store the unmasked mode, test a `0o664` file) must be honoured by the implementation and the auditor should check it.

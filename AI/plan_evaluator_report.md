# Plan Evaluation Report (Round 2)
- **Date**: 2026-10-05
- **Issue**: GitHub-only — fix: gdm backup publication cleanup and stale test comment (GitHub #335; follow-up of #333/#334)
- **Branch**: `fix/gdm-backup-publish-cleanup`
- **Base commit**: `7d98021`
- **Spec**: `AI/architect_spec_gdm_backup_publish_cleanup.md` (§1–§5 + "Revision 1")

## 1. Coverage Matrix
| Acceptance line / TDD test | Spec element | Status |
|---|---|---|
| #335 item 1: consistent post-link failure | Rev1 item 1 (reuse `discard_created_backup`), item 2 (hook seam), GBP3, GBP3b, GBP4 | Covered |
| #335 item 2: collision-resistant temporary name | §2.1 `create_temp_sibling`, Rev1 items 4, 7; GBP1, GBP2, GBP5 | Covered (enable backup, enable PAM file, restore) |
| #335 item 3: GHF7 doc comment | §2.3 | Covered (comment-only) |
| ADR | §5 amendment | Covered; wording needs the R2-2 exception (Finding R2-2) |
| Docs | Rev1 item 6 (`Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1) | Covered |

## 2. Round-1 Findings Resolution
| Round-1 finding | Revision 1 response | Status |
|---|---|---|
| 1 MAJOR — guard omits managed-block re-read | Post-link failure goes through the unchanged `discard_created_backup` (managed block / unreadable ⇒ keep; identity ⇒ remove; replaced ⇒ note); GBP3b added | Resolved |
| 2 MAJOR — seam not reachable from enable | New private `ensure_gdm_pam_line_with_hooks`; `ensure_gdm_pam_line_with` signature unchanged ⇒ `enable_with` and all GHF1 tests untouched | Resolved |
| 3 MINOR — invented suffix | GBP4 asserts existing `could not be removed: it was replaced` (`gdm.rs:406-412, 433`) | Resolved |
| 4 MINOR — `publish_backup` also deletes stale temp files | Rev1 item 4 states both sites; both remove only the helper's returned path | Resolved |
| 5 MINOR — `gdm restore` untested | GBP5 added | Resolved |
| 6 MINOR — no doc update | Rev1 item 6 | Resolved |
| 7 MINOR — fast path vs link-time `AlreadyExists` | Rev1 item 4 distinguishes them; link-time race removes own temp only | Resolved |
| 8 MINOR — exhaustion assertions | Rev1 item 7 pins error text, 16 stale files and PAM file byte-identical, no backup | Resolved |

## 3. Facts Verified Against Code (round 2 deltas)
| Fact | Code location | Actual value | Match |
|---|---|---|---|
| `discard_created_backup(pam_file, backup, identity, err)` signature and behaviour | `gdm.rs:400-437` | As stated in Rev1 item 1 | Yes |
| Note text on replaced backup | `gdm.rs:408-411`, `gdm.rs:433` | `... could not be removed: it was replaced` | Yes |
| `ensure_gdm_pam_line_with(pam_file, &mut dyn FnMut())` used by `enable_with` | `gdm.rs:271-274` | Signature kept by Rev1 item 2 | Yes |
| `write_atomic_checked` restore caller | `gdm.rs:780` | Covered by GBP5 | Yes |
| Restore's `leftover_temp_files` filters `soos-tmp` | `gdm.rs:1116-1123` | Existing restore tests have no stale file planted ⇒ still empty | No breakage |
| Error prefix used when `publish_backup` fails | `gdm.rs:362` | `Failed to write PAM file atomically '<backup>': <io err>` | Yes (GBP1 exhaustion, GBP3) |

## 4. Pillar Analysis
### Pillar 1 — Architecture & threat model
- Scenario: concurrent enable B wins after A's link; A's post-link step fails. With Rev1, `discard_created_backup` re-reads `gdm-password`, sees B's managed block and keeps the backup (GBP3b simulates this). Residual (replacement between identity check and unlink) is the already-documented #333 residual.
- Result: PASS.
### Pillar 2 — PAM deadline & concurrency
- No `crates/pam` change; ≤ 16 opens per temp site. PASS.
### Pillar 3 — Panic safety & fail-closed
- Exhaustion ⇒ error, nothing removed; every new path returns `Err`; no `unwrap`/`expect`; bounded `0..16` loop (no arithmetic under `arithmetic_side_effects = "warn"` + `-D warnings`). PASS.
### Pillar 4 — Dependencies
- No new crate. PASS.
### Pillar 5 — File safety
- `create_new` never follows a symlink at a temp name; callers delete only their own returned path; stale files and foreign backups are never removed. PASS.
### Pillar 6 — Test integrity / power
- GBP3 through `ensure_gdm_pam_line_with_hooks` fails against: the current code (backup left behind), a "treat as done" implementation (backup left), and a guard-less removal (GBP3b and GBP4 catch it). GBP1/GBP2/GBP5 fail today (`create_new` hits `AlreadyExists` on the `-<pid>` name? — note: today's name is the legacy `-<pid>` form, so the legacy stale file is the red case; the `-0` stale file is red only against an implementation that does not retry). Exhaustion catches a non-retrying or unbounded implementation. Existing tests unchanged.
- Result: PASS, with clarifications below.

## 5. Findings
1. **[MINOR]** GBP3 "no temporary file of this run remains": Rev1 item 2 lets the hook replace the production steps, so if the injected hook returns an error without removing the temp file, only the §2.2 "best-effort removes its own temporary file again" step guarantees that assertion. Rev1 does not restate it. — Keep §2.2's best-effort removal of the returned temp path after a hook error (ignore `NotFound`), and have GBP3 inject one hook that does *not* remove the temp file.
2. **[MINOR]** §5 ADR text still says "so a failed `enable` leaves no new backup"; after Rev1 item 1 it should add "unless the re-read PAM file holds a soos managed block (a concurrent `enable` relies on it) or cannot be re-read", matching Rev1 item 6's doc sentence.
3. **[MINOR]** GBP3/GBP3b/GBP4 should also assert the `before_rename` hook ran 0 times (the failure occurs before the PAM-file write), which keeps "PAM file byte-identical" meaningful against an implementation that swallows the post-link error and continues.
4. **[MINOR]** Make GBP1/GBP2/GBP5 plant the legacy `-<pid>` name in at least one case on its own: that is the case that fails today (red phase); `-0` alone is red only against a non-retrying new implementation.

## 6. Verdict
No CRITICAL or MAJOR findings remain; the four MINOR items are clarifications the tester and developer apply without another architect round.

VALIDATION_VERDICT: APPROVED

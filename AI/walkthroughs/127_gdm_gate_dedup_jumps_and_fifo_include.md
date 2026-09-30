# Walkthrough 127 — GDM Gate De-duplication With Jumps and FIFO Include Targets

- **Date**: 2026-09-30
- **Issue**: GitHub #278 ([STO-FU] follow-ups of the round-3 candid review of the P1 storage/vision batch)
- **Branch**: `fix/p2-storage-followups`
- **Matrix criteria**: SFU1, SFU2, SFU3 (new, ✅ Verified); SFU4, SFU5 (new, ⏳ Pending)
- **ADR**: 2026-09-30 "GDM Gate De-duplication With Jumps and Non-Blocking PAM Reads" in `AI/DECISIONS.md`

---

## 1. Context & Objectives

Issue #278 lists four follow-ups. Two are code defects in `soos-admin gdm enable`, both
reachable only through a non-standard `/etc/pam.d` written by root and neither fail-open by
default:

1. `crates/admin-cli/src/gdm.rs` dropped a gate copied from a delegated stack when the edited
   file already ran the same gate with a plain `required`/`requisite` control, even when an
   earlier `[success=N ...]` jump could skip that earlier copy. Example: a VIP user matched by
   `pam_succeed_if.so user ingroup vip` jumps over `requisite pam_nologin.so`, and the managed
   block then lets a face success bypass `nologin`.
2. `crates/admin-cli/src/pam_stack.rs` `read_bounded_utf8` used `File::open`, so a FIFO
   include target (no writer) blocked `open(2)` and `gdm enable` forever.

The other two (renaming the legacy `.webp.enc` suffix, evaluating a lighter embedding model)
are blocked by test integrity and by model/data availability respectively (section 4).

## 2. Design

- `plan_gdm_enable` already collects every pre-anchor jump for the "jump would change
  target" refusal. It now records `has_jump = !jumps.is_empty()` and, when true, uses an empty
  `already_present` list: every delegated gate is copied. Running a gate twice is harmless;
  computing exactly which rules each jump skips was rejected as needless complexity.
- `read_bounded_utf8` opens with `O_NONBLOCK | O_CLOEXEC`, calls `fstat` through the open
  descriptor and refuses a non-regular file (`not a regular file (FIFO, device, socket or
  directory)`) before reading. Symlinks stay followed: libpam follows them and authselect
  ships `/etc/pam.d/system-auth` as a symlink. The bounded read (`MAX_PAM_FILE_BYTES`) and the
  UTF-8 check are unchanged; `O_NONBLOCK` has no effect on regular-file reads.

## 3. Tests (TDD)

New suite `crates/admin-cli/tests/gdm_followup_tests.rs` (no pre-existing test touched):

| Test | Red on origin/main | Green |
|---|---|---|
| `test_gdm_enable_keeps_a_duplicate_gate_when_a_jump_can_skip_the_earlier_copy` | managed block held only `pam_soos.so` | nologin copied before `pam_soos.so` |
| `test_gdm_enable_refuses_a_fifo_include_target_without_blocking` | `gdm enable blocked on a FIFO include target for 10s` | refused immediately, file unchanged, no backup |
| `test_gdm_enable_still_follows_a_symlinked_regular_include_target` | passes (regression guard) | passes |

The existing `gdm_stack_order_tests::test_gdm_enable_does_not_duplicate_an_earlier_enforcing_gate`
still passes: without a jump, de-duplication is unchanged.

## 4. Not Done (Needs a Decision or Hardware)

- **`.webp.enc` rename (SFU4)**: `crates/evidence-store/tests/permissions_tests.rs`
  `test_evidence_filename_format` asserts `filename.ends_with(".webp.enc")` and slices the
  suffix. Renaming `OPAQUE_SNAPSHOT_EXTENSION` would break a pre-existing contract, so the
  exact migration is proposed for user approval instead of being applied.
- **Lighter embedding model / recalibration (SFU5)**: needs real model files and a labelled
  face dataset; it cannot be measured hermetically.

## 5. Audit

No `unwrap`/`expect` added to production code; reads remain bounded; nothing sensitive is
logged; `soos-admin-cli` keeps `#![forbid(unsafe_code)]` (`custom_flags` is safe std API);
the PAM module is untouched.

## Integration: user-approved suffix migration (2026-09-30)

`OPAQUE_SNAPSHOT_EXTENSION` is now `.opaque.enc`; `permissions_tests::test_evidence_filename_format`
uses the constant instead of the `.webp.enc` literal. Retention works on dated directories and the
daily counters count `*.enc`, so existing `.webp.enc` files keep being counted and purged.

# Walkthrough 98 — GDM Enable Stack Order, Restore and Matrix Traceability

- **Date**: 2026-09-30
- **Issues**: Review findings STO-03 (GitHub #177) and STO-07 (GitHub #180) — **Branch**: `fix/gdm-enable-stack-order`
- **Matrix criteria**: GSO1–GSO9 (✅ Verified), GSO10 (real hosts, pending); ASG5, LSF1 and GEPU1 citations corrected
- **ADR**: 2026-09-30 "GDM PAM Stack Placement" in `AI/DECISIONS.md`

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Storage & CLIs):

- **STO-03 (MAJOR)**: `soos-admin gdm enable` inserted `auth  sufficient  pam_soos.so timeout_ms=2500`
  before the first `@include common-auth`, otherwise at index 0. On Fedora and Arch the rule became
  line 1, above `pam_selinux_permit.so` and the `substack password-auth` that runs
  `pam_faillock preauth`; on Ubuntu with the faillock profile it ran before `preauth` inside
  `common-auth`. A facial success could therefore bypass account lockout (and, on Arch,
  `pam_shells`/`pam_nologin` in `system-login`). The command also never checked that
  `pam_soos.so` is installed and had no restore action. (Atomic write and byte-exact
  `.soos-backup` were already fixed by walkthrough 91.)
- **STO-07 (MAJOR, doc drift)**: matrix rows ASG5/LSF1 cited six `gdm_tests` functions that did
  not exist (the `GDM_PAM_LINE` constant was added by walkthrough 91), GEPU1 named a `<uid>.bio`
  template file (the store writes `<uid>.cbor.enc`), and `soos-admin gdm` was undocumented.

Objective: face success never skips lockout, nologin, shell or user-filter gates; fail closed on
anything unexpected; restore path; matrix citations that resolve.

## 2. Architect Design

- `crates/admin-cli/src/pam_stack.rs` (new, crate-private, zero writes): `PamLine` parser
  (`@include`, `[-]type control module args`, bracket controls), `is_auth`, `delegation`,
  `module_name`, `max_jump`, `is_pre_credential`; `delegated_gates(dir, name)` scans a
  delegated stack (bounded by `MAX_PAM_INCLUDE_DEPTH` = 4 and `MAX_PAM_FILE_BYTES` = 64 KiB)
  and returns the plain `required`/`requisite` gates (`pam_faillock.so preauth`,
  `pam_nologin.so`, `pam_shells.so`, `pam_succeed_if.so`) met before the first credential,
  conditional or unknown rule.
- `crates/admin-cli/src/gdm.rs`:
  - `GDM_PAM_LINE` = `auth  [success=done default=ignore]  pam_soos.so timeout_ms=2500`;
    `LEGACY_GDM_PAM_LINE`; `GDM_BLOCK_BEGIN` / `GDM_BLOCK_END`; `PAM_MODULE_FILE`;
    `DEFAULT_PAM_MODULE_DIRS` (same list as `scripts/install.sh`); `find_pam_module`.
  - `plan_gdm_enable` (pure): strip the managed block and legacy/bare soos rules → pristine;
    leave an administrator-written `pam_soos.so` rule alone; anchor = first `auth` rule that is
    not a pre-credential rule; refuse without an anchor or when a `[...=N]` jump before it would
    change target; copy delegated gates (deduplicated against in-file rules); emit
    `BEGIN / gates / GDM_PAM_LINE / END` immediately before the anchor; `None` when unchanged.
  - `ensure_gdm_pam_line` writes the pristine content as backup (only if none exists), then the
    plan, atomically. `Enable` now edits the PAM file **before** removing `gdm.disable`.
  - `GdmAction::Restore` → `restore_gdm_pam_file`: regular-file backup only, bounded, atomic
    write with the backup's mode/owner, backup removed, directory fsynced.
- `args.rs`: `GdmAction::Restore`, `GdmArgs::pam_module_dir`. `main.rs`: `gdm enable` refuses
  unless `find_pam_module` finds `pam_soos.so` (library tests use temporary directories, so the
  check lives in the binary).

Control decision: `sufficient` is defined by Linux-PAM as
`[success=done new_authtok_reqd=done default=ignore]`; the explicit form is kept for
consistency with every packaged soos rule. `done` never overrides an earlier failed `required`
gate, so a copied `pam_faillock preauth` still fails a locked account after a face match.

## 3. Tester Contract (written first)

`crates/admin-cli/tests/gdm_stack_order_tests.rs` — fixtures copied from Ubuntu 24.04 (`gdm3`
`gdm-password`, stock and faillock `common-auth`), Fedora 40 (`gdm-password`,
`authselect local with-faillock` `password-auth`, `postlogin`) and Arch (`gdm-password`,
`pambase` `system-local-login`, `system-login`, `system-auth`). A test-side flattener expands
includes and asserts gate → `pam_soos.so` → `pam_unix.so` order on the evaluated auth stack.

| Test | Contract |
|---|---|
| `test_gdm_enable_ubuntu_inserts_after_nologin_and_succeed_if_before_common_auth` | exact position, original lines verbatim, no guard |
| `test_gdm_enable_ubuntu_faillock_profile_adds_preauth_guard_before_soos` | guard `auth  requisite  pam_faillock.so preauth` |
| `test_gdm_enable_fedora_never_inserts_at_top_and_guards_faillock_preauth` | after `pam_selinux_permit`, before `substack` |
| `test_gdm_enable_arch_guards_shells_nologin_and_faillock_from_nested_includes` | three nested levels, order kept, no `authfail`/`authsucc` |
| `test_gdm_enable_inline_faillock_inserts_after_preauth_before_pam_unix` | no duplicate preauth |
| `test_gdm_enable_refuses_a_stack_without_a_credential_anchor`, `..._when_a_jump_crosses_the_insertion_point`, `..._non_utf8_pam_file` | unchanged, no backup |
| `test_gdm_enable_migrates_a_misplaced_legacy_line`, `test_gdm_enable_is_idempotent_on_distribution_stacks`, `test_gdm_enable_leaves_an_admin_managed_soos_rule_untouched` | migration / idempotency |
| `test_gdm_restore_*` (3) | restore bytes+mode, no backup → error, symlinked backup refused |
| `test_find_pam_module_*` (2), `test_soos_admin_gdm_enable_refuses_when_module_is_not_installed` | module check, end to end through the binary |

Appended to `crates/admin-cli/tests/gdm_tests.rs` (existing tests unchanged): the names the
matrix already cited — `test_gdm_pam_line_includes_timeout_ms_2500`,
`test_gdm_status_unconfigured`, `test_gdm_status_configured_enabled`,
`test_gdm_status_configured_disabled`, `test_gdm_disable_creates_flag` — plus
`test_gdm_restore_and_module_dir_parsing`. Invariant
`test_matrix_gdm_references_resolve_to_real_tests` (`tests/invariants`).

Red evidence: first compile failure on the specified API only (`GdmAction::Restore`,
`pam_module_dir`, `find_pam_module`, block constants); with API stubs, 12 of 16
`gdm_stack_order_tests` failed on assertions and `test_gdm_pam_line_includes_timeout_ms_2500`
failed on the `sufficient` control; the invariant failed on
`gdm_tests::test_gdm_enable_idempotent`; the binary test failed with the module check disabled.
The status/disable tests passed immediately: they pin existing behavior that the matrix cited.

## 4. Auditor Constraints

1. No `unwrap`/`expect`/indexing/unchecked arithmetic in production (`saturating_*`, `get`).
2. Bounded I/O: 64 KiB per PAM file, 4 include levels, include names restricted to
   `[A-Za-z0-9._-]` not starting with `.` (no traversal, absolute targets not followed).
3. Fail closed with zero writes on every refusal; no lossy UTF-8 rewrite; symlinked PAM file
   or backup refused; group/world write bits never propagated.
4. Never rewrite rules the administrator wrote; never touch the account/session phases.
5. Copied gates must not weaken the password path: only plain `required`/`requisite` gates
   met before the delegated credential module; conditional gates end the scan.

## 5. Implementation Notes

- `plan_gdm_enable` operates on `split_inclusive('\n')`, so every original byte (tabs, trailing
  comments, missing final newline) is preserved outside the block.
- The pristine content is what the backup stores, so a file migrated from a legacy line gets a
  soos-free backup and `scripts/uninstall.sh` restores a clean stack.
- `tests/docker/pam_rollback_test.sh::simulate_gdm_enable` still simulates the pre-#177 line;
  the rollback assertions (byte-exact restore of the backup) are unaffected, so the existing
  Docker test is left unchanged.

## 6. Verification

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features --no-fail-fast
./scripts/candid_review.sh
```

All green (see the PR). GSO10 (real GDM login and lock-screen unlock on Ubuntu, Fedora and
Arch, locked faillock account refused with a matching face) remains a manual hardware check.

## 7. Documentation

- `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1: `soos-admin gdm status|enable|disable|restore`,
  options, the managed block and its placement rules; §7 rescue step uses `gdm restore`.
- `AI/ARCHITECTURE.md` §5 GDM paragraph; `AI/DECISIONS.md` ADR; project facts updated.
- `AI/VERIFICATION_MATRIX.md`: ASG5/LSF1 cite real tests, GEPU1 names `<uid>.cbor.enc`, new
  component `gdm-stack-placement`.

## 8. Follow-ups

- The invariant is scoped to the GDM suites: 15 other matrix citations (camera, preview,
  PAM info, timeout rows) still name functions that do not exist; a repository-wide check
  should be added once those rows are corrected by their owners.
- `soos-enroll import` is still missing from `Docs/ENROLLMENT_CLI.md` (`debug-vision` and
  `--mock` are documented); left to the GUI import work (GitHub #156), which changes that command.

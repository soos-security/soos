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

## 9. Rework — Candid Review 2026-09-30 (Finding 1, MAJOR; Finding 5, SUGGESTION)

**Defect.** `plan_gdm_enable` took as anchor "the first auth rule not on the pre-credential
allow-list", whatever it was, and only copied delegated gates when that rule delegated. The
reviewer reproduced two bypasses: `auth required pam_tally2.so ...` before `@include
common-auth` put the managed block at index 0, in front of the lockout module; `auth optional
pam_group.so` between `pam_nologin.so` and `@include common-auth` (whose stack runs
`pam_faillock.so preauth`) put soos in front of `pam_group` without copying the faillock gate.

**Design (ADR "GDM PAM Stack Placement", rules 1, 2 and 5 updated).**

- `pam_stack::CREDENTIAL_MODULES` (documented allow-list): `pam_unix.so`, `pam_sss.so`,
  `pam_ldap.so`, `pam_krb5.so`, `pam_winbind.so`, `pam_systemd_home.so`, `pam_fprintd.so`.
  `pam_gnome_keyring.so` is deliberately absent (it verifies nothing). `pam_systemd_home.so` is
  required: Arch `system-auth` runs it before `pam_unix.so`.
- Edited file: each auth rule is a delegation or credential module (anchor), a pre-credential
  rule (soos goes after it, jump check unchanged) or unclassified → `GdmConfig` error naming
  `'<module>' (control '<control>')`, returned by the pure planner before any write.
- Delegated scan (`delegated_gates(dir, &lines[anchor..])`): starts at the anchor so that, when a
  delegated stack ends without a credential module, the rest of the edited file is classified
  the same way. Per rule: delegation → recurse (≤ 4 levels); credential → stop; plain
  `required`/`requisite` gate (now also `pam_access`, `pam_listfile`, `pam_securetty`) → copied;
  `pam_env`/`pam_faildelay` → skipped; anything else (unknown module, conditional or
  `sufficient`/`optional` gate) → refuse. An absolute or otherwise unresolvable include target
  and a line continuation in a delegated file now refuse instead of silently ending the scan; a
  stack reaching no credential module at all refuses. A missing include file still ends the scan
  (Linux-PAM fails the whole stack then, so soos never runs).
- Scenario (b) is refused rather than placing soos after `pam_group` with the faillock gate
  copied: `pam_group` is unclassified, and the rule is "prefer refusal for unknown modules". The
  administrator resolves it as documented in `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 (migrate
  the module or write the `pam_soos.so` rule by hand, which `enable` never touches).
- Finding 5: `restore` now opens the backup once with `O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`,
  checks regular-file type and size on that descriptor (`fstat`) and reads it through the same
  descriptor, bounded by `take(MAX_PAM_FILE_BYTES + 1)`; mode and owner come from the same
  `fstat`. A swap between check and read is not reproducible deterministically in a test; the
  oversized-backup refusal is pinned by a new test and the existing symlink test still passes.

**Tests (added to `gdm_stack_order_tests.rs`; no existing test changed).** Red evidence before
the fix: 7 of the 8 new enable tests failed (`enable` succeeded and rewrote the file):

| Test | Contract | Red |
|---|---|---|
| `test_gdm_enable_refuses_unclassified_lockout_module_before_include` | scenario (a) refused, file/backup/flag unchanged | failed |
| `test_gdm_enable_refuses_unclassified_module_before_include_with_faillock` | scenario (b) refused | failed |
| `test_gdm_enable_refuses_unclassified_module_before_inline_credential` | unknown module before `pam_unix` | failed |
| `test_gdm_enable_refuses_unclassified_module_inside_delegated_stack` | unknown module in `common-auth` before `pam_unix` | failed |
| `test_gdm_enable_refuses_conditional_gate_inside_delegated_stack` | `[success=1 ...] pam_succeed_if` in `common-auth` | failed |
| `test_gdm_enable_refuses_an_unresolvable_include_target` | `auth include /etc/pam.d/common-auth` | failed |
| `test_gdm_enable_scans_past_a_gate_only_include` | tail of the file scanned after a gate-only include | failed |
| `test_gdm_enable_accepts_documented_credential_modules_as_anchor` | every credential module is an anchor, inline and delegated | passed (regression guard) |
| `test_gdm_restore_refuses_an_oversized_backup` | Finding 5 bound on the read descriptor | passed (regression guard) |

Matrix: new rows GSO11, GSO12 (✅ Verified); GSO2, GSO4 and GSO6 wording updated.

## 10. Rework — Minor Findings 2–4 (same review)

- **Finding 2**: `crates/daemon/src/dispatcher.rs` — `is_frame_fresh` has its doc comment back;
  `evidence_pixel_format` has its own. No behaviour change.
- **Finding 3**: the crate docs of `soos-enrollment-cli` and `soos-gui` no longer claim
  "anti-forensic secure erasure"; they state the erasure ADR (best-effort in-place overwrite,
  encryption at rest + master-key destruction as the guarantee).
- **Finding 4**: plaintext buffers are pre-sized so they never reallocate while holding
  secrets; see the test list in section 11.

## 11. Finding 4 — Pre-Sized Plaintext Buffers

A `Vec` that grows by reallocation frees its previous allocation without zeroizing it, even
when the final buffer is wrapped in `Zeroizing`. Each plaintext producer now reserves its
buffer once:

| Producer | Change | Test (red → green) |
|---|---|---|
| `EvidenceRecord::to_cbor` (`crates/evidence-store/src/snapshot.rs`) | a first `ciborium` pass into a `ByteCounter` sink (stores nothing) measures the exact size; `try_reserve_exact`, then the real encode | `cbor_presize_tests::test_evidence_to_cbor_reserves_the_exact_size_and_never_reallocates` (red: capacity 268 vs length 135) |
| GUI import JSON (`crates/gui/src/privileged.rs`) | new `embedding_json`: counting `serde_json::to_writer` pass, exact reservation in `Zeroizing<Vec<u8>>`, second `to_writer`; `import_template_with` uses it | `import_json_presize_tests::test_embedding_json_is_presized_exactly_and_roundtrips` (red: capacity 128 vs length 2) |
| `soos-enroll import` JSON (`crates/enrollment-cli/src/service.rs`) | new `decode_json_embedding`: a `serde` visitor pushes into a `Zeroizing<Vec<f32>>` reserved once at `IMPORT_EMBEDDING_DIM` (now `pub`), counts but never stores extra values, rejects trailing bytes; the dimension check uses the full count, so the error message is unchanged | `import_json_presize_tests::test_import_json_decodes_into_a_buffer_that_never_grows`, `..._counts_but_never_stores_values_beyond_the_dimension` (red: capacity 0/513 vs 512), `..._rejects_non_float_arrays` |

The observable contract is `capacity == len` (exact reservation, no growth), which a
reallocating implementation cannot satisfy for the tested sizes. The red runs used stubs that
kept the previous `to_vec` / `from_slice::<Vec<f32>>` behaviour behind the new function
names. The counting pass costs one extra serialization walk (no copy of the payload); the
input to `import` is already bounded to `MAX_IMPORT_INPUT_BYTES` (64 KiB) before parsing.

Gate for the rework: `cargo fmt --all -- --check`, `cargo clippy --locked --workspace
--all-targets --all-features -- -D warnings`, `cargo test --locked --workspace --all-targets
--all-features --no-fail-fast` and `./scripts/candid_review.sh` all pass.

# Candid Review Report

- **Date**: 2026-09-30
- **Target Branch**: `fix/p1-storage-vision-batch`
- **Base (merge-base)**: `850abc5`
- **Reviewed-Diff-Fingerprint**: `a290d4dab8b66f77f8cc22f5b340eb360f00e95e91abc5b45f2905733382e279`
- **Review round**: 3. Round 1 (`27ece66e...`) and round 2 (`d767676d...`) returned CHANGES_REQUESTED.
- **Claimed issues**: #156, #177, #178, #179, #180, #181, #182, #183, #184, #190, #191
- **Audited Files**: 80 files (see `target/candid_diff.patch`). This round focuses on the rework
  commits since round 2 (`d017f96`, `7a58284`, `75be84e`, `9a9b184`), which touch only
  `crates/admin-cli/src/{gdm.rs,pam_stack.rs}`, `crates/admin-cli/tests/{gdm_stack_order_tests.rs,gdm_tests.rs}`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/98_gdm_enable_stack_order.md`
  and `Docs/DISTRIBUTION_DEPLOYMENT.md`. The rest of the batch was checked for regressions from
  these commits only.

## 1. Executive Summary

All four round-2 findings are fixed in code. A missing, dangling, unreadable or non-file include
target now refuses. The file is left unchanged, no backup is written and `gdm.disable` is kept.
PAM type and control keywords are now read case-insensitively in gate classification, delegation
and jump detection. Module paths and arguments stay case-sensitive. De-duplication counts only
earlier enforcing (`required`/`requisite`) auth rules. The known refusals are documented.

Adversarial probing ran in a scratch crate, since deleted. The Ubuntu, Fedora and Arch
`gdm-password` stacks, with their includes present, still enable correctly. It found two new
MINOR issues: a de-duplication hole that needs a non-default jump, and a FIFO include target that
hangs the CLI. Both need a root-authored, non-standard `/etc/pam.d`. `cargo fmt --check`,
workspace `clippy -D warnings` and all `soos-admin-cli` tests pass. There is no CRITICAL or MAJOR
finding.

## 2. Test Changes (mechanical listing from step 3)

Frozen patch, `^-[^-].*(assert|#[test]|...)`: exactly two lines (patch 7578-7579):
`assert_eq!(enroll.model_id, "mobilefacenet")` and `assert_eq!(enroll.model_version, "1.0.0")` in
`crates/enrollment-cli/tests/scaffold_tests.rs::test_cli_parse_enroll_subcommand_with_uid`,
replaced by manifest constants. User-approved (a), #182.

Pre-existing test files modified vs `origin/main` (`--diff-filter=M`): `gdm_tests.rs`,
`scaffold_tests.rs`, `daemon/tests/pipeline_integration_tests.rs`, `tests/invariants/src/lib.rs`.
The last two only gain lines.

`crates/admin-cli/tests/gdm_tests.rs`, compared line by line with `origin/main` (whose tests are
`subcommand_parsing`, `disable_and_enable_lifecycle`, `enable_creates_byte_exact_backup_with_original_mode`,
`enable_never_overwrites_an_existing_backup`, `enable_is_atomic_and_idempotent` and
`enable_refuses_a_symlinked_pam_file`):
- The `COMMON_AUTH` constant and the `write_common_auth` helper are new.
- A `write_common_auth(temp.path())` setup line is added to exactly the four approved tests.
- One line, `"common-auth".to_string()`, is added to the expected listing in
  `test_gdm_enable_is_atomic_and_idempotent`. User-approved (b).
- `test_gdm_status_configured_enabled` and `test_gdm_status_configured_disabled` also get the
  setup line, but those tests are new on this branch, not on `origin/main`, so this is not a
  pre-existing change.
- No other line of a pre-existing test was modified or removed.

New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`): none.
Inline test modules: only the two new `mod tests` (`pam_stack.rs`, `biometric-store/src/store.rs`).
The branch-local `gdm_stack_order_tests.rs` gains 8 tests and loses no line in the rework.

Test integrity: PASS.

## 3. Deep Reasoning Audit

### Logic & Architecture

**Round-2 findings, verified in code.**

1. (MAJOR, missing include) `pam_stack.rs:294-306`: `ReadError::NotFound` now returns
   `GdmConfig` naming the target and the vendor-directory limitation. `ReadError::Other` also
   refuses. `plan_gdm_enable` runs before any backup or write (`gdm.rs:197-213`), and
   `configure_gdm` only removes `gdm.disable` on success. FIXED.
2. (keyword case) `is_auth`, `delegation` and the new `is_enforcing` use `eq_ignore_ascii_case`.
   `as_guard` uses `is_enforcing` and writes the copied control in lowercase, and `max_jump` reads
   every `key=value` pair. The pre-anchor loop in `gdm.rs:245-272` uses the same predicates, so
   gate classification, delegation and jump detection all agree. `module_name`, `has_arg` and the
   module lists stay case-sensitive, as in libpam. FIXED.
3. (dedup) `gdm.rs:295-301` keeps only earlier rules with `is_auth() && is_enforcing()`. The
   earlier `requisite`/`required` choice is safe: after an earlier `required` failure, a
   `success=done` still returns the stored failure in libpam. FIXED for the round-2 scenarios (see
   Finding 1 for a remaining variant).
4. (docs) `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 item 6 lists the authselect `sssd` profile,
   openSUSE `common-auth` and vendor `/usr/lib/pam.d` stacks, with a manual recipe and its caveat.
   ADR, matrix rows GSO13-GSO15 and walkthrough 98 §12 are consistent with the code. FIXED.

Scenarios run through `configure_gdm(Enable)` (scratch crate, since deleted):

| Scenario | Result |
|---|---|
| Ubuntu 24.04 gdm-password + pam-auth-update `common-auth` (fprintd, unix) + `common-account` | enabled before `@include common-auth`, backup written, flag removed. PASS |
| Ubuntu, `common-auth` missing | refused, unchanged, no backup, flag kept. PASS |
| Ubuntu, `common-auth` mode 000 | refused (EACCES), unchanged, no backup, flag kept. PASS |
| Ubuntu, `common-auth` is a directory | refused (EISDIR). PASS |
| Ubuntu, `common-auth` is a dangling symlink | refused (not found). PASS |
| Fedora authselect `local` + `with-faillock` + fprintd (`substack password-auth`, `include postlogin`) | enabled after `pam_selinux_permit`, `faillock preauth silent` copied. PASS |
| Fedora, `postlogin` missing (after the credential stop) | enabled. Acceptable: scanning stops at the credential, and libpam's must-fail handler still fails the password path as before. PASS |
| Fedora, `password-auth` missing | refused. PASS |
| Arch gdm-password -> system-local-login -> system-login -> system-auth | enabled; `pam_shells`, `pam_nologin`, `faillock preauth` copied. PASS |
| Arch, `system-auth` missing at depth 3 | refused. PASS |
| `AUTH INCLUDE` + `AUTH REQUISITE pam_nologin.so` + `AUTH SUFFICIENT pam_unix.so` | enabled, gate copied in lowercase. PASS |
| `-Auth required pam_foo.so` before the include | refused. PASS |
| `Account include missing-account` (non-auth) | ignored for auth placement. PASS |
| `auth sufficient Pam_Unix.so` (module case) | not a credential, refused (fail-closed). PASS |
| in-file `auth Requisite pam_nologin.so` + delegated `requisite pam_nologin.so` | de-duplicated correctly. PASS |
| **in-file `[success=1 default=ignore] pam_succeed_if.so user ingroup vip`, `requisite pam_nologin.so`, `pam_faildelay.so`, `@include common-auth` (with `requisite pam_nologin.so`)** | **enabled, delegated nologin dropped as duplicate, although the earlier copy can be jumped over. FINDING 1** |
| **include target is a FIFO** | **`soos-admin` blocks forever in `open()`. FINDING 2** |

Regressions: the rework changes nothing outside `admin-cli` and docs. Refusal now covers every read
error, and no path that refused in round 2 now accepts. No regression found.

### PAM Concurrency & Deadlines
`crates/pam` is untouched by the rework. `admin-cli` runs outside the PAM stack. PASS.

### Panic Safety & Fail-Closed
The rework adds no `unwrap`/`expect`/indexing in production. Every read error of a delegated stack
now fails closed. The only remaining fail-open is Finding 1, which needs a non-default
administrator-written jump.

### Test Integrity & Anti-Weakening
See section 2. The new tests (capitalized gate/unclassified/jump/`Include`, optional-vs-enforcing
dedup, missing include) each fail against the round-2 code. The missing-include test checks
the unchanged file, no backup and the kept flag through `assert_enable_refused`. PASS.

### Memory, Bounds & Secrets
Include reads stay bounded at 64 KiB, include depth is bounded at 4, and error messages contain
only paths and module names. PASS, apart from the unbounded blocking open in Finding 2.

### Supply Chain & Automation
The rework does not change `Cargo.*`, CI, scripts or hooks. PASS.

### English-Only Policy
Code, comments, tests, ADR, matrix, walkthrough and docs added in the rework are English. PASS.

## 4. Detailed Findings & Action Items

1. **[MINOR]** `crates/admin-cli/src/gdm.rs:295-301` (with the jump check at `:256-261`, `:283-291`)
   — a delegated gate is dropped as a duplicate when the edited file already runs the same gate
   with `required`/`requisite`, even when a pre-anchor jump can skip that earlier copy. The jump
   check only refuses jumps that land on or beyond the insertion point.
   Reproduced with:

   ```
   auth [success=1 default=ignore] pam_succeed_if.so user ingroup vip
   auth requisite pam_nologin.so
   auth required pam_faildelay.so delay=1
   @include common-auth
   ```

   `common-auth` runs `auth requisite pam_nologin.so` before `pam_unix`. In the original stack,
   members of `vip` skip the first nologin but still hit the one in `common-auth`. After `enable`,
   that one is not copied, so for `vip` members a face match returns `success=done` while
   `/etc/nologin` exists. This needs a non-default, administrator-written jump over a gate, so it
   is MINOR, like round-2 Finding 3. Correction: skip de-duplication when any pre-anchor jump
   exists, or drop de-duplication altogether (running a gate twice is harmless, as the docs
   already say for `preauth`). Add a contract test.
2. **[MINOR]** `crates/admin-cli/src/pam_stack.rs:369-372` (called from `scan_stack`, `:294`) —
   `read_bounded_utf8` opens delegated stacks with a plain `File::open`, with no `O_NONBLOCK` and
   no regular-file check. An include target that is a FIFO makes `soos-admin gdm enable` block
   forever: reproduced, killed by `timeout 10`, exit 124. Nothing is written, so this is not
   fail-open, and only root can create a FIFO in `/etc/pam.d`. Correction: open include targets
   like `read_backup_bounded` does (`O_NONBLOCK|O_CLOEXEC`, `is_file()` on the descriptor), while
   still following symlinks as libpam does.

## 5. Final Verdict

All round-1 and round-2 findings are resolved in code. Test integrity holds: the only changes to
pre-existing tests are the user-approved ones. The distribution stacks still place `pam_soos.so`
correctly. The two remaining findings are MINOR, need root-authored non-standard configuration,
and can be fixed in a follow-up.

**VERDICT: APPROVED**

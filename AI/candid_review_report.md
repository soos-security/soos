# Candid Review Report

- **Date**: 2026-11-20
- **Target Branch**: `fix/pam-success-done`
- **Base (merge-base)**: `0cce947`
- **Reviewed-Diff-Fingerprint**: `ba36637b4c71cf8d73d562736e8a5459b3f8e815033345fd12e66f24c982a1cd`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_pam_success_done.md`,
  `AI/auditor_constraints_pam_success_done.md`, `AI/walkthroughs/264_pam_success_done.md`,
  `Cargo.lock`, `Cargo.toml`, `Docs/DISTRIBUTION_DEPLOYMENT.md`,
  `Docs/PACKAGING_AND_PROVISIONING.md`, `crates/admin-cli/Cargo.toml`,
  `crates/admin-cli/src/{args,gdm,lib,main,pam_check}.rs`,
  `crates/admin-cli/tests/{gdm_flag_path_tests,pam_success_done_tests}.rs`,
  `packaging/pam/fedora/soos/README`, `packaging/rpm/soos.spec`, `scripts/install.sh`,
  `tests/invariants/src/{lib,pam_success_done_contract}.rs`

Review round 3 of security review batch 10 (GitHub #487: #574 `PSTK-01`, #575 `PSTK-02`,
#581 `PSTKC-01`), raw diff only, independent of the author's reasoning.

## 1. Executive Summary

The branch adds a read-only `soos-admin pam check` (bounded flattener plus a two-scenario
face/password simulator), makes `gdm enable` refuse a managed `[success=done]` block that would
skip an enforcing auth rule the password path still runs after the anchor, derives the `gdm`
disable flag from the rule `pam_soos.so` applies (`PamConfig::is_disabled`), and gates the
`custom/soos` authselect activation hint (installer and RPM `%post`) on local-only profiles.

The round-2 MAJOR item (a `pam_deny.so` of the anchor's shared stack reached on one branch only
was exempted) is fixed: the exemption now requires the password path to reach that rule in every
explored assignment, and an unfollowable run exempts nothing. A mutation that restores the old
exemption makes `test_pstk01_gdm_enable_refuses_conditional_deny_in_anchor_stack` fail, so the
new contract test pins the fix.

`crates/pam` is untouched; no PAM/FFI path changes; no existing test was modified. #574 is only
partly closed: the packaged Debian pam-config keeps `success=done` pending an owner decision
(PSK12, an existing contract test would need migration), which the branch records as deferred and
references (`Refs #574`) instead of closing. No CRITICAL or MAJOR finding.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `crates/admin-cli/tests/gdm_flag_path_tests.rs` (new),
  `crates/admin-cli/tests/pam_success_done_tests.rs` (new),
  `tests/invariants/src/pam_success_done_contract.rs` (new), `tests/invariants/src/lib.rs`
  (one `mod` declaration appended, no line removed).
- Removed or changed assertions / `#[test]` lines (`^-` grep): none.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance): none.
- Inline test modules: one new `mod tests` in the new file `crates/admin-cli/src/pam_check.rs`;
  no pre-existing inline test module touched (the `gdm.rs` hunks contain no test code).
- The batch-3 default-migration exception is not used.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Round-2 scenario (`[success=1] pam_unix` / `requisite pam_deny` / `required pam_permit` /
  `[success=1] pam_succeed_if ingroup admins` / `requisite pam_deny`): the second `pam_deny.so`
  is reached only on the failure branch, so it is not in the every-assignment set and is
  reported; refusal confirmed by the contract test and by mutation. PASS.
- Stock Debian `common-auth`: the `pam_deny.so` is jumped over on the password path, never
  visited, never reported; stock Ubuntu `gdm-password` tail (`optional` keyring) accepted. PASS.
- Unknown run (jump over a substack, unknown control, opaque anchor): `reach` is `None`, the
  `pam_deny.so` exemption is off and every reportable tail rule counts. PASS (fail closed).
- `PAM_IGNORE` forking: only when its action differs from both success and failure; bound
  `3^8` assignments, two forward-only runs each over at most 1024 rules. PASS.
- `covered_by`: identical plain `required`/`requisite` copy, or `authsucc` after an enforcing
  `preauth`; gates list = head lines plus anchor rules before the first credential module,
  matching what `delegated_auth` copies or refuses. PASS.
- Flags (#581): `module_flag_names` mirrors `PamConfig::is_disabled` (global, `gdm.disable` for
  `*gdm*`, `<service>.disable` with the same name charset); PFL1 pins agreement with the module
  crate. `Disable` refuses a flag the module never reads and writes nothing; a `--disable-file`
  outside `/etc/soos` is refused for `disable` and warned for other actions, the status being
  recomputed from `/etc/soos`. PASS.
- Installer (#575): the command is printed only for `local`/`minimal` (features carried over,
  no `--force`); every other profile gets a warning and no command. RPM `%post` matches whole
  profile strings (no `%%` expansion pitfall). PASS.

### PAM Concurrency & Deadlines

`crates/pam` is not modified; `soos-pam` is a dev-dependency of `soos-admin-cli` only. No
runtime, thread or blocking call is added to the module. PASS.

### Panic Safety & Fail-Closed

No `unwrap`/`expect`/indexing in the new production code (`get`, `checked_*`, `saturating_*`,
`unwrap_or`); `to_json` falls back to `{}`. Every bound or unreadable target is reported as
`unanalysed` (exit 2) or refuses `gdm enable`; nothing maps an error to an accepted plan. No
path reaches `PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening

New tests only. The PSK14 conditional-deny test fails against the round-2 implementation
(mutation run). PSK10b executes the macro-expanded `%post` block. Unit tests cover the ignore
fork and control-character stripping. PASS.

### Memory, Bounds & Secrets

Reads go through `read_bounded_utf8`; depth 4, 32 reads, 1024 rules, 8 branch points, 256
services. Output only file names, line numbers, controls and module names, passed through
`printable()`. No biometric or credential data involved. PASS.

### Supply Chain & Automation

Only an in-workspace path dependency added (dev-only). `scripts/install.sh` function quotes its
expansions. No CI/workflow change. PASS.

### English-Only Policy

All added code, comments, docs and commit messages are English. No owner-host material, no
private review material: only public issue numbers and finding IDs are cited. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/admin-cli/src/gdm.rs` (`pam_service_name`, `Disable` arm) — the service is
  the file name of `--pam-file`. When `--pam-file` points at a stack included by other services
  (for example `/etc/pam.d/system-local-login`), `gdm enable` can insert the block there, but
  `gdm disable` writes `<file>.disable`, which `pam_soos.so` never reads for the including
  services, and `status` reports `Disabled`. Same class as #581, needs a non-default option;
  follow-up: document that `--pam-file` must be a service file, or refuse a file other services
  include.
- **[MINOR]** `crates/admin-cli/src/pam_check.rs` (`gdm_tail_skips`) — a `pam_deny.so` of the
  anchor stack that the password path avoids only on an unexplored return code (e.g.
  `user_unknown=1`) is exempt. Contrived and listed as a known limitation in the walkthrough.
- **[MINOR]** `scripts/install.sh` (`authselect_activation_hint`, empty profile case) — still
  prints a `--force` command, preceded by a local-accounts-only warning; acceptable since no
  authselect profile is recorded, documented in §4.3.
- **[SUGGESTION]** #574 remains open for the packaged Debian layout until the owner decides
  PSK12; the PR must keep `Refs #574` (no closing keyword) and state the deferral.

## 5. Final Verdict

**VERDICT: APPROVED**

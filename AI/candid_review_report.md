# Candid Review Report

- **Date**: 2026-11-20
- **Target Branch**: `fix/pam-success-done`
- **Base (merge-base)**: `0cce947`
- **Reviewed-Diff-Fingerprint**: `3d978d916d290f61a75d551dfa034adb134c4b40509e6b6ee56f2785a861f53b`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_pam_success_done.md`,
  `AI/auditor_constraints_pam_success_done.md`, `AI/walkthroughs/264_pam_success_done.md`,
  `Cargo.lock`, `Cargo.toml`, `Docs/DISTRIBUTION_DEPLOYMENT.md`,
  `Docs/PACKAGING_AND_PROVISIONING.md`, `crates/admin-cli/Cargo.toml`,
  `crates/admin-cli/src/{args,gdm,lib,main,pam_check}.rs`,
  `crates/admin-cli/tests/{gdm_flag_path_tests,pam_success_done_tests}.rs`,
  `packaging/pam/fedora/soos/README`, `packaging/rpm/soos.spec`, `scripts/install.sh`,
  `tests/invariants/src/{lib,pam_success_done_contract}.rs`

Review round 1 of security review batch 10 (GitHub #487: #574 `PSTK-01`, #575 `PSTK-02`,
#581 `PSTKC-01`).

## 1. Executive Summary

The diff adds a bounded PAM stack simulator (`pam_check.rs`), a `gdm enable` refusal for
enforcing rules after the anchor, a read-only `soos-admin pam check`, module-aligned disable
flags for `soos-admin gdm`, and a profile-gated authselect activation hint. `crates/pam` is not
touched, no `PAM_SUCCESS` path is added, no existing test is changed, fmt/clippy/tests are green.

Two MAJOR defects remain:
1. The RPM `%post` warning of #575 does not fire on a typical directory host: `%%` is an RPM
   macro escape, so the scriptlet runs `${current% *}` (strip the last word only), not
   `${current%% *}`.
2. `gdm enable` still inserts its `[success=done]` block when an enforcing factor sits in the
   anchor's own shared stack after the credential module (for example `auth required
   pam_u2f.so` appended to `common-auth`): a face match skips a factor the password runs, which
   is the #574 defect for the GDM managed block, and the new `soos-admin pam check` reports the
   very layout `gdm enable` just wrote.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:
- Test files touched: `crates/admin-cli/tests/gdm_flag_path_tests.rs` (new),
  `crates/admin-cli/tests/pam_success_done_tests.rs` (new), `tests/invariants/src/lib.rs`
  (one added `mod` declaration at the end), `tests/invariants/src/pam_success_done_contract.rs`
  (new).
- Removed or changed assertions / `#[test]` lines: none.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance): none.
- Inline test modules: one new `mod tests` in `crates/admin-cli/src/pam_check.rs`.

No pre-existing test was weakened; the batch-3 default-migration exception is not used.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: Ubuntu `gdm-password` = gates, `@include common-auth`, `auth required pam_u2f.so`.
  `gdm enable` refuses, file unchanged (PSK1). PASS.
- Scenario: bracketed controls, `-auth`, later `include`/`substack`/`@include`, missing target
  (caveat G302-03). Classified and refused fail closed. PASS.
- Scenario (probe run by the reviewer, then deleted): `common-auth` = stock Ubuntu stack plus
  `auth required pam_u2f.so`, `gdm-password` = `auth requisite pam_nologin.so`,
  `@include common-auth`, `auth optional pam_gnome_keyring.so`. `gdm enable` returns
  `Ok(installed=true)` and writes the managed `[success=done]` block; `soos-admin pam check
  gdm-password` on the result then reports `[SKIP] ... common-auth:5 'pam_u2f.so' (control
  'required')`. FINDING (MAJOR, see section 4).
- Scenario (probe): stock Arch `gdm-password` -> `system-local-login` -> `system-login` ->
  `system-auth` without a soos rule. `gdm enable` inserts and copies the pre-credential gates;
  `pam check` then reports `pam_shells.so`, `pam_nologin.so` and `pam_faillock.so preauth`
  (gates that the managed block copied in front of the face rule) as skipped, exit 1.
  FINDING (MINOR, false positives on a layout the tool itself produced).
- Disable flags: `pam_service_name`, `module_flag_names`, `module_disabled` match
  `PamConfig::is_disabled` (PFL1 pins it against `soos-pam`); `gdm disable --pam-file
  .../sddm` writes `/etc/soos/sddm.disable`, refuses `gdm.disable` and any path outside
  `/etc/soos`. PASS.
- `install.sh` hint: `local`/`minimal` print the command with features and without `--force`;
  other profiles warn. PASS.
- RPM `%post`: see MAJOR finding 1.

### PAM Concurrency & Deadlines
`crates/pam` is untouched. The new code runs only in the admin CLI. PASS.

### Panic Safety & Fail-Closed
No `unwrap`/`expect`/indexing in production code of `pam_check.rs` (only `get`, checked
arithmetic, `unwrap_or_default`); `to_json` falls back to `{}`. An unanalysable tail refuses,
an unanalysable anchor or unfollowable run counts every enforcing tail rule. No path reaches
`PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening
Only new tests. The PSK/PFL tests fail against a plausible wrong implementation (keyword-only
predicate, no delegation following, `gdm.disable` for `sddm`). Gap: no test covers an enforcing
rule inside the anchor's shared stack after the credential module (finding 2), and no test runs
the RPM scriptlet after macro expansion (finding 1; PSK10 only checks substrings).

### Memory, Bounds & Secrets
Reads through `read_bounded_utf8`; bounds 256 services, depth 4, 32 reads, 1024 rules,
8 branch points (256 assignments); names validated as `[A-Za-z0-9._-]`; control characters in
operator text replaced. No secrets, frames or embeddings involved. PASS.

### Supply Chain & Automation
`soos-pam` added as a workspace path dev-dependency of `soos-admin-cli` only; no external crate.
`scripts/install.sh` function quotes every expansion. PASS (RPM scriptlet: finding 1).

### English-Only Policy
All added text is English. No owner-host material or private review material found in the
patch. PASS.

## 4. Detailed Findings & Action Items

- **[MAJOR]** `packaging/rpm/soos.spec:141` and `:143` — `%%` is the RPM macro escape for a
  literal `%`, so the installed `%post` runs `case "${current% *}"`, which removes only the
  last word. `authselect current --raw` = `sssd with-mkhomedir with-sudo` yields
  `sssd with-mkhomedir`, which matches no pattern: the #575 warning is silent on a typical
  directory-joined host (it fires only with zero or one feature). Fix: avoid the `%%` operator
  in the spec, e.g. `case "${current}" in sssd|"sssd "*|winbind|"winbind "*|nis|"nis "*)`, and
  print the id from the matched pattern (or write `%%%%`). Add an invariant that rejects a
  `%%` parameter expansion in this scriptlet, or that checks the expanded form.
- **[MAJOR]** `crates/admin-cli/src/pam_check.rs:814-826` (`gdm_tail_skips`, `!rule.in_anchor`)
  — enforcing rules of the anchor's own delegated stack that run after its credential module
  are never reported, so `gdm enable` inserts `[success=done]` in front of
  `@include common-auth` even when `common-auth` ends with `auth required pam_u2f.so`; a face
  match then replaces "password and second factor" with "face" on GDM, the #574 defect for the
  managed block ("a face match skips every rule the administrator placed after it"). The
  pre-existing delegated scan stops at the credential module (`pam_stack.rs`), so nothing else
  classifies these rules, and `soos-admin pam check` flags the result immediately. Fix: in
  `gdm_tail_skips`, also report reachable enforcing anchor rules after the first credential
  module (keep the "up to the first credential module" exemption, which covers the copied
  gates), with explicit, narrow exemptions only where the face path is known to be equivalent
  (e.g. `pam_faillock.so authsucc`, the already exempt modules), and add a contract test
  (`common-auth` + trailing `required pam_u2f.so` -> refused). If the owner prefers to keep the
  exemption, record it as an accepted residual in the ADR, Docs §2.1 and the walkthrough; the
  current Docs §2.1 text ("follows the credential module or shared stack") does not tell the
  operator that such a factor is skipped.
- **[MINOR]** `crates/admin-cli/src/pam_check.rs:658-676` (`check_service`) — `pam check` does
  not recognise the gates `gdm enable` copied in front of its managed rule, so on a stock Arch or
  Fedora GDM stack after `gdm enable` it reports `pam_shells.so`, `pam_nologin.so`,
  `pam_faillock.so preauth` as skipped (exit 1). Docs §3.2 tells operators to run it, so a
  supported layout looks broken. Apply the same "rules up to the first credential module"
  logic after a managed block, or treat a skipped rule whose identical copy (module and
  arguments) ran before the face rule as not skipped; add a test on a post-`gdm enable`
  Fedora/Arch stack.
- **[MINOR]** Scope / tracking — the Debian package rule (`packaging/pam/debian/soos`, the
  default layout of #574) still uses `success=done`; PSK12 is deferred for an owner-approved
  contract migration. Acceptable as an owner question, but the PR body must reference #574 with
  `Refs`, not close it, and say that the packaged Debian layout stays affected until PSK12.
- **[SUGGESTION]** `crates/admin-cli/src/main.rs:193-229` — the flag directory and the
  "outside" recomputation of `status.enabled` live in `main.rs`; moving them into the library
  (`configure_gdm_with_options` taking the flag directory) would let tests cover the CLI rule
  without spawning the binary.

## 5. Final Verdict

**VERDICT: CHANGES_REQUESTED**

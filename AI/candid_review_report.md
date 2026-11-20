# Candid Review Report

- **Date**: 2026-11-20
- **Target Branch**: `fix/pam-success-done`
- **Base (merge-base)**: `0cce947`
- **Reviewed-Diff-Fingerprint**: `c395aae29a139e8ddd0cb997cb4d75fec34ec20869bf74e2a1dfb6e38417d862`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_pam_success_done.md`,
  `AI/auditor_constraints_pam_success_done.md`, `AI/walkthroughs/264_pam_success_done.md`,
  `Cargo.lock`, `Cargo.toml`, `Docs/DISTRIBUTION_DEPLOYMENT.md`,
  `Docs/PACKAGING_AND_PROVISIONING.md`, `crates/admin-cli/Cargo.toml`,
  `crates/admin-cli/src/{args,gdm,lib,main,pam_check}.rs`,
  `crates/admin-cli/tests/{gdm_flag_path_tests,pam_success_done_tests}.rs`,
  `packaging/pam/fedora/soos/README`, `packaging/rpm/soos.spec`, `scripts/install.sh`,
  `tests/invariants/src/{lib,pam_success_done_contract}.rs`

Review round 2 of security review batch 10 (GitHub #487: #574 `PSTK-01`, #575 `PSTK-02`,
#581 `PSTKC-01`), raw diff only.

## 1. Executive Summary

Round 1 items are fixed: the RPM `%post` warning now matches the whole recorded profile
(PSK10b runs the macro-expanded block) and `gdm enable` counts enforcing rules after the
credential module of its anchor stack (PSK13). #581 (`gdm disable`/`status` follow the module's
flag rule, pinned against `PamConfig::is_disabled`) and #575 (activation hint gated on the
`local`/`minimal` profile, warnings elsewhere) are correct. The Debian packaged `[success=done]`
rule is deferred as an owner question (PSK12) because it changes asserted values of existing
contracts; the PR only references #574 — consistent with the zero test weakening rule.

One MAJOR defect remains in the #574 refusal: the `pam_deny.so` exemption of the anchor's own
stack (`ANCHOR_FALLBACK_MODULE`) is unconditional, so a group/user gate written with the
standard "conditional jump over `requisite pam_deny.so`" idiom after `pam_unix.so` in
`common-auth` is accepted by `gdm enable`, whose `[success=done]` rule then lets a face match
skip that gate while the password path still enforces it; `soos-admin pam check` flags the
layout `gdm enable` has just written (the same inconsistency class as round 1 item 2).
Reproduced end to end with the built binary.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `crates/admin-cli/tests/gdm_flag_path_tests.rs` (new),
  `crates/admin-cli/tests/pam_success_done_tests.rs` (new),
  `tests/invariants/src/pam_success_done_contract.rs` (new), `tests/invariants/src/lib.rs`
  (adds only the `mod pam_success_done_contract;` declaration).
- Removed/changed assertions (`^-` with assert/#[test]/proptest/should_panic): **none**.
- New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance): **none**.
- Inline test modules: one new `mod tests` in the new file `crates/admin-cli/src/pam_check.rs`.
- Removed lines in any `crates/**/*.rs` file are production only (`args.rs` doc + `disable_file`
  type, `gdm.rs` status predicate, `lib.rs` re-export list, `main.rs` call). No pre-existing
  test was modified; the batch-3 migration exception is not used.

## 3. Deep Reasoning Audit

### Logic & Architecture

- `soos-admin pam check` on hand-built stacks: `@include common-auth` followed by
  `REQUIRED pam_u2f.so`, by `-auth required pam_google_authenticator.so`, by
  `[success=ok ... default=bad] pam_u2f.so`, by a `sufficient pam_permit.so` branch before
  `pam_u2f.so`: all reported (exit 1). PASS.
- Tried to hide a factor behind unforked return codes (`[success=done auth_err=done
  default=ignore]`): the hiding rule itself is reported as enforcing, so the service is still
  flagged. PASS (residual limitation documented).
- `gdm enable` on a `common-auth` ending with
  `auth [success=1 default=ignore] pam_succeed_if.so user ingroup admins` +
  `auth requisite pam_deny.so` after `pam_unix.so`/`pam_permit.so`: **accepted**, managed block
  written; `pam check` on the result reports `common-auth:5 'pam_deny.so'` (exit 1).
  **FINDING (MAJOR)**, section 4.
- #581: `module_flag_names`/`default_disable_file`/`module_disabled` match
  `PamConfig::is_disabled` (`disabled`, `gdm.disable` when the service contains `gdm`,
  `<service>.disable`); `Disable` refuses an unread name before writing; `main.rs` refuses a
  flag outside `/etc/soos` for `disable` and recomputes the status from `/etc/soos` otherwise.
  PASS.
- #575: `authselect_activation_hint` prints a command only for `local`/`minimal`, with the
  recorded features carried over (no duplicate `with-faillock`) and no `--force`; warns for any
  other id. `%post` patterns contain no `%`, so they survive macro expansion. PASS.
- Caveat G302-03: no literal `success=N` written; brackets, `-auth`, later
  `include`/`substack`/`@include` classified (PSK2/PSK3). PASS.

### PAM Concurrency & Deadlines

`crates/pam` untouched (only added as a dev-dependency of `soos-admin-cli`). No runtime,
thread or blocking call added to the module. PASS.

### Panic Safety & Fail-Closed

- No `unwrap`/`expect`/indexing in the new production code (`get`, `saturating`/`checked`
  arithmetic, `to_json` falls back without panicking). PASS.
- No error becomes `PAM_SUCCESS` (module unchanged). Unanalysable tails refuse; unanalysable
  services report exit 2, never clean. PASS except the exemption in section 4, which fails
  open for one layout.

### Test Integrity & Anti-Weakening

- Mutation: forcing `gdm_tail_skips` to return `Ok(None)` makes 14 of 29
  `pam_success_done_tests` fail — the contract bites. PASS.
- No test covers a conditional gate over the anchor-stack `pam_deny.so` (why the MAJOR finding
  passes the suite).

### Memory, Bounds & Secrets

Reads through `read_bounded_utf8`; flattening bounded (4 levels, 32 files, 1024 rules,
8 branch points, 256 services). No secret, frame or embedding involved. PASS. MINOR: module and
control words from PAM files are printed verbatim in the table and refusal messages
(`Rule::describe`); only the directory name goes through `printable`.

### Supply Chain & Automation

Only a workspace path dependency (`soos-pam`, dev-only) is added; no new external crate, no CI
or hook change. PASS.

### English-Only Policy

Code, comments, docs, commit messages and walkthrough are English. No host-specific path,
login or private review material in the diff. PASS.

### Verification run by the reviewer

Build env of the rules (per-worktree `CARGO_TARGET_DIR`, `CARGO_INCREMENTAL=0`,
`CARGO_BUILD_JOBS=3`, `--test-threads=4`): `cargo fmt --all -- --check` clean;
`cargo clippy --locked -p soos-admin-cli -p soos-invariants --all-targets --all-features -- -D
warnings` clean; `cargo test --locked -p soos-admin-cli -p soos-invariants -p soos-pam
--all-targets --all-features` all green.

## 4. Detailed Findings & Action Items

- **[MAJOR]** `crates/admin-cli/src/pam_check.rs:900` (with `ANCHOR_FALLBACK_MODULE` at
  `:53-58`): the exemption `!(rule.in_anchor && rule.module == ANCHOR_FALLBACK_MODULE)` drops
  every `pam_deny.so` of the anchor's delegated stack, including one the password path reaches
  only on one branch of a conditional rule. The exemption's own rationale ("a fallback the
  password path still reaches denies the correct password too") holds only when the deny is
  reached on every branch. With
  `common-auth` = `[success=1 default=ignore] pam_unix.so` / `requisite pam_deny.so` /
  `required pam_permit.so` / `[success=1 default=ignore] pam_succeed_if.so user ingroup admins` /
  `requisite pam_deny.so`, the second deny is the gate for users outside `admins`; `gdm enable`
  inserts the `[success=done]` block, so such a user is now let in at GDM by a face match while
  a correct password is refused, and `soos-admin pam check` reports
  `common-auth:5 'pam_deny.so'` on the file `gdm enable` just wrote. Fail-open on #574's own
  acceptance ("refuses when an enforcing auth rule after the anchor is reached") and the
  gdm-enable/pam-check disagreement round 1 fixed for other rules.
  **Required correction** (production code only): exempt the anchor-stack `pam_deny.so` only
  when the password path reaches it in every explored assignment (or when no branch point
  precedes it after the credential module); otherwise count it like any other enforcing rule.
  Add a new test (PSK14 family) with the layout above: `gdm enable` must refuse, file unchanged,
  and `pam check` stays consistent. Do not modify
  `test_igf18_block_removal_with_crossing_jump_is_refused` or existing PSK14 assertions.
- **[MINOR]** `crates/admin-cli/src/pam_check.rs:309-311`, `:139-146`: `Rule::describe` and the
  stack/rule lines of `format_table` print module/control words from PAM files without
  `printable()`; a control character in a (root-owned) PAM file reaches the terminal. Route
  them through `printable` like the directory name.
- **[MINOR]** `crates/admin-cli/src/pam_check.rs:554-569`: non-branch rules always return
  success and branch points fork only on `success`/`auth_err`; a rule whose `ignore` or
  `user_unknown` action differs from both is not explored. I could not build a layout this hides
  (the hiding rule is itself reported), and the limitation is documented (walkthrough §9);
  consider adding `Ret::Ignore` as a third branch.
- **[SUGGESTION]** `crates/admin-cli/src/main.rs:192-229`: move the flag-path derivation and the
  "outside `/etc/soos`" recomputation into the library so `gdm status` has one tested entry
  point (already deferred by the author).

## 5. Final Verdict

**VERDICT: CHANGES_REQUESTED**

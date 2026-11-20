# Walkthrough 264 — `success=done` and the PAM Stack (Security Review Batch 10)

- **Date**: 2026-11-20
- **Issue**: GitHub #487 (security review 2026-11-10, batch 10): #574 `PSTK-01` (MEDIUM), #575 `PSTK-02` (LOW), #581 `PSTKC-01` (LOW); no backlog id — **Branch**: `fix/pam-success-done`
- **Matrix criteria**: PSK1–PSK11, PFL1–PFL4 (PSK12 pending, owner decision)

## 1. Context & Objectives

- `PSTK-01`: every non-Arch primary soos rule is `[success=done default=ignore]`. `done` ends
  the whole stack, callers included when the shared file is spliced with `include` /
  `@include`. On Debian/Ubuntu the password module jumps onto `pam_permit.so` and the caller's
  rules after `@include common-auth` still run, so a face match silently skipped a second factor
  (`pam_u2f.so`, TOTP) or gate an administrator placed there. The GDM managed block had the same
  effect on every rule after its anchor, and `gdm enable` copied those lines unclassified.
- `PSTK-02`: the `custom/soos` authselect profile is derived from `local` (no `pam_sss`, no
  `sss` NSS source), yet `install.sh` printed `authselect select custom/soos with-faillock
  --force` whatever the current profile, including `sssd`/`winbind` hosts joined to a directory.
- `PSTKC-01`: `soos-admin gdm disable/status` trusted `--disable-file` and `--pam-file`, not
  the flags `pam_soos.so` reads; an SDDM stack showed `Disabled` while face login stayed active.

## 2. Architect Design

Spec `AI/architect_spec_pam_success_done.md` (with Revision 1). New `pub mod pam_check` in
`soos-admin-cli`: a bounded flattener of auth stacks (`include`/`@include` inline, `substack`
nested; `MAX_PAM_INCLUDE_DEPTH` 4, `MAX_PAM_STACK_READS` 32, `MAX_PAM_CHECK_RULES` 1024) and a
two-scenario simulator (face match: soos returns success; password: soos returns ignore, the
credential modules met after it succeed), forking every conditional rule
(`MAX_PAM_CHECK_BRANCHES` 8). A rule visited by the password path and not by the face path whose
control can fail the stack is "skipped". `gdm enable` uses it on the edited file from the anchor
(only rules after the first credential module and outside the anchor's own stack count) and
refuses; `soos-admin pam check` reports it per service. Flags: `pam_service_name`,
`module_flag_names`, `default_disable_file`, `module_disabled` and `DEFAULT_FLAG_DIR` in
`gdm.rs`; `--disable-file` becomes optional. Installer: `authselect_activation_hint` in
`scripts/install.sh`. ADR 2026-11-20 in `AI/DECISIONS.md`.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: APPROVED after one MAJOR finding (a success-only simulation
misses the branch where a conditional rule falls through to a factor), resolved by forking
conditional rules. MINOR: hosts using a vendor PAM directory get "unanalysed" (exit 2), fail
closed and documented; a stale custom flag outside `/etc/soos` now gets a warning.

## 4. Tester Contract

New files only; no existing test was modified.

| Test file | Rows | Red evidence |
|---|---|---|
| `crates/admin-cli/tests/pam_success_done_tests.rs` (22 tests) | PSK1–PSK8 | 16 failed against the stubs (`enable must refuse ...`, empty report); the 5 acceptance/negative controls passed by design; the opaque-anchor test was added during Phase 4 |
| `crates/admin-cli/tests/gdm_flag_path_tests.rs` (7 tests) | PFL1–PFL4 | 6 failed (`module_flag_names` empty, Disable wrote `gdm.disable` for `sddm`, CLI `disable_file` defaulted); the agreement control passed |
| `tests/invariants/src/pam_success_done_contract.rs` (6 tests) | PSK9–PSK11 | 6 failed (`scripts/install.sh must define authselect_activation_hint()`, docs) |
| `crates/admin-cli/src/pam_check.rs` unit tests (3) | PSK2, PSK7 | written with the implementation |

Migrated existing tests: none. The batch-3 migration exception does not apply here.

## 5. Auditor Constraints

`AI/auditor_constraints_pam_success_done.md`, CLEARED, 10 constraints: no panic paths (workspace
lints), every read through `read_bounded_utf8`, every bound fails closed, names validated as
`[A-Za-z0-9._-]`, refusals write nothing, `gdm disable` writes only a module-read flag, output
limited to file names/lines/control and module words (control characters replaced), `crates/pam`
untouched, no new external dependency, installer function quotes every expansion and prints no
command on directory profiles.

## 6. Implementation

- `crates/admin-cli/src/pam_check.rs` (new): flattener, simulator, `check_pam_services`,
  `gdm_tail_skips`. The Debian `common-account`/`common-password`/`common-session`/
  `common-session-noninteractive` stacks may be absent from the PAM directory when reached by
  `@include` (a present one is analysed).
- `crates/admin-cli/src/gdm.rs`: refusal in `plan_gdm_enable` (Insert plan only; the shared-rule
  path writes nothing and is covered by `pam check`); flag helpers; `Disable` refuses a flag the
  module never reads for the service; `get_gdm_status` counts exactly the module's flags in the
  directory of `disable_file` plus the global `/etc/soos/disabled`.
- `crates/admin-cli/src/args.rs`, `main.rs`, `lib.rs`: `soos-admin pam check [--pam-dir]
  [SERVICE...]` (exit 0/1/2, `--format json`); `--disable-file` optional and derived; `gdm
  disable` refuses a flag outside `/etc/soos`, other actions warn and compute the status from
  `/etc/soos`.
- `Cargo.toml`, `crates/admin-cli/Cargo.toml`, `Cargo.lock`: `soos-pam` workspace entry, used as
  a dev-dependency of `soos-admin-cli` only (contract PFL1 against `PamConfig::is_disabled`).
- `scripts/install.sh`: `authselect_activation_hint`; `packaging/rpm/soos.spec` `%post` warning
  on `sssd`/`winbind`/`nis`; `packaging/pam/fedora/soos/README` restriction.
- Docs: `Docs/DISTRIBUTION_DEPLOYMENT.md` §2 (post-include skip per distribution, `pam check`),
  §2.1 (flags per service, refusal), §3.2 (Debian warning), §4.1/§4.3 (local accounts only);
  `Docs/PACKAGING_AND_PROVISIONING.md`; `AI/ARCHITECTURE.md` §5; project facts.

Notable decision: `gdm_tail_skips` ignores every rule up to the first credential module, because
the existing delegated scan already copies those gates in front of the managed rule or refuses
(found by the pre-existing `test_gdm_enable_scans_past_a_gate_only_include`, which stayed
unchanged).

## 7. Candid Review

Not run in this phase: the fingerprint-bound review is performed next by an independent reviewer
on the committed diff.

## 8. Verification Results

Environment: per-worktree `CARGO_TARGET_DIR`, `CARGO_BUILD_JOBS=3`, `--test-threads=4`.

```bash
cargo fmt --all -- --check
cargo clippy --locked -p soos-admin-cli -p soos-invariants --all-targets --all-features -- -D warnings
cargo test --locked -p soos-admin-cli -p soos-invariants --all-targets --all-features -- --test-threads=4
cargo test --locked -p soos-pam --all-targets --all-features -- --test-threads=4
python3 scripts/sync_issue.py --check
bash -n scripts/install.sh
```

All green: fmt clean, clippy clean, 1458 tests passed and 0 failed across `soos-admin-cli`,
`soos-invariants` and `soos-pam`; `sync_issue` self-check OK. `./scripts/candid_review.sh`
(layer 1) and the fingerprint-bound layer 2 run with the independent review that follows.

## 9. Known Limitations / Follow-ups

- **Owner question (PSK12)**: the Debian pam-config should use `[success=end default=ignore]`
  (pam-auth-update rewrites it into a jump onto the success block, as `libpam-fprintd` does), so
  a face match runs the caller's rules after the include like a password. That changes the
  asserted control of the existing contract
  `test_debian_notify_profile_sits_between_pam_unix_and_pam_deny`, the upgrade fixtures and the
  Docker/distro harness regexes: it needs an owner-approved contract migration. The Docker matrix
  regression of #574 depends on it. Until then `soos-admin pam check` and Docs §2/§3.2 cover the
  Debian layout. A jump or `substack` form of `GDM_PAM_LINE` is not needed once `gdm enable`
  refuses the skipping layouts (and would change `test_gdm_pam_line_includes_timeout_ms_2500`).
- An sssd-derived `custom/soos-sssd` profile (#575 "deeper" fix) is L effort, out of scope.
- `pam check` assumes a module returns success unless its control makes it a branch point;
  `required`/`requisite`/`optional` rules are not forked (an enforcing rule visited only by the
  password path is reported whatever it returns).
- Related, other batch: #541 `G304-01` (manual GDM recipe for the authselect `sssd` profile).

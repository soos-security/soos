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
unchanged). Every enforcing rule after it counts, in the anchor's own delegated stack too (candid
review round 1), with two narrow equivalences: `pam_faillock.so authsucc` when an enforcing
`pam_faillock.so preauth` gate runs before the managed rule (same tally check; only the reset
is skipped), and the `pam_deny.so` failure fallback of the anchor's own stack when the simulated
password path reaches it in every explored assignment (it then denies the correct password too;
the pre-existing `test_igf18_block_removal_with_crossing_jump_is_refused` uses such a stack and
stayed unchanged). A `pam_deny.so` of the anchor's stack reached on one branch only counts
(candid review round 2).

## 7. Candid Review

Round 1 (independent reviewer, fingerprint-bound): **CHANGES_REQUESTED**, two MAJOR items, both
fixed in round 2:

1. `packaging/rpm/soos.spec` `%post`: RPM macro expansion turns `%%` into `%`, so the installed
   scriptlet ran `case "${current% *}"`, which strips only the last word: `sssd with-mkhomedir
   with-sudo` became `sssd with-mkhomedir` and the #575 warning stayed silent on typical joined
   hosts. Fix: whole-profile `case` patterns (`sssd|winbind|nis|"sssd "*|"winbind "*|"nis "*`),
   no parameter expansion, the warning names the recorded profile. New invariant PSK10b runs the
   macro-expanded `%post` authselect block with a stubbed `authselect` (PSK10 only greps).
2. `gdm_tail_skips` ignored enforcing rules after the credential module of the anchor's own
   delegated stack: `common-auth` + `auth required pam_u2f.so` was accepted, and `pam check`
   then flagged the layout `gdm enable` had just written. Fix: those rules count (PSK13), with
   the two equivalences above (PSK14).

Minor items: `pam check` now recognises the gates `gdm enable` copied in front of its managed
rule (an identical `required`/`requisite` rule the face path ran before its soos rule) and
`authsucc` after an enforcing `preauth` gate, so the stock Arch and Fedora stacks are clean after
`gdm enable` (PSK15). Tracking: the PR uses `Refs #574`, not a closing keyword: the packaged
Debian rule (`packaging/pam/debian/soos`, the default #574 layout) keeps `success=done` until the
owner decides PSK12. The suggestion to move the flag-directory derivation of `main.rs` into the
library is deferred (refactor without behaviour change; the behaviour is covered by PFL4).

Test changes in round 2: new tests only (PSK10b, PSK13-PSK15), plus one setup-only change in the
branch's own unit test `pam_check::tests::test_assignments_are_bounded` (the `Rule` literal gains
`args: Vec::new()`; assertions untouched). No existing assertion was modified.

Round 2 (independent reviewer): **CHANGES_REQUESTED**, one MAJOR item, fixed in round 3:

1. `gdm_tail_skips` exempted every `pam_deny.so` of the anchor's delegated stack, including one
   the password path reaches on one branch of a conditional rule only. A `common-auth` holding
   `[success=1 default=ignore] pam_unix.so` / `requisite pam_deny.so` / `required pam_permit.so`
   / `[success=1 default=ignore] pam_succeed_if.so user ingroup admins` / `requisite pam_deny.so`
   was accepted, so a face match would let a user outside `admins` in at GDM while a correct
   password is refused, and `pam check` then reported `common-auth:5` on the written file. Fix:
   the exemption needs the password path to reach that `pam_deny.so` in every explored
   assignment (an unfollowable run exempts nothing). New contract test (PSK14):
   `test_pstk01_gdm_enable_refuses_conditional_deny_in_anchor_stack` (refusal, file unchanged, no
   backup, and `pam check` reports `common-auth:5` on the layout the managed rule would produce).

Minor items fixed in round 3: `Rule::describe` and `PamCheckReport::format_table` route module,
control, stack, service and reason text through `printable()` (control characters of a PAM file
no longer reach the terminal); a branch point is now also forked on `PAM_IGNORE` when its action
differs from both the success and the failure actions (at most `3^8` = 6561 assignments; the
visited sets are hashed). Unit tests: `test_ignore_outcome_is_explored_when_its_action_differs`,
`test_reported_rules_strip_control_characters`. The suggestion to move the flag-path derivation
of `main.rs` into the library stays deferred (refactor without behaviour change, covered by PFL4).

Test changes in round 3: new tests only; no existing test or assertion was modified.

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
- Until PSK12 lands, the packaged Debian layout (`packaging/pam/debian/soos`,
  `[success=done]` in `common-auth`) stays affected by #574: the PR references #574 and does not
  close it.
- An sssd-derived `custom/soos-sssd` profile (#575 "deeper" fix) is L effort, out of scope.
- `pam check` assumes a module returns success unless its control makes it a branch point;
  `required`/`requisite`/`optional` rules are not forked (an enforcing rule visited only by the
  password path is reported whatever it returns). A branch point is forked on success, failure
  and ignore; other return codes (`user_unknown`, `cred_insufficient`, ...) whose bracket action
  differs from those three are not explored.
- Related, other batch: #541 `G304-01` (manual GDM recipe for the authselect `sssd` profile).

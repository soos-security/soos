# Plan Evaluation Report
- **Date**: 2026-11-20
- **Issue**: GitHub #487 (security review 2026-11-10, batch 10): findings #574 `PSTK-01`, #575 `PSTK-02`, #581 `PSTKC-01` (no `AI/BACKLOG.md` id)
- **Branch**: `fix/pam-success-done`
- **Base commit**: `0cce947`
- **Spec**: `AI/architect_spec_pam_success_done.md`

## 1. Coverage Matrix
| Acceptance line (issue "Recommendation" / "Regression test") | Spec element | Status |
|---|---|---|
| #574: `gdm enable` refuses when an enforcing auth rule follows the anchor | `gdm_tail_skips` + refusal in `plan_gdm_enable` | Covered |
| #574 caveat G302-03: bracketed controls, `-auth`, later `include`/`substack` lines are parsed | flattener + control classifier (bracket values, `-auth` stripped, tail delegations followed) | Covered |
| #574 caveat G302-03: no literal `success=N` under authselect / pam-auth-update | packaged controls unchanged; no jump written | Covered (nothing written) |
| #574: `soos-admin pam check` warns about post-include factors | `check_pam_services`, `soos-admin pam check` | Covered |
| #574: deployment documentation states the behaviour | Docs §2.1, §3.2, §4.2 | Covered |
| #574: Debian `success=end` / Docker regression | Deferred: owner question (contract migration of an invariant assertion) | Deferred with reason |
| #575: hint only for `local`/`minimal`, local-only warning otherwise, features carried over, `--force` only when not authselect-managed | `authselect_activation_hint` | Covered |
| #575: Docs §4.1/§4.3 and profile README carry the restriction; spec comment | Docs, README, `soos.spec` | Covered |
| #581: disable writes only a flag the module reads; status uses the module's rule | `module_flag_names`, `module_disabled`, Disable refusal, CLI `/etc/soos` check | Covered |
| #581 regression (1): status agrees with `PamConfig::is_disabled` for `sddm` | dev-dependency contract test | Covered |
| #581 regression (2): `--disable-file <elsewhere>` refused | library name check + CLI directory check | Covered |

## 2. Facts Verified Against Code
| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Module flags | `crates/pam/src/config.rs` `is_disabled` | `disabled`, `disable_if_file=`, `gdm.disable` if service contains `gdm`, `<service>.disable` for flag-safe names | Yes |
| `DEFAULT_FLAG_DIR` | `crates/pam/src/config.rs` | `/etc/soos` | Yes |
| `GDM_PAM_LINE` | `crates/admin-cli/src/gdm.rs` | `auth  [success=done default=ignore]  pam_soos.so timeout_ms=2500` (unchanged) | Yes |
| Post-anchor lines copied unclassified | `plan_gdm_enable` | `for raw in lines.iter().skip(anchor_index)` | Yes |
| Debian control pinned | `tests/invariants/src/lib.rs` `test_debian_notify_profile_sits_between_pam_unix_and_pam_deny` | asserts `[success=done default=ignore]` | Yes (hence deferred) |
| `GdmOptions` built with struct literals in tests | `gdm_shared_rule_tests.rs`, `gdm_restore_stale_backup_tests.rs` | `GdmOptions { force: true }` | Yes: no field added |
| CLI tests pass a temporary `--disable-file` to `enable` and `status` | `gdm_stack_order_tests.rs`, `gdm_shared_status_tests.rs` | yes | Yes: refusal limited to `disable` |
| install hint | `scripts/install.sh` | unconditional `info "Activate with: ... --force ..."` | Yes |

## 3. Pillar Analysis
### Pillar 1 — Architecture & threat model
- Failure scenario considered: a simulator that assumes every module succeeds misses a branch
  (`pam_succeed_if [success=1 default=ignore]` jumping over `pam_u2f` for some users only).
- Result: FINDING (MAJOR) — resolved in the spec revision: conditional non-credential rules
  (bracket controls whose success and default actions differ, `sufficient`) are forked; both
  scenarios run under the same assignment; more than 8 conditional rules make the run unknown
  (fail closed). Credential modules always succeed in scenario B (the password path).
### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: a change leaking into `crates/pam`. None: the module is untouched.
- Result: PASS.
### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: an unreadable tail include accepted as "nothing to skip".
- Result: PASS — the tail must be analysable or `gdm enable` refuses; only the four Debian
  non-auth stacks may be missing (they hold no auth rule by pam-auth-update construction), and if
  present they are read and analysed like any other stack.
- Failure scenario considered: the stock Debian `common-auth` reporting `pam_deny.so`/`pam_permit.so`
  as skipped (false positive on every host). Result: PASS — credential success jumps over
  `pam_deny.so`, `pam_permit.so` is exempt.
### Pillar 4 — Dependencies
- `soos-pam` as a dev-dependency of `soos-admin-cli` (workspace path crate, no new external crate,
  no duplicate version). Result: PASS.
### Pillar 5 — Data confidentiality
- `pam check` prints file names, line numbers and control/module words only (no arguments beyond
  the module line, no user data). Result: PASS.
### Pillar 6 — Test integrity
- Failure scenario considered: the new refusal breaking an existing `gdm enable` acceptance test.
  The plan keeps every existing test unchanged; any break is a production defect to fix, not a
  test to edit. Debian/GDM control changes are deferred because they would need test migration.
- Result: PASS.

## 4. Findings
- **[MAJOR]** (resolved in revision) success-only simulation misses conditional branches —
  fork conditional rules, cap 8.
- **[MINOR]** `pam check` on hosts using vendor PAM directories reports unanalysed services
  (exit 2): documented, fail closed.
- **[MINOR]** A stale custom flag outside `/etc/soos` still exists after an older `gdm disable`;
  the CLI now warns and computes `Status` from `/etc/soos`.

## 5. Verdict
VALIDATION_VERDICT: APPROVED

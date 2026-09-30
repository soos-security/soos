# Candid Review Report

- **Date**: 2026-09-30
- **Target Branch**: `fix/p1-quality-ci-batch`
- **Base (merge-base)**: `83ad42203`
- **Reviewed-Diff-Fingerprint**: `e2298b4c94ed73eb4b02da909dbc061edc2f363cb55a3d33c563934ff3c03389`
- **Review round**: 3 (round 1: CHANGES_REQUESTED, fingerprint `b387dcbf…`; round 2: APPROVED, fingerprint `bbde79da…`)
- **Claimed issues**: #185, #186, #187, #188, #189, #274, #168, #280
- **Audited Files**: .agents/skills/dev-workflow/SKILL.md, .agents/skills/dev-workflow/references/project-facts.md, .agents/skills/traceability-agent/SKILL.md, .github/workflows/ci.yml, AGENTS.md, AI/ARCHITECTURE.md, AI/BACKLOG.md, AI/DECISIONS.md, AI/VERIFICATION_MATRIX.md, AI/walkthroughs/103_packaged_timeout_and_physical_scripts.md, AI/walkthroughs/104_verification_matrix_citations.md, AI/walkthroughs/105_sync_issue_mapping_and_explicit_completion.md, AI/walkthroughs/106_failable_pam_matrix_and_distro_deploy_ci.md, AI/walkthroughs/107_deflake_timing_tests.md, Docs/CI_CD_AND_SECURITY.md, Docs/DEVELOPMENT_WORKFLOW.md, Docs/DISTRIBUTION_DEPLOYMENT.md, Docs/INFERENCE_ORT_CRATE.md, Docs/IPC_PROTOCOL.md, Docs/PACKAGING_AND_PROVISIONING.md, Docs/PAM_DOCKER_TEST_MATRIX.md, Docs/PAM_MODULE.md, Docs/SECURITY_AND_QUALITY_GUIDELINES.md, crates/camera-v4l/tests/bench_latency_tests.rs, crates/camera-v4l/tests/common/mod.rs, crates/camera-v4l/tests/error_recovery_tests.rs, crates/camera-v4l/tests/hotunplug_tests.rs, crates/camera-v4l/tests/mock_camera_tests.rs, crates/camera-v4l/tests/shutdown_tests.rs, crates/camera-v4l/tests/warmup_tests.rs, crates/daemon/tests/pipeline_integration_tests.rs, crates/daemon/tests/template_model_binding_tests.rs, crates/gui/tests/common/mod.rs, crates/gui/tests/layout_tests.rs, crates/vision/tests/bench_tests.rs, packaging/pam/arch/system-auth, packaging/pam/arch/system-auth.snippet, packaging/pam/debian/soos, packaging/pam/fedora/soos/README, packaging/pam/fedora/soos/password-auth, packaging/pam/fedora/soos/system-auth, packaging/rpm/soos.spec, save.sh, scripts/pr_loop.sh, scripts/sync_issue.py, tests/distro/arch_linux_test.sh, tests/distro/debian_ubuntu_test.sh, tests/distro/fedora_rhel_test.sh, tests/docker/authselect_profile_test.sh, tests/docker/mock_daemon.py, tests/docker/pam_case_lib.sh, tests/docker/pam_rollback_test.sh, tests/docker/test_packages.sh, tests/docker/test_suite.sh, tests/invariants/src/distro_matrix.rs, tests/invariants/src/lib.rs, tests/invariants/src/matrix_citations.rs, tests/invariants/src/pam_deadline_contract.rs, tests/invariants/src/physical_contract.rs, tests/invariants/src/sync_issue_contract.rs, tests/physical/adversarial_test.sh, tests/physical/enrollment_test.sh, tests/physical/multi_user_test.sh, tests/physical/screensaver_test.md

## 1. Executive Summary

Round 2 reviews the full frozen diff against `origin/main`. It focuses on the round-1 rework: commit
`e869cea`, plus `3c6e329` and `f7a225d` after `7fbab62`. The rework touches only `packaging/rpm/soos.spec`,
`tests/invariants/src/lib.rs`, `Docs/PACKAGING_AND_PROVISIONING.md`, the matrix, walkthroughs 106/107, and it
removes the `eprintln!` from the three gated benchmarks. No production Rust source changed.

Checks run by this reviewer:
- `cargo fmt --all -- --check`: clean.
- `cargo test -p soos-invariants`: 114 passed.

Docker was not run (out of scope).

The round-1 MAJOR finding is resolved. The `%files` guard now fails on the exact defect: the line
`%ghost %attr(0600, root, root) %{_sharedstatedir}/soos/master.key` is not a comment and contains `master.key`.
Round-1 findings 2 and 3 are resolved in substance. Two new MINOR issues remain in the upgrade guard and in
stale matrix wording. Round-1 findings 4 and 5 are tracked (issue #281, PR listing). Neither is a security
invariant or a CI failure. Verdict: APPROVED.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Removed or changed assertion lines (`^-.*assert`):
- patch L358 is a matrix row (PAD5), which is documentation.
- patch L4440, `test_pam_config_ordering_matches_spec`: `contains("pam_soos.so timeout_ms=250")` becomes
  `any(is_default_timeout_primary_soos_rule)`. The Fedora/Arch/authselect hunks swap the literal for
  `primary_soos_rule_offset`, and the ordering asserts are byte-identical. **User-approved (a).**

Multi-line assertions missed by the grep, found by reading every `tests/invariants/src/lib.rs` hunk:
- `lib.rs:1092-1102`, `test_package_scriptlets_provision_key_via_shared_helper`. The old assertion,
  `%ghost` + `master.key` present, becomes: no non-comment `%files` line contains `master.key`.
  **User-approved (c), 2026-09-30.** It is stricter and non-vacuous: a reintroduced `%ghost` key line fails it,
  and the explanatory comment no longer satisfies it.
- `lib.rs:1455-1467`, `test_rpm_packaging_specification`. The old check, `0600` + `master.key` anywhere,
  becomes: the spec contains `provision-master-key` and the helper has a non-comment `chmod 0600` or
  `umask 077` line. **User-approved (c).**
  - The `%post` call itself is asserted separately at `lib.rs:1075-1079`: the `%post` section must contain
    the helper path.
  - `lib.rs:896` independently requires `umask 077` in the helper, so the combination is not weaker than
    intended.

Other test changes:
- Camera, GUI and daemon test files: the polling migrations, env-gated benchmarks (`SOOS_LATENCY_BENCH=1`,
  run by the CI step "Latency benchmarks (single-threaded)" in job `test`, `ci.yml:171-178`), widened idle
  windows and the 5 s `connection_timeout`. **User-approved (b).**
  - The only change since round 1 is removing the `eprintln!` from the gated early returns
    (`shutdown_tests.rs` ×2, `vision/bench_tests.rs`). Thresholds are unchanged.
- New escape hatches: none. The `should_panic`/`tolerance` hits are string fixtures, docs or shell messages.
- Shell harness changes are equal or stricter (see round 1). None were changed since round 1.

No other assertion is weakened.

## 3. Deep Reasoning Audit

### Logic & Architecture

**Finding 1 (round 1).** `soos.spec:158-174` `%files` lists no `master.key`, only a comment.
- `%post` (`soos.spec:94`) calls the helper.
- The helper refuses symlinks, creates the key under `umask 077`, checks 32 bytes and publishes it with a hard
  link. It never overwrites an existing key.
- PASS.

**Finding 2 (round 1), `%pre`/`%posttrans` upgrade guard.** Scenarios attempted:
- **Fresh install** (`$1 = 1`): `%pre` skips the copy. `%posttrans` finds no `.master.key.upgrade` and does
  nothing. Unaffected. PASS.
- **Upgrade from a `%ghost` build.** RPM order is new `%pre` (copy), new files, new `%post` (key present, helper
  no-op), old `%preun`, old-file erase (key deleted), old `%postun`, then `%posttrans`. `%posttrans` sees the key
  missing and moves the copy back, then applies 0600 and root:root. Correct under the normal ordering. PASS.
- **Upgrade from this build to a later one**: the copy is made, the key survives, and `%posttrans` removes the
  copy. PASS.
- **Symlink or TOCTOU.** `/var/lib/soos` is `%dir %attr(0755, root, root)`, so only root can place entries.
  - `%pre` skips a symlinked key (`! -L`).
  - `mv -f` renames over a dangling symlink rather than following it.
  - `MasterKey::load_or_create` rejects a symlinked key (`O_NOFOLLOW`).
  - No unprivileged race exists. PASS.
- **Copy mode.** `cp -p` copies the source mode. The source is 0600, which the helper tightens on every
  `%post`, and the subshell `umask 077` covers creation. The copy is root-only. PASS.
- **Aborted transaction** (`%posttrans` never runs): a stale root-only 0600 copy of the same key remains in a
  root-only-writable directory. A later `%posttrans` removes it. Not a secret exposure. Acceptable.
- **Key recreated before `%posttrans`**: this path fails. See finding 1 below.

**Finding 3 (round 1).** `Docs/PACKAGING_AND_PROVISIONING.md:221,234` no longer say `%ghost` ownership. PASS.
Stale matrix wording remains (finding 2 below).

**Rest of the batch.** Re-read for regressions: #185 packaged stacks, #186 physical scripts, #188
`sync_issue.py`/`save.sh`, #189 matrix, #274 `distro-deploy`. Nothing changed since round 1. PASS.

### PAM Concurrency & Deadlines
No change to `crates/pam`. T2/T2b deadline assertions are unchanged since round 1. PASS.

### Panic Safety & Fail-Closed
No production code changed. A missing or wrong key makes the daemon fail to decrypt, which is fail-closed and
never reaches `Allow`. PASS.

### Test Integrity & Anti-Weakening
Only approved changes (a), (b) and (c) were found. The new `%files` guard was checked by reasoning against the
exact defect line. PASS.

### Memory, Bounds & Secrets
- No key bytes are printed by the scriptlets. `cp`, `mv`, `chmod` and `rm` handle paths only.
- `verify_package_has_no_key_material` still covers RPM with `--noghost`. With no `master.key` entry, the
  package can neither ship nor own the key.
- PASS, with finding 1 below.

### Supply Chain & Automation
Unchanged since round 1:
- Actions are pinned by SHA.
- `distro-deploy` uses `persist-credentials: false` and inherits `contents: read`.
- `${{ matrix.* }}` reaches scripts only through `env:`.

PASS.

### English-Only Policy
The rework lines (spec comments, docs, walkthrough, test messages) are English. PASS.

## 4. Detailed Findings & Action Items

1. **[MINOR]** `packaging/rpm/soos.spec:152-155`: `%posttrans` deletes the only surviving key copy without
   comparing it.
   - Failure scenario:
     - The upgrade starts from a build that owned the key as `%ghost`.
     - Something recreates `master.key` between the old-file erase and `%posttrans`. One case is an immediate
       `systemctl try-restart` in the old `%postun`: EL8-style `%systemd_postun_with_restart`, or any macro
       set that restarts before `%posttrans` runs. Another is a crash-restart (`Restart=on-failure`).
     - The daemon then calls `MasterKey::load_or_create` (`crates/daemon/src/pipeline.rs:311`,
       `crates/biometric-store/src/crypto.rs:90-98`), which silently generates a new key.
     - `%posttrans` takes the `else` branch and runs `rm -f .master.key.upgrade`.
     - Result: the original key is gone for good and every enrolled template is undecryptable.
   - A partially written copy (ENOSPC during `cp -p`) would also be restored without a 32-byte check.
   - Impact is limited: no RPM has been released (0.1.0-1), and the restart timing depends on the macros.
   - Suggested correction: remove the copy only when `cmp -s` shows it equals the current key. Otherwise keep
     it, for example renamed `.master.key.upgrade.<epoch>`, and print a path-only warning to stderr. Restore
     only a regular 32-byte copy.
   - Add this to the `rpm -U` Docker follow-up already tracked in #281.
2. **[MINOR]** `AI/VERIFICATION_MATRIX.md`: three rows still contradict the implemented design.
   - PMK4 (L530), a pre-existing row, says the RPM `%files` has "`master.key` kept `%ghost`".
   - PK6 (L244) still lists "`%files` strict attributes (… `0600` master.key)".
   - PMX8 (L839) says the RPM branch "fails on the real `%ghost` master-key defect", which is now fixed and
     verified in PK6.
   - Scenario: a reader or a future agent trusts the matrix and reintroduces `%ghost`. The invariant would then
     fail, but the traceability record is wrong.
   - Correction: reword the three rows to "not listed in `%files` (not even `%ghost`); 0600 enforced by the
     helper".
3. **[MINOR, tracked, non-blocking]** Round-1 finding 4: `tests/invariants/src/matrix_citations.rs` blind spots.
   - Rows with no code span are not checked.
   - `fn` in a `proptest!` file counts as a test.
   - `#[cfg(all(test, …))]` is accepted as a test attribute.
   - All three are listed in issue #281 (first checkbox). The invariant still catches unresolved citations. It
     only over-accepts, so it does not block this batch.
4. **[SUGGESTION, tracked, non-blocking]** Round-1 finding 5: after the 50 ms dwell in the negative polling
   checks, the reaction window is bounded by the `wait_until` loop rather than a fixed sleep. The caller says it
   is listed in the PR description. Approved migration (b), assertions unchanged.
5. **[SUGGESTION]** `tests/invariants/src/lib.rs:1461`: `spec_content.contains("provision-master-key")` is
   already satisfied by the `%install`/`%files` lines, so it does not by itself prove the `%post` call. The
   `%post` call is independently pinned at `lib.rs:1075-1079`, so no gap exists today. Scoping the check to the
   `%post` section would make the test self-contained.

## Round 3 — posttrans hardening delta

Scope: the only change since round 2 is commit `dfdcfb0 fix(packaging): never discard a differing
pre-upgrade rpm master key`. Checked with `git log` (HEAD = `dfdcfb0`, parent `e869cea` is the round-2 head)
and `git status` (only this report is modified). `git diff e869cea dfdcfb0` touches three files:
`packaging/rpm/soos.spec` (`%posttrans`), `AI/VERIFICATION_MATRIX.md` (PK6, PMK4, PMX8) and
`AI/walkthroughs/106_failable_pam_matrix_and_distro_deploy_ci.md` (one appended bullet). No Rust source or
test changed, so the step-3 mechanical listing is unchanged from round 2.

### Round-2 finding 1 (`packaging/rpm/soos.spec:146-166`) — RESOLVED
- The copy is only acted on when it is `-f` and `! -L`. Anything else (symlink, directory, absent) is a no-op.
- A copy whose size is not 32 bytes (`wc -c`) is left in place with a path-only warning. This covers
  the ENOSPC partial copy.
- If `master.key` is missing, the copy is restored (`mv -f`, `0600`, `root:root`).
- If `cmp -s` shows it equals the current key, the copy is deleted.
- Otherwise the copy is kept and a two-line warning names both paths. No key bytes are printed.
- Tested in a scratch shell harness that runs the scriptlet body verbatim, under `/bin/sh`:
  - missing key: restored, mode `600`, copy gone.
  - equal key: copy removed.
  - different key: both kept, warning printed.
  - 5-byte and 0-byte copy: kept, "not a 32-byte key" warning.
  - copy is a symlink: untouched.
  - no copy: no-op.
  - dangling symlink at `master.key`: `[ ! -e ]` is true, and `mv -f` renames over the link itself
    (rename(2), no write-through), leaving a regular `0600` file.
  - `cmp` removed from `PATH`: `command not found` (status 127) falls to the `else` branch, so the copy is
    **kept** and the warning printed. Fail-safe.
  - Every run's scriptlet exit status was 0.
- Shell adversarial notes:
  - `$(wc -c < "$UPGRADE_COPY")` on a vanished file: the redirect fails and yields `""`, so
    `[ "" -ne 32 ]` errors with status 2. That is false, so control goes to the `elif` branches, where
    `mv` fails harmlessly under `|| :` or `cmp` fails and the copy is kept. It is only reachable through a
    race inside a root-only `0755` directory while RPM holds the transaction lock. Acceptable.
  - GNU `wc -c` prints a bare integer, and `-ne` tolerates surrounding blanks.
  - Macros expand to fixed `/var/lib/soos` paths without spaces. The variables are quoted throughout.
  - RPM runs scriptlets with `/bin/sh` without `-e`. Every mutating command has `|| :`, and the
    `if` compound ends on an `echo` or a guarded command, so the scriptlet returns 0. A `%posttrans`
    failure cannot abort the transaction in any case.
  - `%pre` makes a copy on every upgrade, including builds that never owned the key. On those upgrades the
    key survives and `cmp -s` removes the copy. The copy is `0600 root` (`umask 077`, `cp -p`), so a
    leftover never widens exposure.
  - `cmp` comes from `diffutils`. The spec has no `Requires(posttrans): diffutils`. `diffutils` is in
    normal Fedora and RHEL installs and in the `fedora` container image, but it is not guaranteed on
    minimal/UBI-minimal images. Without it the outcome is safe (copy kept), but the warning wrongly says the
    keys differ and it repeats on every upgrade. See finding R3-1.

### Round-2 finding 2 (`AI/VERIFICATION_MATRIX.md`) — RESOLVED
- PK6 now reads "`master.key` is never listed, the `%post` helper creates it `0600`". Its remaining
  `%ghost` wording is only the history of the fix.
- PMK4 now reads "`master.key` is never listed, not even `%ghost`".
- PMX8 now says the RPM branch passes since the `%ghost` fix.
- None of the three describes the `%ghost` design as current.

### Checks
- `cargo test --locked -p soos-invariants --all-features`: 114 passed, 0 failed.
- Docker / `rpm -U` not run (out of scope; the `rpm -U` case stays tracked in #281).
- The logic, panic-safety, secrets and English-only pillars were re-run on the delta.
  - Warnings print paths only.
  - No PAM or Rust code changed.
  - The text is English.
  - Result: PASS.

### Round-3 findings
- **R3-1 [SUGGESTION]** `packaging/rpm/soos.spec:159` (the `cmp -s` call; the dependency itself belongs in the preamble): add `Requires(posttrans): diffutils`. The other
  option is to tell `cmp` status 2/127 apart and warn "could not compare". Today a missing `cmp` is
  fail-safe, but the message is misleading. Non-blocking; can go into #281.

## 5. Final Verdict

No CRITICAL or MAJOR finding remains. The round-1 MAJOR finding is resolved, and the test changes are limited
to the user-approved set (a), (b) and (c). Round 3: round-2 MINOR findings 1 and 2 are resolved by `dfdcfb0`.
Only one non-blocking SUGGESTION remains (R3-1, `diffutils` dependency).

**VERDICT: APPROVED**

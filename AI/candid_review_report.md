# Candid Review Report

- **Date**: 2026-09-30
- **Target Branch**: `fix/p1-install-batch`
- **Base (merge-base)**: `6e029e5`
- **Reviewed-Diff-Fingerprint**: `b7fb16666ec1905615c25cf5eb3e888d2b4f5b3b00e4982c5935f846a9693e1e`
- **Review round**: 2 (round 1: CHANGES_REQUESTED, fingerprint `4b4db6a5...9c65`)
- **Audited Files**: .github/workflows/ci.yml, AI/BACKLOG.md, AI/VERIFICATION_MATRIX.md, AI/walkthroughs/91_pam_packaging_order_and_rollback.md, AI/walkthroughs/92_distro_validation_matrix.md, AI/walkthroughs/93_installer_preflight_and_deps.md, Dockerfile, Docs/CI_CD_AND_SECURITY.md, Docs/DEVELOPMENT_WORKFLOW.md, Docs/DISTRIBUTION_DEPLOYMENT.md, Docs/PACKAGING_AND_PROVISIONING.md, Docs/PAM_DOCKER_TEST_MATRIX.md, Docs/SECURITY_AND_QUALITY_GUIDELINES.md, README.md, crates/admin-cli/src/gdm.rs, crates/admin-cli/tests/gdm_tests.rs, packaging/arch/PKGBUILD, packaging/arch/soos.install, packaging/debian/control, packaging/debian/rules, packaging/pam/debian/soos-notify, packaging/rpm/soos.spec, packaging/soos-daemon.service, run_tests.sh, scripts/build_arch.sh, scripts/build_deb.sh, scripts/check_build_deps.sh, scripts/download_models.sh, scripts/install.sh, scripts/pam_snapshot.sh, scripts/uninstall.sh, tests/distro/arch_linux_test.sh, tests/distro/debian_ubuntu_test.sh, tests/distro/fedora_rhel_test.sh, tests/distro/run_distro_validation.sh, tests/docker/Dockerfile.arch, tests/docker/Dockerfile.fedora, tests/docker/Dockerfile.ubuntu, tests/docker/mock_daemon.py, tests/docker/pam_rollback_test.sh, tests/docker/run_matrix.sh, tests/docker/test_suite.sh, tests/invariants/src/distro_matrix.rs, tests/invariants/src/installer_contract.rs, tests/invariants/src/lib.rs, tests/physical/pam_integration_test.sh

## 1. Executive Summary

Round 2 reviews the full batch diff again. It gives particular attention to the two rework
commits `0baabe5` (install/snapshot/uninstall hardening and the new `package-deploy` CI job) and
`c39cf21` (matrix and walkthrough evidence).

Round-1 status, checked in the code and not just the docs:

| # | Round-1 finding | Status | Evidence in code |
|---|---|---|---|
| 1 | MAJOR: #168 has no CI for packaging/deployment; matrix over-claims | **Resolved** | `ci.yml:266` job `package-deploy` (`if: github.event_name != 'schedule'` → runs on PRs, `needs: lint`, SHA-pinned checkout, `persist-credentials: false`, no event interpolation). It runs `./tests/distro/run_distro_validation.sh ubuntu`, then `tests/docker/test_packages.sh` in image `soos-distro-val-ubuntu` on volume `soos-distro-target-ubuntu`. Both names match `run_distro_validation.sh` (`tag="soos-distro-val-${distro}"`, `-v "soos-distro-target-${distro}":/workspace/target`). The job is listed in `ci-success` `needs`. Pinned by `test_ci_runs_ubuntu_package_deployment_on_pull_requests`. Matrix: PK5/DV1/DV4 cite the job; PK6/PK7/DV2/DV3 downgraded to `⬜ Pending` |
| 2 | MINOR: `--purge-data` destroys the snapshot after an incomplete rollback | **Resolved** | `uninstall.sh:402-410` keeps `state/pam-backup`, purges siblings (key, templates, evidence) and warns. `test_uninstall_purge_keeps_snapshot_when_pam_rollback_is_incomplete` + regression guard |
| 3 | MINOR: 6a untested; a partial snapshot leaks `.pam-backup.*` | **Resolved** | `pam_snapshot.sh:156-179` EXIT/INT/TERM trap; `discard` drops stale temp dirs; `install.sh:606-607` sets `PAM_SNAPSHOT_CREATED=true` before the helper runs. Live root tests D0a (after 6a) and D0b (inside the helper) in `pam_rollback_test.sh`, run by CI job `pam-rollback` |
| 4 | MINOR: walkthrough 92 P1 evidence stale | **Resolved** | WT92 §8.2 records the post-`2fbe969` ubuntu/arch runs |
| 7 | MINOR: `--destdir` guessed the PAM dir from the build host | **Resolved** | `install.sh:229-258`: under `--destdir` only `${DESTDIR}${candidate}` is probed, otherwise `/usr/lib/security` with a `--pam-dir` warning. All staging callers (`build_deb.sh`, `debian/rules`, `build_arch.sh`) pass `--pam-dir` explicitly; the RPM spec does not use `install.sh`. `test_install_destdir_never_guesses_pam_dir_from_build_host` |
| 5, 6, 8, 9, 10 | follow-ups | **Documented** | 5, 9, 10 in WT93 "Follow-ups"; 6, 8 in WT91 "Follow-ups" |

The rework introduces no new CRITICAL or MAJOR defect. What remains are MINOR or SUGGESTION items
(see §4).

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- `grep '^-[^-].*(assert|#[test]|#[tokio::test|proptest!|#[should_panic)'` on the frozen patch:
  **no hits**.
- `grep '^+.*(#[ignore|#[cfg(any())]|should_panic|tolerance|epsilon)'`: **no hits**.
- `grep '^[-+].*mod tests'`: **no hits**.
- `tests/invariants/src/lib.rs`, `distro_matrix.rs`, `installer_contract.rs`: additions only (zero
  `-` lines against `origin/main`). The only change to existing tests is the setup migration the
  user approved: `test_install_script_creates_required_directories` and
  `test_install_script_destdir_stages_no_key_material` gained `--artifact-dir` fixture setup, with
  their assertions untouched. The rework (`2fbe969..HEAD`) touches tests with +366/-0 lines.
- New tests in round 2: `test_ci_runs_ubuntu_package_deployment_on_pull_requests`,
  `test_install_journals_pam_snapshot_before_invoking_helper`,
  `test_pam_snapshot_failure_leaves_no_temporary_directory`,
  `test_uninstall_purge_keeps_snapshot_when_pam_rollback_is_incomplete`,
  `test_uninstall_purge_removes_state_after_verified_rollback`,
  `test_install_destdir_never_guesses_pam_dir_from_build_host`, and Docker D0a/D0b.
- Shell test removals are the same as in round 1 (`chmod 777`, fabricated `/dev/urandom`
  templates, `sleep 0.2` starts, `echo "\n"` stacks, `${distro}_test.sh`, the silent Docker
  fallback to dry-run). Each is replaced by a stricter check. This is a strengthening.
- Local run: `cargo test --locked -p soos-invariants` → 81 passed, 0 failed. `bash -n` is clean
  on `install.sh`, `uninstall.sh`, `pam_snapshot.sh` and `pam_rollback_test.sh`.

## 3. Deep Reasoning Audit

### Logic & Architecture
- *6a ordering*: the flag is set only when `SHA256SUMS` is absent, and the helper's own "keep
  existing" test uses the same predicate, so a pre-existing valid snapshot is never journaled or
  discarded. A helper failure under `set -e` exits `install.sh` non-zero, and `on_exit` then runs
  the rollback. That rollback restores backups, removes files, `discard`s the snapshot (which also
  removes the temp dir), does `rmdir state`, then the journaled dirs, then `groupdel`. D0b proves
  that no `/var/lib/soos` is left behind. PASS.
- *`pam_snapshot.sh` traps*: the trap is armed right after `mktemp` and disarmed right after the
  `mv`. A kill between `mv` and the disarm runs `rm -rf` on a path that no longer exists, which is
  harmless. The `current_manifest` failure path removes the temp dir and returns 1, and the trap
  is idempotent. PASS.
- *Uninstall purge*: the snapshot branch triggers only when `pam-backup` survived step 2, which
  happens only on an unverified or incomplete rollback, or with no helper. It guards against a
  symlinked `TARGET_STATE_DIR` or `state`. `rm -rf` does not follow symlinks. `master.key`
  (root-level) is purged. After a verified rollback the old full `rm -rf` path is kept. PASS.
- *`--destdir` fallback*: a Debian stage built with explicit `--pam-dir` is unaffected. A stage
  with no `--pam-dir` never ends up under `lib64`. The live path is unchanged (the host is the
  target). PASS.
- *#168 scope*: ubuntu runs on every PR. The issue's "fedora/arch on main" recommendation is
  deferred, and the matrix rows are honestly `Pending` (finding 1).

### PAM Concurrency & Deadlines
- No change in `crates/pam`. N/A. PASS.

### Panic Safety & Fail-Closed
- No new Rust production code in the rework. The round-1 conclusions (the notify hook cannot
  grant, the module is kept while any stack references it, `gdm.rs` does not panic) still hold.
  PASS.

### Test Integrity & Anti-Weakening
- See §2. The new tests fail against plausible wrong implementations. The CI test extracts the job
  body and checks the `ci-success` needs line. The purge test asserts that the snapshot survives
  and that the key is purged. D0b injects a `cp` failure only for `.pam-backup.*/pam.d/*`, so it
  exercises exactly the helper path. PASS.

### Memory, Bounds & Secrets
- The CI job mounts the workspace read-write into a root container, but all build output goes to
  the named volume (`target/`) and `/tmp` stages. The key is generated in the container and
  checked to be absent from the `.deb`. No secrets are passed to the job. PASS.

### Supply Chain & Automation
- `package-deploy`: SHA-pinned checkout (the same pin as the other jobs), workflow-level
  `permissions: contents: read`, `persist-credentials: false`, no `${{ github.event.* }}` in
  `run:`, 75-minute ceiling. On a cold runner, step 1 builds the image (apt + rustup) and runs a
  full `cargo build --release --workspace` inside the container on an empty volume, which is
  plausible within 75 minutes. Step 2 reuses the image and volume from step 1 on the same runner,
  so `test_packages.sh` skips the build. `WORKDIR /workspace` makes its relative paths valid. The
  `dpkg -i` dependencies (`libpam-runtime`, `adduser|passwd`, `systemd`) are present in
  `Dockerfile.ubuntu`. PASS, with finding 2.

### English-Only Policy
- A scan of the added lines found no non-English diacritics. PASS.

## 4. Detailed Findings & Action Items

No CRITICAL or MAJOR findings.

1. **[MINOR]** `.github/workflows/ci.yml:266`, `AI/VERIFICATION_MATRIX.md` PK6/PK7/DV2/DV3 — #168's
   recommendation also asks for Fedora/Arch deployment in CI on `main`. This is deferred (the rows
   are correctly `⬜ Pending`, and the follow-up is documented in WT92/WT93). The PR must not
   present #168 as fully closed: use `Refs #168` (as the commits do) plus a follow-up issue, or
   close it explicitly as "ubuntu done, fedora/arch tracked in #NNN". The rows DV1/PK5/DVM9 cite a
   job that has not run yet ("first CI run happens on the PR"). Add the run URL once it is green,
   as #168's "job name + run log" evidence line requires.
2. **[SUGGESTION]** `tests/distro/debian_ubuntu_test.sh:178`, `tests/docker/test_packages.sh:44` —
   the CI release build runs `cargo build --release --workspace` without `--locked`, which
   contradicts the workflow's "Cargo.lock enforced (--locked)" header. A stale `Cargo.lock` would
   be silently re-resolved in `package-deploy` instead of failing. Add `--locked`.
3. **[SUGGESTION]** `tests/distro/debian_ubuntu_test.sh:255` —
   `pam-auth-update --package --enable soos soos-notify || true` ignores failure. DV1 now cites
   this job for "pam-auth-update configuration", but the facial/password checks run against the
   hand-written `test-soos-debian` stack. The real `pam-auth-update` activation is asserted (rc=0
   plus order) by `pam-rollback` D2/D3, so coverage exists. Drop the `|| true`, or cite D2/D3 in
   DV1.
4. **[SUGGESTION]** `scripts/uninstall.sh:407-408` — if a `find ... -exec rm` fails under
   `set -e`, the script aborts before printing the "PAM rollback is INCOMPLETE" guidance. Consider
   tolerating the purge error and still emitting the final PAM status.

## 5. Final Verdict

All blocking round-1 findings are resolved in code, and the deferred items are documented as
follow-ups. The mechanical listing shows no weakened test assertion (only the approved setup
migration). The rework introduces no new blocking defect.

**VERDICT: APPROVED**

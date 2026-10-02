# Candid Review Report

- **Date**: 2026-10-03
- **Target Branch**: `chore/upgrade-procedure`
- **Base (merge-base)**: `bcb73de`
- **Reviewed-Diff-Fingerprint**: `c868d58bed5f23a87fe8aafe982886f0712aa6b549d00dfc91c5765c0c8efd35`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_upgrade_procedure.md`, `AI/walkthroughs/177_upgrade_procedure.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `README.md`, `packaging/debian/postinst`, `scripts/install.sh`, `tests/docker/systemd_unit_acceptance_test.sh`, `tests/docker/test_packages.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/upgrade_procedure_contract.rs`

## 1. Executive Summary

GitHub #327 documents and tests the upgrade path. There are two production changes:

- **`scripts/install.sh`**: step 8 records the unit's `is-enabled` and `is-active` state before it enables the unit. Step 9 restarts a unit that was already active, with or without `--start`, and waits for readiness. The rollback disables only a unit that this run enabled.
- **`packaging/debian/postinst`**: a profile is treated as disabled by the administrator when it is listed in `/var/lib/pam/seen` but no state file has its `Module:` line. In that case the postinst only refreshes the stack with `pam-auth-update --package` and does not enable anything.

I reviewed the raw diff. I also ran the postinst PAM block under `dash` against the real `pam-auth-update` in an Ubuntu noble container (`mcr.microsoft.com/playwright:v1.59.1-noble`). The results:

| Scenario | Result |
|---|---|
| First install | Both profiles enabled |
| Upgrade with both profiles enabled | Stack and `/var/lib/pam` byte-identical |
| `soos-notify` disabled | Stays disabled |
| Both profiles disabled | Byte-identical, stays disabled |
| `--remove` (prerm), then reinstall | `seen` loses both profiles, so they are re-enabled |
| `seen` missing | Profiles enabled |

The `pam-auth-update` source also confirms that `seen` is written only after the "Local modifications … not updating" exit. A failed first enable therefore cannot make the postinst think the administrator disabled the profiles.

No existing assertion was changed. The gates are green. No CRITICAL or MAJOR finding. There are two MINOR items and one SUGGESTION.

Gates run by the reviewer on this tree:

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| `cargo test --locked -p soos-invariants --all-features` | 410 passed, 0 failed |
| `cargo deny --locked check` | advisories, bans, licenses and sources ok |
| `bash -n` on the three bash scripts | ok |
| `sh -n packaging/debian/postinst` | ok |
| `dash` execution of the postinst PAM block (container) | ok |
| `shellcheck -S warning` | only the existing SC2034 (`BOLD`, `install.sh:86`); nothing new |

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/docker/systemd_unit_acceptance_test.sh`, `tests/docker/test_packages.sh`, `tests/invariants/src/lib.rs` (adds `mod upgrade_procedure_contract;` only) and the new file `tests/invariants/src/upgrade_procedure_contract.rs`.
- Removed or changed assertions (`^-.*assert|#[test]|…`): **none**.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): **none**.
- `mod tests` changes: **none**.
- All 19 removed lines in the patch are listed below. None of them removes or loosens a check:
  - Two harness lines in `systemd_unit_acceptance_test.sh`: the stage list in `--help`, and the `Unknown stage` message. Both now include `upgrade`.
  - The postinst `--enable` block, which moved unchanged into the `else` branch. Its #281 warning text is identical, and the existing #281 invariants still pass.
  - `install.sh` lines: help text, the rollback guard (now also requires `UNIT_WAS_ENABLED != true`) and the step 9 start block (now a restart or a start).
  - Two Docs lines that were reworded.
- Would the new tests fail on the old code?
  - The Docker part 6 compares the `MainPID` and the `/proc/<pid>/exe` target before and after the upgrade. The old `install.sh` does not restart the daemon, so the PID stays the same and the test fails. The walkthrough reports this failure, and the code confirms it.
  - The Debian "disabled" case fails on the old postinst: its `--enable` rewrites `common-auth` and `/var/lib/pam/auth`. I reproduced this in the container.
  - The invariant postinst tests run the real block extracted from the postinst, with a stub that records the `pam-auth-update` arguments. The old block always passes `--enable`, so the test fails on it.

## 3. Deep Reasoning Audit

### Logic & Architecture

- **`install.sh` unit states**:
  - Unit absent before the run: `is-enabled` and `is-active` are false, so the behaviour is the old one.
  - Masked unit: `is-enabled` is non-zero, and `systemctl enable` fails under `set -e`. The rollback then sees `UNIT_ENABLED=true` and `UNIT_WAS_ENABLED=false`, so it runs `disable` (a no-op), which matches the old behaviour.
  - Failed or inactive unit: no implicit restart; `--start` still starts it.
  - `activating` (auto-restart loop): `is-active` is false, so no implicit restart. The next automatic restart runs the new binary anyway.
  - **PASS**.
- **`--destdir`, `--skip-systemd`, no `/run/systemd/system`, no `systemctl`**: step 8 is skipped and `UNIT_ENABLED` stays false. The new step 9 condition `UNIT_WAS_ACTIVE && UNIT_ENABLED` is then false, so no `systemctl` call is made. Under `--destdir` nothing touches the host. **PASS** (but see the MINOR item about `--skip-systemd`).
- **Rollback after each step**:
  - A failure in steps 1–7 leaves `UNIT_ENABLED` false, so the rollback is the old one.
  - A failed `daemon-reload` happens before `UNIT_ENABLED=true`.
  - A failed `systemctl enable` on a unit that was enabled before no longer disables it. This is the fix.
  - `COMMITTED=true` is set right after the enable, so a restart or readiness failure exits 70 without a rollback. This keeps the documented contract.
  - Fail-closed preflight and rollback: no regression.
  - **PASS**.
- **Capture order**: the state is captured after step 5 has replaced the unit file. `is-enabled` reads the `[Install]` symlinks, which the replacement does not touch. **PASS**.
- **postinst detection** (POSIX):
  - `grep -qsx` is POSIX, and the profile names are matched as whole lines, so `soos` never matches `soos-notify`.
  - A missing state file with no match gives status 2, which keeps the profile. A missing `seen` enables the profiles (first install).
  - Upgrade from a version that had no `soos-notify` profile, with `soos` still enabled: both are enabled, as before.
  - `soos` disabled and `soos-notify` new: kept, and `--package` does not enable `soos-notify` (`Default: no`).
  - A profile dropped by a conflict: treated as disabled. The old `--enable` would not have selected it either.
  - Detecting a profile as disabled when it is not never enables anything, so a wrong detection cannot re-enable a profile the administrator disabled.
  - **PASS**.
- **Docs accuracy**: I checked the following against the code.
  - Output paths and names: `build_deb.sh` → `target/packages/soos_<version>_<arch>.deb`; `build_arch.sh` → `soos-<version>-<release>-<arch>.pkg.tar.zst`; RPM in `target/packages`.
  - RPM `%systemd_postun_with_restart` and `%posttrans`; Arch `post_upgrade` → `post_install`.
  - The Debian postinst does not restart the daemon, so the docs are right to require `systemctl restart`.
  - `provision_master_key.sh` re-applies `0600 root:root` to an existing key.
  - `download_models.sh` keeps files whose checksum already matches.
  - `uninstall.sh --keep-data` exists and removes `/etc/systemd/system/soos-daemon.service`.
  - `soos-enroll enroll --username` exists, and the `[WARN] UID N: … re-enroll` text matches `main.rs`.
  - Same-version caveats are correct: `dpkg -i` and `pacman -U` reinstall, and `dnf upgrade` skips the same version.
  - The StateDirectory group note matches the harness's owner-and-mode-only comparison.
  - **PASS**.

### PAM Concurrency & Deadlines

- No Rust or PAM module code changed. **PASS** (not applicable).
- During the restart window PAM falls back to `PAM_IGNORE` because the socket is unavailable. This is unchanged.

### Panic Safety & Fail-Closed

- No path turns an error into `PAM_SUCCESS`.
- The postinst never forces the stack, and a `pam-auth-update` failure is reported, never fatal (as in #281).
- A failed restart in `install.sh` exits 70 after the commit and points to the journal.
- **PASS**.

### Test Integrity & Anti-Weakening

- No existing assertion was changed or removed (§2).
- The Docker part 6 asserts byte-identity of the key, the template, `daemon.toml`, `/etc/pam.d`, `/var/lib/pam` and the PAM snapshot. It also asserts that the unit is enabled, a new `MainPID`, an exe path that is not `(deleted)`, a `cmp` against the release binary, and `is_healthy`.
- `test_packages.sh` diffs SHA-256 digests (masked in the output) around `dpkg -i`, `rpm -U --replacepkgs` and `pacman -U`.
- In both harnesses the digest helpers fail on any difference.
- **PASS**.
- Note: I did not re-run the full privileged systemd harness. The reviewer's container run of the postinst logic independently confirms the Debian half.

### Memory, Bounds & Secrets

- The harnesses compare digests and never print key or template bytes. The digests are masked in mismatch output.
- The synthetic template is created under `umask 077`.
- No new file is created insecurely by production code.
- **PASS**.

### Supply Chain & Automation

- No changes to `Cargo.*`, `deny.toml`, `.github/` or `.githooks/`. `cargo deny` is clean.
- The Docker stage is wired into the existing CI jobs (`systemd-unit` with `--models download`, `package-deploy`, `distro-deploy`).
- **PASS**.

### English-Only Policy

- I grepped the added lines for accented characters and common French words: no hits. **PASS**.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/install.sh:874` — If the daemon is running but systemd handling is skipped (`--skip-systemd`, or `systemctl` unusable), the re-run replaces the binaries and leaves the old, `(deleted)` binary running, with no message. Both help texts (`install.sh` and Docs §1) say "an already active unit is restarted on the new binaries even without it". Correction: in the live-install path, when `UNIT_ENABLED != true` and `systemctl is-active --quiet soos-daemon.service` succeeds, print `warn "soos-daemon.service still runs the replaced binary; run: sudo systemctl restart soos-daemon"`. Alternatively, add "(not with --skip-systemd)" to the help text and to Docs §1 and §9.2.
- **[MINOR]** `packaging/debian/postinst:72` — In the keep branch (a profile disabled by the administrator), the #281 "`common-auth` does not call `pam_soos.so`" warning is skipped, and so is any hint about how to re-enable the profiles. This is correct, but the only output is "keeping the administrator's PAM profile selection". Correction (optional wording): add "re-enable with: pam-auth-update --enable soos soos-notify" to that message, so an administrator who forgot the earlier `--disable` knows why face authentication is inactive.
- **[SUGGESTION]** `tests/invariants/src/upgrade_procedure_contract.rs:37` — `scratch_dir` leaves `soos_upg_<tag>_<pid>` directories in the temp directory after each run. Remove them at the end of `run_postinst_pam_block`, after the log has been read.

## 5. Final Verdict

**VERDICT: APPROVED**

# Walkthrough 177 — Documented and Tested Upgrade Path for Local Installs

- **Date**: 2026-10-03
- **Issue**: GitHub #327 (GitHub-only, no backlog sub-issue; the branch is not registered in
  `scripts/sync_issue.py`, the commit carries `Closes #327`) — **Branch**: `chore/upgrade-procedure`
- **Matrix criteria**: UPG1–UPG9 (`AI/VERIFICATION_MATRIX.md` section `upgrade-procedure`), all `✅ Verified`
  (UPG6 runs in CI on push to `main`; verified locally)
- **Phase documents**: `AI/architect_spec_upgrade_procedure.md`, `AI/plan_evaluator_report.md`
  (round 2, `APPROVED`)

## 1. Context & Objectives

Users update soos by pulling `main` and reinstalling, but there was no upgrade section in the README or
the Docs, and nothing proved that re-running `scripts/install.sh` (or installing a newer package) keeps
the master key, the enrolled templates, `/etc/soos/daemon.toml` and the PAM activation state, and leaves
the daemon healthy.

## 2. Findings (read from the code, then confirmed in Docker)

| Method | Result before this change |
|---|---|
| `scripts/install.sh` re-run (`--build` too) | Key, templates, `daemon.toml`, `/etc/pam.d` and the PAM snapshot are kept; models are only re-verified. **Defect D1**: the running daemon is never restarted — `--start` runs `systemctl start`, a no-op on an active unit — so it keeps executing the replaced (deleted) binary, and the readiness wait passes against that old process. **Defect D2**: if `systemctl enable` fails, the rollback disables a unit that was enabled before the run. |
| `.deb` (`dpkg -i` newer over older) | Key, templates and `daemon.toml` kept. **Defect D3**: the `postinst` always runs `pam-auth-update --package --enable soos soos-notify`, and `--enable` pushes the profile into the enabled list unconditionally, so an administrator's `pam-auth-update --disable soos soos-notify` was undone by every upgrade. The daemon is not restarted (no `#DEBHELPER#` snippet). |
| Arch (`pacman -U`) | `post_upgrade` = `post_install`; key kept, `/etc/pam.d` untouched; daemon not restarted. |
| RPM (`dnf upgrade` / `rpm -U`) | Key kept (also from a legacy `%ghost` owner, #281), authselect never selected automatically, `%systemd_postun_with_restart` try-restarts the daemon. An active `custom/soos` profile needs `authselect apply-changes` to pick up a changed template. |

Side observation (not changed): systemd re-owns the `StateDirectory=soos` tree to `root:soos` at each
daemon start (the unit has `Group=soos`), while the installers set `root:root`. Modes stay `0600` /
`0700`, so the group gains no access; the harness compares owner and mode, not the group.

## 3. Production Changes

- `scripts/install.sh`: before step 8 the unit state is recorded (`systemctl is-enabled --quiet`,
  `systemctl is-active --quiet`). Step 9 now also runs when the unit was active: `systemctl restart`
  for an active unit, `systemctl start` otherwise, then the same bounded `scripts/wait_daemon_ready.sh`
  wait (exit 70 if not ready; the install stays committed). The rollback disables the unit only when this
  run enabled it (`UNIT_WAS_ENABLED`).
- `packaging/debian/postinst`: a profile listed in `/var/lib/pam/seen` without a `Module: <name>` line
  in the pam-auth-update state files is one the administrator disabled; if `soos` or `soos-notify` is in
  that state, the postinst runs `pam-auth-update --package` (refresh, no `--enable`) and keeps the
  selection. First install, reinstall after `dpkg -r` (`--remove` drops the profiles from `seen`) and an
  upgrade with both profiles enabled still run `pam-auth-update --package --enable soos soos-notify`.

## 4. Tests

- `tests/docker/systemd_unit_acceptance_test.sh` part 6 (new stage `upgrade`, CI job `systemd-unit`):
  previous install with the Debian profiles activated by `pam-auth-update`, a `daemon.toml`, a synthetic
  template `4242.cbor.enc` and a running daemon; then a plain re-run and a `--start` re-run. Asserts
  byte-identical key / template / `daemon.toml` / `/etc/pam.d` / `/var/lib/pam` / PAM snapshot, unit still
  enabled, new `MainPID`, `/proc/<pid>/exe` = `/usr/libexec/soos/soos-daemon` (not `(deleted)`), release
  binary installed, `is_healthy`.
- `tests/docker/test_packages.sh`: `verify_upgrade_preserves_state` around `dpkg -i` of a version-bumped
  copy of the `.deb` (`dpkg-deb -R` / `-b`, no second build) with soos enabled and with soos disabled by the
  administrator (the image's hand-written `common-auth` is handed to pam-auth-update for this case and
  restored afterwards), `rpm -U --replacepkgs` and `pacman -U`.
- `tests/invariants/src/upgrade_procedure_contract.rs`: postinst behaviour with a recording
  `pam-auth-update` stub (admin-disabled, first install, after remove, both enabled), install.sh restart
  and rollback guards, harness coverage, README and Docs sections.

## 5. Documentation

- README `## Upgrade`: command-only procedure per method and the verification commands.
- `Docs/PACKAGING_AND_PROVISIONING.md` §9: procedure, preserved / not preserved state per method, same
  version reinstalls, switching from an `install.sh` install to a package, evidence.

## 6. Verification Evidence

| Check | Before the fix (Red) | After the fix (Green) |
|---|---|---|
| `tests/docker/systemd_unit_acceptance_test.sh --models host` | Parts 1–5 pass; part 6 fails: `Part 6 (re-run): MainPID 603 is unchanged: the daemon was not restarted onto the new binary` | Passes: `MainPID 609 -> 902` (re-run) and `902 -> 1211` (`--start`), exe `/usr/libexec/soos/soos-daemon`, `is_healthy`; key, template, `daemon.toml`, `/etc/pam.d`, `/var/lib/pam`, PAM snapshot byte-identical, unit enabled |
| `test_packages.sh` in `soos-distro-val-ubuntu` | `dpkg -i newer .deb (soos enabled)` passes; `dpkg -i .deb again (soos disabled by the administrator)` fails: `/etc/pam.d/common-auth` and `/var/lib/pam/auth` changed (soos re-enabled) | Passes: the postinst prints "keeping the administrator's PAM profile selection" and the state is byte-identical; the rest of the Debian branch unchanged |
| `test_packages.sh` in `soos-distro-val-fedora` / `-arch` | not run before the fix (no production change on those paths) | Pass: `rpm -U --replacepkgs` and `pacman -U` reinstalls keep the state byte-identical; existing RPM `%ghost` upgrade case still passes |
| `cargo test -p soos-invariants upgrade_procedure` | 6 of 7 fail (`test_upg_postinst_enables_profiles_on_first_install` passes: unchanged behaviour) | 7 / 7 pass; the whole invariants suite (410) passes |

The first harness iterations also surfaced two test-side corrections, made before any production change:
a `diff | sed` pipeline aborted the harness silently under `pipefail` (now `{ diff || true; }`), and the
group of `master.key` / `biometrics/` flips to `soos` when systemd starts the unit (section 2), so the
harness compares owner and mode only. The package image ships a hand-written `common-auth` that
pam-auth-update never rewrites, so the Debian case hands the stack to pam-auth-update (`--force`, test
container only) and restores the image's files afterwards.

## 7. Owner Decisions

- OD1: `.deb` and Arch upgrades do not restart the daemon (RPM does). Documented as
  `sudo systemctl restart soos-daemon`; adding `systemctl try-restart` to `postinst` (when `$2` is set) and
  to `post_upgrade` is left to the owner.
- OD2: `/var/lib/soos` group ownership flips between `root:root` (installers) and `root:soos` (systemd
  `StateDirectory=` with `Group=soos`); harmless with the current modes, but the documented
  `root:root` owner of `master.key` is only true until the first daemon start.

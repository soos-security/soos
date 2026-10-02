# Architect Spec — Documented and Tested Upgrade Path for Local Installs

- **Issue**: GitHub #327 (GitHub-only, no `AI/BACKLOG.md` entry; the commit carries `Closes #327`)
- **Branch**: `chore/upgrade-procedure` (not registered in `scripts/sync_issue.py`)
- **Base commit**: `bcb73de`
- **Phases**: condensed 1–4 (shell/packaging/docs change; no Rust production code)

## 1. Findings — What an Upgrade Does Today (read from the code)

### 1.1 `scripts/install.sh` (and `--build`) re-run over an existing live install

| Step | Behaviour on a re-run | Upgrade-safe? |
|---|---|---|
| Preflight, `--build` | Read-only; `--build` compiles as the invoking sudo/doas/pkexec user | yes |
| 1 group | `getent group soos` → skipped | yes |
| 2 directories | `ensure_dir ... owned` re-applies 0755/0700 and `root:root` to the **directories** only; files inside are not touched | yes |
| 3 master key | `provision_master_key.sh`: an existing regular key is kept (mode re-tightened to 0600 `root:root`), never regenerated; not journaled, so a rollback never deletes it | yes |
| 4 binaries, PAM module | `tracked_install`: temp file + `rename(2)` (a running process keeps its old inode), previous file backed up for rollback | yes |
| 5 unit | same atomic replace of `/etc/systemd/system/soos-daemon.service` | yes |
| 6a PAM snapshot | an existing pre-install snapshot is never overwritten (it holds the pristine state used by `uninstall.sh`) | yes |
| 6 PAM templates | only template files are replaced (`/usr/share/pam-configs/soos*`, `/etc/authselect/custom/soos/*`, Arch snippet); `pam-auth-update` / `authselect select` are never run, `/etc/pam.d` is never written. Fedora: `authselect.previous` is re-recorded only when the current profile is not `custom/soos` | yes |
| `/etc/soos/daemon.toml` | never written by the installer | yes |
| `/var/lib/soos/biometrics/*` | never read or written | yes |
| 7 models | files already present with the attested SHA-256 are kept (no download); `manifest.toml` is re-copied | yes |
| 8 enable | `daemon-reload` + `enable` (idempotent) | yes |
| **8 rollback** | **if `systemctl enable` itself fails, the rollback runs `systemctl disable` although the unit was already enabled before the run: a failed re-run leaves a previously enabled unit disabled** | **no (D2)** |
| **9 start** | **only with `--start`, and it runs `systemctl start`, a no-op on an active unit: the daemon keeps running the replaced (deleted) binary, and the readiness wait passes against that old process. Without `--start` nothing restarts the daemon either** | **no (D1)** |

### 1.2 Debian / Ubuntu `.deb` installed over an older one (`dpkg -i`)

`prerm upgrade` and `postrm upgrade` do nothing; files are replaced by dpkg; `postinst configure <old-version>`
re-runs the first-install steps: directories re-moded, `provision-master-key` keeps the key, `systemctl enable`
(idempotent). `/etc/soos/daemon.toml` and `/var/lib/soos/biometrics` are not package files and are untouched.
The maintainer scripts have no `#DEBHELPER#` token, so no debhelper restart snippet exists.

- **D3 (defect)**: `postinst` always runs `pam-auth-update --package --enable soos soos-notify`. In
  `pam-auth-update` an `--enable` profile is pushed into the enabled list unconditionally, so an administrator
  who disabled facial login with `pam-auth-update --disable soos soos-notify` (or in the interactive menu) gets
  it **re-enabled by every upgrade**. That changes the PAM activation state.
- **O1 (not a defect, documented)**: the running daemon is not restarted; it keeps the old binary until
  `sudo systemctl restart soos-daemon`.

### 1.3 Arch `pacman -U` over an installed package

`pre_remove` is not called on an upgrade; `post_upgrade` → `post_install`: directories re-moded, key kept,
`daemon-reload`. pacman never edits `/etc/pam.d` (the snippet is reference material), `/etc/soos/daemon.toml`
and the templates are not package files. **O1** applies: the daemon is not restarted.

### 1.4 Fedora RPM upgrade (`dnf upgrade ./soos-*.rpm` / `rpm -U`)

`%pre` (`$1 = 2`) keeps a private copy of a legacy `%ghost`-owned key, `%posttrans` restores it (GitHub #281);
`%post` keeps the key; `%preun`'s authselect restore runs only on erase (`$1 = 0`); the authselect profile is
never activated automatically. `%postun` runs `%systemd_postun_with_restart`: the daemon is **try-restarted**
on the new binary. `/etc/authselect/custom/soos/*` is replaced: when `custom/soos` is the active profile the
generated `/etc/pam.d` files keep the old template until `authselect apply-changes` (documented).

## 2. Production Fixes (minimal)

### F1 — `scripts/install.sh` restarts an active daemon onto the new binaries (D1)

Live install with systemd, before step 8: record `UNIT_WAS_ENABLED` (`systemctl is-enabled --quiet`) and
`UNIT_WAS_ACTIVE` (`systemctl is-active --quiet`). Step 9 runs when `--start` was given **or** the unit was
active: `systemctl restart soos-daemon.service` when it was active, `systemctl start soos-daemon.service`
otherwise, then the existing bounded `scripts/wait_daemon_ready.sh` wait (same exit 70 on failure; the install
stays committed). An inactive unit without `--start` is left inactive (unchanged behaviour).

### F2 — the rollback only disables a unit this run enabled (D2)

`rollback()` runs `systemctl disable` only when `UNIT_ENABLED = true` **and** `UNIT_WAS_ENABLED = false`.

### F3 — `postinst` keeps an administrator's disabled profile (D3)

Step 4 of `packaging/debian/postinst`: a profile counts as disabled by the administrator when it is listed in
`/var/lib/pam/seen` (pam-auth-update knows it) and no `Module: <name>` line exists in any
`/var/lib/pam/{auth,account,password,session,session-noninteractive}` (it is not enabled). `--remove`
(our `prerm remove`) deletes the profile from `seen`, so a reinstall after `dpkg -r` is a first install again.

- No profile disabled → unchanged: `pam-auth-update --package --enable soos soos-notify` and the GitHub #281
  warning when `common-auth` does not call `pam_soos.so`.
- Any profile disabled → `pam-auth-update --package` (no `--enable`): refreshes the stack from the current
  selection (picks up changed profile text for enabled profiles) and keeps every administrator choice; an info
  line says so. Never `--force`, never `|| true` (QFU4 stays green).

## 3. Acceptance Criteria

| ID | Criterion | Evidence |
|---|---|---|
| UPG1 | Re-running `scripts/install.sh` over a live install keeps `/var/lib/soos/master.key`, an enrolled template (`/var/lib/soos/biometrics/<uid>.cbor.enc`), `/etc/soos/daemon.toml`, `/etc/pam.d/*`, `/var/lib/pam/*` and the PAM snapshot byte-identical, with key mode `600` and `biometrics/` mode `700`, owned by root | `systemd_unit_acceptance_test.sh` stage `upgrade` (`part6_reinstall_preserves_state`) |
| UPG2 | After the re-run the unit is still enabled; an active daemon is restarted on the new binary (new `MainPID`, `/proc/<pid>/exe` is the installed path, not `(deleted)`), and comes back healthy (`wait_daemon_ready.sh`, `is_healthy`); both a plain re-run and `--start` | same stage, models `host`/`download` |
| UPG3 | `install.sh` restarts an active unit and its rollback never disables a unit that was enabled before the run | `upgrade_procedure_contract::test_upg_install_restarts_an_active_unit`, `test_upg_install_rollback_keeps_a_previously_enabled_unit` |
| UPG4 | `.deb` upgrade (`dpkg -i` of a newer version over the installed one) keeps key, template, `daemon.toml` and `/etc/pam.d` byte-identical, both with soos enabled and with soos disabled by the administrator | `test_packages.sh` `verify_upgrade_preserves_state` (deb branch) |
| UPG5 | `postinst` never re-enables a profile the administrator disabled; it still enables both profiles on a first install and after `dpkg -r` | `upgrade_procedure_contract::test_upg_postinst_keeps_admin_disabled_profiles`, `test_upg_postinst_enables_profiles_on_first_install` |
| UPG6 | `pacman -U` and `rpm -U --replacepkgs` over an installed package keep key, template, `daemon.toml` and `/etc/pam.d` byte-identical | `test_packages.sh` arch / RPM branches (push to `main`) |
| UPG7 | README has an `## Upgrade` section with the commands per method (.deb, Arch, RPM, `install.sh --build`), `systemctl restart soos-daemon`, `soos-admin status`, `soos-enroll list`, re-enroll only on a foreign template | `test_upg_readme_documents_upgrade_per_method` |
| UPG8 | `Docs/PACKAGING_AND_PROVISIONING.md` §9 documents what is preserved / not preserved per method and the switch from an `install.sh` install to a package | `test_upg_packaging_doc_documents_preservation_and_switch` |
| UPG9 | The Docker harnesses keep the upgrade cases (static guard) and the systemd job runs them in CI | `test_upg_docker_harnesses_cover_upgrade` |

## 4. Tests (Phase 2) and CI Wiring

- **Systemd harness**: new stage `upgrade` run after `after-reboot` in the same container (CI job
  `systemd-unit`, `--models download`): no extra build, two `install.sh` runs and two daemon restarts
  (~20 s). Without models the preservation half still runs; the health half is skipped with a `SKIPPED` notice.
- **Package harness**: deb branch runs on every PR (`package-deploy`); the version-bumped `.deb` is produced by
  `dpkg-deb -R` / `dpkg-deb -b` of the built package (no second build). RPM/Arch reinstall cases run on push
  to `main` (`distro-deploy`), like the existing RPM upgrade case.
- **Static invariants**: new module `tests/invariants/src/upgrade_procedure_contract.rs`.
- Synthetic template: 512 random bytes at `/var/lib/soos/biometrics/4242.cbor.enc` (mode 0600). No camera.

## 5. Security Invariants Touched

- Master key never regenerated, never logged (only `sha256sum` digests of fixtures are compared, the key
  digest is never printed).
- No `--force` to `pam-auth-update`; PAM never activated by `install.sh`; `/etc/pam.d` never written by it.
- Restart window: during `systemctl restart` PAM returns `PAM_IGNORE` (password fallback) — fail-safe.

## 6. Owner Decisions (not implemented)

- **OD1**: `.deb` / Arch upgrades do not restart the daemon (RPM does). Option: `systemctl try-restart` in
  `postinst` when `$2` is set and in `post_upgrade`. Documented as a manual `systemctl restart` for now.
- **OD2** (found by the Docker run): systemd re-owns the `StateDirectory=soos` tree to `root:soos` at each
  daemon start (`Group=soos`), the installers re-apply `root:root`. Modes stay `0600` / `0700`, so the
  group gains no access; the harness compares owner and mode only. Whether the documented `root:root`
  owner should be enforced (e.g. a dedicated `Group=` for the socket only) is left to the owner.

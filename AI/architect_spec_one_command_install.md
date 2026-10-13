# Architect Spec — GitHub #372: `scripts/setup.sh` core (PR 1 of #362)

- **Date**: 2026-10-13
- **Issue**: GitHub-only #372 (PR 1 of #362, planned split item 1); no `AI/BACKLOG.md` id.
- **Branch**: `feat/one-command-install` (not registered in `scripts/sync_issue.py`; the commit carries `Closes #372`).
- **Base commit**: `37d03c4` (`origin/main`).
- **Owner decisions** (#362 comments of 2026-10-12 and 2026-10-13, restated by the owner for this run):
  - OD-1 Name: `scripts/setup.sh` now; the packages later ship it as `/usr/bin/soos-setup`.
  - OD-2 Enrollment from a local seat only. When `loginctl` reports a remote session, or no seat, setup skips enrollment and prints the exact command to run at the PC.
  - OD-3 The default install enables everything (presence auto-unlock, phone companion, Web Push, Funnel, camera view), each toggleable later in the GUI and explained by the onboarding tour (#363). This reverses the opt-in defaults of the installer path and needs an ADR amendment. In this PR only the face profile applies.
  - OD-4 Steps that need the owner's Tailscale account (login, Funnel policy attribute) are guided, never silent.
  - OD-5 PAM activation keeps its safety checks (test before activation, rollback). Not in this PR.
  - OD-6 Agents never install anything on the host, never run `sudo` and never run `setup.sh` for real; tests use a fake root and stub commands.

## 1. Scope & Blast Radius

| Area | Change |
|---|---|
| `scripts/setup.sh` | **new**, mode 0755: detect, plan, `--dry-run`, packages, toolchain, build, system install, access, presence flag, verify gate, local-seat enrollment, bounded state file, rollback (§2–§6) |
| `tests/invariants/src/setup_contract.rs` + `lib.rs` | **new** static and behavioral contracts (§9) |
| `README.md` | "Install" section leads with `./scripts/setup.sh` (manual procedure kept below) |
| `Docs/PACKAGING_AND_PROVISIONING.md` | new §3.0 "One-Command Setup (`scripts/setup.sh`)" |
| `AI/DECISIONS.md` | new ADR (OD-1..OD-6, defaults amendment, rollback model) appended at the end |
| `AI/VERIFICATION_MATRIX.md` | new component section, rows `SET1`–`SETn` (prefix unused today), owner hardware row |
| `AI/walkthroughs/200_one_command_install.md` | walkthrough (196–199 are taken by parallel branches) |

**Untouched**: every crate, `scripts/install.sh`, `scripts/install_remote.sh`, `scripts/uninstall.sh`, `scripts/check_build_deps.sh`, `scripts/wait_daemon_ready.sh`, `scripts/camera_report.sh`, `scripts/install_rustup.sh`, `packaging/**`. `setup.sh` only **calls** them. No new binary, no new config key, no new dependency. `uninstall.sh` needs no change: `--purge-data` already removes `state/` entries other than `pam-backup`, and `--keep-data` keeps the state file with the rest of `/var/lib/soos` (documented).

No consumer of a public Rust item changes. Existing installer invariants (`installer_contract.rs`, `onboarding_packaging_contract.rs`, `remote_*_contract.rs`, `network_lock_contract.rs`) stay unchanged.

## 2. Command Line

```
scripts/setup.sh [OPTIONS]
```

| Option | Value | Default | Meaning |
|---|---|---|---|
| `-y`, `--yes` | — | off | Non-interactive: no confirmation prompt. Without it, a TTY on stdin and stdout is required and the plan is confirmed with `y`; without a TTY and without `--yes` → exit 2 before any change |
| `--dry-run` | — | off | Detect, print the full plan, exit. No write, no `sudo`/`doas`, no build, no package manager write, no temp file |
| `--user <NAME>` | POSIX name, `^[a-z_][a-z0-9_-]*[$]?$`, ≤ 32 chars, must resolve with `getent passwd`, never `root` / uid 0 | invoking user | Account added to `soos` and enrolled; owns the build |
| `--no-enroll` | — | enroll | Skip enrollment (the command is printed) |
| `--presence <on\|off>` | `on` or `off` | unset (= keep) | `off` creates `/etc/soos/presence.disable`; `on` removes it; unset keeps the current state and reports it (the daemon default is on; an existing flag is never removed silently) |
| `--skip-packages` | — | off | Do not install packages (the missing list and command are still printed) |
| `--distro <FAMILY>` | `debian`, `fedora`, `arch` | auto | Package family override (same vocabulary as `install.sh --distro`, minus `none`) |
| `--prefix`, `--sysconfdir`, `--localstatedir`, `--runstatedir` | absolute path | `/usr`, `/etc`, `/var`, `/run` | Passed through to `install.sh`; every path setup reads or writes derives from them (tests point them into a fake root) |
| `--pam-dir <DIR>` | absolute path | auto (same candidate list as `install.sh`) | Passed through to `install.sh` |
| `-h`, `--help` | — | — | Usage, every flag, exit codes, then exit 0 |
| `--system-phase` | — | — | Internal: the root half, re-executed through `sudo`/`doas` by the user half. Refused (exit 2) when not root or without `--user` |

Validation is complete **before** anything runs: an unknown option, a missing value, a relative path, `--presence` other than `on|off`, an invalid `--user`, or `--distro` outside the list → exit 2 with a message and usage on stderr.

Not in this PR (later PRs of #362, each recorded in the ADR): `--remote`, `--push`, `--camera-view`, `--funnel`, `--remote-unlock`, `--tailscale-serve`, `--activate-pam`, `--gdm`, `--profile`, `--from-release`, `--uninstall`, `--network-lock`, `--format json`.

## 3. Constants (all `readonly` in `scripts/setup.sh`)

| Name | Value | Meaning / bound |
|---|---|---|
| `SETUP_STATE_FORMAT` | `1` | `format` field of the state file; any other value is invalid |
| `MAX_STATE_BYTES` | `4096` | the state file is read with `head -c $((MAX_STATE_BYTES + 1))`; a larger file is invalid |
| `READY_TIMEOUT_S` | `30` | `wait_daemon_ready.sh --timeout` in the verify gate |
| `CAMERA_TIMEOUT_S` | `15` | bound of `soos-admin camera list` (coreutils `timeout --kill-after=2`) |
| `LOGINCTL_TIMEOUT_S` | `5` | bound of `loginctl show-session` |
| `MAX_USER_LEN` | `32` | `--user` length bound (same as `soos-admin add-user`) |
| exit codes | `0` done, `1` runtime failure (setup's own changes rolled back), `2` usage or preflight error (nothing changed), `3` installed but enrollment failed (nothing rolled back, retry command printed), `40` build failed (nothing installed), `70` installed but the daemon is not ready (verify gate failed; ADR 2026-09-30 exit-70 semantics, nothing rolled back) |

## 4. Phases

Every phase prints one line `[PLAN] <phase>: <status> - <detail>` in the plan and one line `[DONE]`, `[SKIP]` or `[FAIL]` when it runs. Status vocabulary: `will run`, `already done`, `skipped (<reason>)`, `checked at run time (needs root)`.

### 4.0 detect (read-only, always, also in `--dry-run`)

| Item | Source | Rule |
|---|---|---|
| distribution | `${SYSCONFDIR}/os-release`, else `/usr/lib/os-release` | **parsed** line by line (`ID=`, `ID_LIKE=`, `PRETTY_NAME=`), never sourced; quotes stripped; non-printable characters dropped before printing |
| family | `ID` then `ID_LIKE` tokens | `debian ubuntu linuxmint pop` → debian; `fedora rhel centos rocky almalinux` → fedora; `arch archlinux manjaro endeavouros` → arch (same table as `check_build_deps.sh family_of`); none → exit 2 (pass `--distro`) |
| package manager | family | debian `apt-get` + `dpkg-query`; fedora `dnf` + `rpm`; arch `pacman` |
| package lists | `scripts/check_build_deps.sh --distro <family> --print-packages build|models|gui` | the **only** source; setup contains no package name |
| init | `${RUNSTATEDIR}/systemd/system` is a directory | otherwise exit 2 (systemd required) |
| session | `loginctl show-session "${XDG_SESSION_ID:-self}" --property=Remote --property=Seat` (bounded) and `SSH_CONNECTION`/`SSH_TTY` | **local seat** iff the call succeeds, `Remote=no`, `Seat` non-empty and no SSH variable; any error, missing `loginctl` or other value → not local (fail safe) |
| camera | count of character devices `/dev/video*` | informational only in the plan (the enrollment gate uses `soos-admin camera list`, §4.7) |
| existing install | §4.4 comparison | `already done` or `will run` |
| target user | `--user`, else root: `SUDO_USER` / `DOAS_USER` / `PKEXEC_UID` (same function as `install.sh invoking_user`), non-root: `id -un` | root without any → exit 2; uid 0 → exit 2 |

### 4.1 packages (root)

Missing packages = the three groups (build, models, gui) filtered through the package manager query (`dpkg-query -W -f='${db:Status-Status}'` = `installed`; `rpm -q --quiet`; `pacman -Q`). None missing → `already done`. Otherwise the exact command is printed, then run as root: `apt-get install -y --no-install-recommends <pkgs>` (with `DEBIAN_FRONTEND=noninteractive`), `dnf install -y <pkgs>`, `pacman -S --needed --noconfirm <pkgs>`. Never a repository change, never `-Sy`. `--skip-packages` → `skipped (--skip-packages)` with the command printed. A failure → exit 1 (nothing else changed). Packages are never removed by a rollback; the summary lists what was installed.

### 4.2 toolchain (build user)

`cargo` on the build user's `PATH` or `~/.cargo/bin/cargo` executable → `already done`. Otherwise `scripts/install_rustup.sh` runs as the build user (verified `rustup-init`, ADR 2026-09-30 ONB-16). Never removed by a rollback.

### 4.3 build (build user, never root)

`cargo build --release --locked --workspace` in the checkout, as the build user (directly when setup runs as that user; `runuser -u <user> -- env HOME=<home> PATH=<home>/.cargo/bin:<PATH> ...` when setup runs as root), with `SOOS_BINDIR=${PREFIX}/bin` (GitHub #318) and `CARGO_TARGET_DIR` resolved exactly as `install.sh resolve_cargo_target_dir` (relative to the checkout). The artifact directory `<target>/release` is passed to `install.sh --artifact-dir`, so a `sudo` environment reset cannot point `install.sh` at another target. cargo is incremental: a second run rebuilds nothing. Failure → exit 40, nothing installed.

### 4.4 system (root)

`already done` iff all of: `${PREFIX}/libexec/soos/soos-daemon`, `${PREFIX}/bin/soos-admin`, `soos-enroll`, `soos-gui` and `<pam dir>/pam_soos.so` are byte-identical (`cmp -s`) to the artifacts; `${SYSCONFDIR}/systemd/system/soos-daemon.service` is identical to `packaging/soos-daemon.service`; `${LOCALSTATEDIR}/lib/soos/models/manifest.toml` is identical to `models/manifest.toml`; `systemctl is-enabled` and `is-active` succeed for `soos-daemon.service`. Otherwise `scripts/install.sh --start --artifact-dir <dir>` plus the pass-through options. `install.sh` is the authority for its files: it rolls back its own failures. Exit 70 of `install.sh` → setup exit 70 (committed, not ready, nothing rolled back, enrollment not attempted). Any other non-zero → setup exit 1 ("install.sh rolled back its own changes").

### 4.5 access (root)

User in group `soos` (`id -nG <user>`) → `already done`. Otherwise `${PREFIX}/bin/soos-admin add-user <user>`; journaled as `group_added`; the summary says to log out and back in. Failure → rollback (§6), exit 1.

### 4.6 presence (root)

Flag `${SYSCONFDIR}/soos/presence.disable`. `--presence off`: absent → create (empty, `0644`, `${SYSCONFDIR}/soos` created `0755` if absent and journaled), journaled `presence_flag_created`; present → `already done`. `--presence on`: present → removed, journaled `presence_flag_removed` (restored by a rollback); absent → `already done`. Unset: no change; the plan reports `presence auto-unlock: on (daemon default)` or `off (presence.disable present; --presence on enables it)`. A symlinked flag or directory → exit 1 with rollback, never followed.

### 4.7 verify gate (root)

1. `scripts/wait_daemon_ready.sh --socket ${RUNSTATEDIR}/soos/daemon.sock --manifest ${LOCALSTATEDIR}/lib/soos/models/manifest.toml --admin ${PREFIX}/bin/soos-admin --timeout 30`. Failure → exit 70 after the state file is written; enrollment is not attempted; nothing rolled back.
2. Camera report: `${PREFIX}/bin/soos-admin camera list --config ${SYSCONFDIR}/soos/daemon.toml`, bounded by `CAMERA_TIMEOUT_S`; exit 0 → camera available; anything else → `no camera selected` (warning, setup continues, enrollment `skipped (no camera)`).

### 4.8 enrollment (root, local seat only)

Order of checks: `--no-enroll` → `skipped (--no-enroll)`; template `${LOCALSTATEDIR}/lib/soos/biometrics/<uid>.cbor.enc` exists → `already done`; not a local seat → `skipped (remote session or no local seat)`; no camera → `skipped (no camera)`. Otherwise `${PREFIX}/bin/soos-enroll enroll --username <user>` in the foreground. Failure → exit 3 (install and access kept, no rollback). Every skip and failure prints the exact command: `sudo soos-enroll enroll --username <user>` ("run it at the PC, in front of the camera").

### 4.9 state (root)

`${LOCALSTATEDIR}/lib/soos/state/setup.json` (§5). Written last, also on exit 3 and 70.

### 4.10 summary

Done / already done / skipped per phase, packages installed by this run, the re-login notice, presence state, the rescue commands (`sudo touch /etc/soos/disabled`, `sudo ./scripts/uninstall.sh --keep-data`), "PAM is not activated by this version: see Docs/DISTRIBUTION_DEPLOYMENT.md", and the later-PR features as manual pointers (`Docs/REMOTE_COMPANION.md` §4).

### 4.11 Privilege flow

- Setup started as a normal user: detect + plan + confirm as the user; packages through the escalation helper (`sudo --` if present, else `doas --`; neither → exit 2 before any change; never `pkexec`, never `eval`); toolchain + build as the user; then **one** escalation re-executes `bash scripts/setup.sh --system-phase --yes --user <user>` with the pass-through options and the artifact directory (sudo resets the environment; nothing is passed through the environment).
- Setup started as root: the invoking user is required (or `--user`); toolchain and build run through `runuser -u <user>`; the system phase runs inline.
- `--dry-run` never calls the escalation helper, `cargo`, `install.sh`, `install_rustup.sh`, a package manager write, `soos-admin add-user`, `soos-enroll`, `mktemp` or any write.

## 5. State File

Path `${LOCALSTATEDIR}/lib/soos/state/setup.json`, directory created `0700` if absent, file `0600`, owner root. Content is generated only from validated values (no free text, no secret, no timestamp so that a second identical run writes nothing):

```json
{
  "format": 1,
  "tool": "soos-setup",
  "profile": "face",
  "user": "<validated user name>",
  "group_added_by_setup": true,
  "presence_flag_created_by_setup": false,
  "presence": "on",
  "enrolled": true
}
```

- `group_added_by_setup` and `presence_flag_created_by_setup` are sticky: OR of the previous file and this run (a later `--uninstall` PR reverses exactly what setup did). `presence` is the current state (`on` or `off`), `enrolled` whether the template exists after this run.
- Read: refused (exit 2 before any change) when it is a symlink, not a regular file, larger than `MAX_STATE_BYTES`, group- or world-writable, or when a line does not match the strict grammar (`format` must be `1`, booleans `true|false`, `user` the name regex, `presence` `on|off`). A missing file is a first run.
- Write: `mktemp` in the state directory under `umask 077`, `printf`, `chmod 0600`, `mv -f` over the target (atomic rename); skipped when the content is identical (`cmp -s`). A symlink target is refused.

## 6. Rollback and Abort

- Setup journals only **its own** reversible actions of the current run, in order: `group_added` (reverse: `gpasswd -d <user> soos`, else `deluser <user> soos`), `presence_flag_created` (reverse: `rm -f`), `presence_flag_removed` (reverse: recreate empty `0644`), `soos_conf_dir_created` (reverse: `rmdir`).
- On a runtime failure of access, presence or state (exit 1), and on `INT`/`TERM`, the journal is replayed backwards, then setup prints `Setup FAILED in phase <name>; setup's own changes were rolled back` and the reverse command for `install.sh`'s committed install (`sudo ./scripts/uninstall.sh --keep-data`).
- `install.sh` is the authority for its own files: a failed `install.sh` rolls itself back; a committed install is never removed by setup (same semantics as `install.sh` exit 70). Exit 3 and exit 70 roll nothing back.
- Packages and the Rust toolchain are never removed; they are listed.

## 7. Defaults (OD-3) — what this PR applies

| Feature | Owner default (#362) | This PR |
|---|---|---|
| Face unlock install (daemon, PAM module file, models, unit started) | on | on |
| Presence auto-unlock | on | on (daemon default kept; `--presence off` available; an existing `presence.disable` is reported, never removed without `--presence on`) |
| Enrollment | on (local seat) | on when the session is a local seat with a camera |
| PAM activation | gated, test before activation, rollback | not done; printed pointer (PR 3) |
| Phone companion, Web Push, Funnel, camera view | on, toggleable in the GUI, guided Tailscale steps | not done; manual pointer (PR 4); code defaults of `soos-remote` and `[preview]` unchanged |
| Remote unlock (`allow_unlock`) | not in the owner's list | stays opt-in (recorded) |
| Wi-Fi network lock | not in the owner's list | not installed; an opt-in `--network-lock` hook is recorded for a later PR (ADR 2026-10-13 keeps it off by default) |

## 8. Invariants Touched

- ADR 2026-09-30 "One Distribution PAM Template per Install, Guarded First Start": exit 70 semantics reused, install.sh unchanged.
- ADR 2026-09-30 ONB-16 (verified rustup): only `install_rustup.sh`, no `curl | sh`.
- ADR 2026-10-02 (GitHub #308): never build as root.
- ADR 2026-10-02 "Presence Auto-Unlock Through logind": default on, flag semantics unchanged.
- ADR 2026-10-05 remote companion item (8) "installer never touches Tailscale": kept (setup runs no `tailscale` command in this PR).
- PAM: never modified (`pam-auth-update`, `authselect select`, `soos-admin gdm`, `/etc/pam.d` absent from setup).
- No secret in output or state; no world-writable file; no `eval`; os-release never sourced.

## 9. Test Hooks for Tester

- Fake checkout: copy `scripts/setup.sh` and `scripts/check_build_deps.sh` (real), `packaging/soos-daemon.service`, `models/manifest.toml`; stub `scripts/install.sh`, `install_rustup.sh`, `wait_daemon_ready.sh`.
- Restricted `PATH` (stub dir first, then symlinks to safe coreutils only): no real `sudo`, `doas`, `pacman`, `apt-get`, `dnf`, `systemctl`, `loginctl`, `cargo`, `runuser`, `gpasswd`, `tailscale` reachable.
- Stubs log `uid=<0|1000> <argv>` to `STUB_LOG`; `id`, `sudo`, `runuser`, `getent` emulate root/user through `STUB_AS_ROOT`.
- Fake root: `--prefix`, `--sysconfdir`, `--localstatedir`, `--runstatedir`, `--pam-dir` under the scratch directory; `${SYSCONFDIR}/os-release` drives detection.

Static contracts (`setup_contract.rs`): script present, executable, `set -euo pipefail`, no `eval`, os-release never sourced, no package name literal from `check_build_deps.sh`, calls `--print-packages`, no `tailscale`/`pam-auth-update`/`authselect select`/`gdm enable`/`network-lock install`/`install_remote.sh`/`curl`/`wget`/`chmod 666|777`, `MAX_STATE_BYTES` = 4096, every parsed option documented in `--help`, docs/ADR/walkthrough present.

## 10. Documentation Drift / ADR

- New ADR "[2026-10-13] One-Command Setup `scripts/setup.sh` and Default-On Installer Profile" (OD-1..OD-6, §6 rollback model, §7 defaults table and what later PRs switch on; amends the installer-path wording of the 2026-10-05/06/07 remote ADRs: their code defaults stay `false`, the later setup user phase writes the keys explicitly with guided Tailscale steps).
- Drift found: `README.md` §2 says "install.sh enables the unit but does not start it", still true for `install.sh` without `--start`; kept. #362 acceptance lines for the Docker idempotence/fault-injection matrix, `pam_snapshot.sh verify` after rollback, PAM activation, `--check-config`, TOML editor, Tailscale and `--from-release` belong to later PRs (recorded in the ADR and the walkthrough §9).

## 11. Revision 1 (answers to `AI/plan_evaluator_report.md` round 1) — normative, overrides §2–§9 where they differ

| Finding | Revision |
|---|---|
| F1 enrollment vs daemon camera | **R-1** Enrollment hand-off: `systemctl stop soos-daemon.service` (bounded), then `soos-enroll --biometrics-dir ${LOCALSTATEDIR}/lib/soos/biometrics --key-file ${LOCALSTATEDIR}/lib/soos/master.key --models-dir ${LOCALSTATEDIR}/lib/soos/models enroll --username <user>`, then `systemctl start soos-daemon.service` and `wait_daemon_ready.sh` again. A flag `DAEMON_STOPPED` makes an `EXIT` trap of the root half restart the unit on **every** exit path (success, exit 3, INT/TERM, errors); a failed restart or readiness after enrollment is exit 70. The printed retry command is the same three-step sequence (`sudo systemctl stop soos-daemon && sudo soos-enroll enroll --username <user>; sudo systemctl start soos-daemon`). Test hook: the `soos-enroll` stub fails "busy" while the stub unit is active. The confirmation prompt exists only with a TTY on stdin and stdout; otherwise `--yes` is required (exit 2) and the prompt is never shown. |
| F2 local seat lost across sudo | **R-2** The user half decides the local seat **before** escalating and passes it as the internal argument `--local-seat yes|no` (required with `--system-phase`, refused otherwise). User half rule: an SSH variable (`SSH_CONNECTION`, `SSH_CLIENT`, `SSH_TTY`) → no; else `loginctl show-session <XDG_SESSION_ID or self>`; if that call fails, `loginctl show-user <user> --property=Display` gives the user's graphical session, which is then checked the same way (`Remote=no`, a seat). Setup started as root: SSH variables, then `XDG_SESSION_ID`/`self` only (no `Display` fallback: a root shell over SSH must not borrow the desk session); any failure → not local. |
| F3 sudo stub keeps the environment | **R-3** The test `sudo` stub runs `env -i` with an allowlist (`PATH`, `LC_ALL`, the `STUB_*` variables, `HOME` of root, `SUDO_USER`, `SUDO_UID`), like `env_reset`. Tests assert the artifact directory, the user and the seat verdict reach the root half as arguments (an SSH variable set for the user half must still skip enrollment). |
| F4 build user vs target user | **R-4** Build user = the current user when not root; when root, the invoking user (`SUDO_USER`/`DOAS_USER`/`PKEXEC_UID`), else the checkout owner (`stat -c %U`) when it is not root; a root build user → exit 2 ("never build as root"). Target user = `--user`, else the invoking user, else (non-root) the current user. A read-only target-directory check (one `find -H <target> ! -uid <build uid> -print -quit`) refuses, in the preflight, a target directory holding files not owned by the build user and prints the `chown` fix (same rule as `install.sh check_build_target_dir`). The build runs with an explicit `PATH=<build home>/.cargo/bin:$PATH` and `HOME`, **not** a login shell: `install_rustup.sh` never edits profiles (ADR 2026-10-01), so a login shell would not find a fresh toolchain, and a profile could reorder `PATH`; recorded as a deliberate deviation from `install.sh --build`. `runuser` must exist when root (preflight). |
| F5 exit 2 after changes | **R-5** Setup started as root validates the state file and the root-only symlink checks in the preflight (exit 2, nothing changed). Through the user half, the root half re-validates first; a refusal there is **exit 4** "system phase refused before any system change; packages, toolchain and build of this run are kept". Exit codes 130/143 (INT/TERM) are documented. |
| F6 "already done" misses damage | **R-6** `system_up_to_date` also requires: `${PREFIX}/libexec/soos/provision-master-key` equal to `scripts/provision_master_key.sh`; `${LOCALSTATEDIR}/lib/soos/master.key` present; the `soos` group (`getent group soos`); the PAM template of the family (arch: `${PREFIX}/share/soos/pam/system-auth.snippet`; fedora: every file of `packaging/pam/fedora/soos/` under `${SYSCONFDIR}/authselect/custom/soos/`; debian: `/usr/share/pam-configs/soos` and `soos-notify`, which `install.sh` writes outside the prefix); `download_models.sh --check-only --manifest <installed manifest> --target-dir <models dir>` succeeds (read-only hash check). New flag `--reinstall` forces the system phase; an exit 70 after a skipped system phase prints "re-run with --reinstall". |
| F7 #362 lines | **R-7** Detection also reports the display manager (basename of the `${SYSCONFDIR}/systemd/system/display-manager.service` link target, else `none`) and the Tailscale version (`tailscale version`, bounded, first line, sanitized; `not installed` otherwise) — read-only. The static ban becomes: no `tailscale` subcommand other than `version`. Defaults invariant: SET3 (no PAM, Tailscale write, remote, `[preview]` or network-lock action). The `strace` check is replaced by the write-free whole-tree snapshot (acceptance allows "a write-free stage"); `check_no_key_material.sh` and `secret_scan.sh` run in CI on every push and the state file content is asserted exactly (no secret field). Deferred to later PRs (ADR list): Docker idempotence/fault-injection matrix, `pam_snapshot.sh verify` after rollback, typed confirmations, inconsistent opt-in combinations, user phase, PAM activation, `--check-config`, TOML editor, `--from-release`. |
| m1 | `--artifact-dir`, `--local-seat` and `--system-phase` are documented as internal in `--help` and §2; the re-exec uses the absolute script path. |
| m2 | The resolved family is always passed as `install.sh --distro`, and the PAM directory is resolved with `install.sh`'s rule (first existing candidate directory, else `/usr/lib64/security` or `/lib/security`) and passed as `--pam-dir`. |
| m3 | INT/TERM while armed rolls back the journal (exit 130/143). A state-write failure keeps the run's changes (no rollback), prints the error and exits 1. |
| m4 | The state file must be owned by the effective user (`-O`, root in production); a stored `user` different from the target user is refused (exit 2/4) with "remove it to start over". |
| m5 | Presence state uses `-e || -L`; the plan and summary also report the global kill switch `${SYSCONFDIR}/soos/disabled`. |
| m6 | A `cargo` without `rustup` (distribution package) gets a warning that `rust-toolchain.toml` is not honoured and `scripts/install_rustup.sh` is recommended. |
| m7 | Debian runs `sudo -- env DEBIAN_FRONTEND=noninteractive apt-get install ...`; a failed package install prints the refresh hint (`apt-get update`, `dnf makecache`, `pacman -Syu`) without running it. `read -r -a` performs no pathname expansion. |
| m8 | Dry run needs no TTY; the build and system plan lines say they are based on the current build; the snapshot covers fake root, HOME, checkout and TMPDIR. |
| m9 | `systemctl` queries and `loginctl` are bounded with `timeout`; `$EUID`/`$UID` are banned (only `id -u`). |
| m10 | After `install.sh` exit 70, access, presence, verify and enrollment do not run; the state is written; exit 70. Exit 1 after an `install.sh` failure says "install.sh rolled back its own changes; setup changed nothing else on the system". |
| m11 | The draft contract is revised against this revision (Phase 2); SET2 also bans every package name as a whole shell word. |

Exit codes (final): `0`, `1`, `2`, `3`, `4`, `40`, `70`, `130`, `143`.

## 12. Revision 2 (round 2 refinements m-A..m-I, plan evaluation APPROVED)

- **m-A** Documented residual: an inherited `XDG_SESSION_ID` or the `Display` fallback can classify a remotely attached tmux pane or a mirrored desk session as local; the enrollment line says a face is captured now, at this PC (ADR, Docs §3.0).
- **m-B** The root half traps HUP, INT, QUIT and TERM (exit 129, 130, 131, 143): journal replay, then the EXIT trap restarts a daemon stopped for the enrollment.
- **m-C** soos-enroll gets `--yes` only when stdin is not a terminal (the run was confirmed with `--yes`, and enrollment is only reached when no template exists, so nothing is overwritten); with a terminal the user answers its own `[y/N]`. Tests assert the restart after exit 0 and exit 3; INT and a failed state write are covered by the EXIT trap (review evidence).
- **m-D** Started as root with a non-local verdict, the enrollment skip line adds "re-run ./scripts/setup.sh without sudo from the desktop session".
- **m-E** The target-directory preflight mirrors `install.sh`: owner, then read/write (`-perm -u+rwx` for directories, `-perm -u+rw` for files).
- **m-F** Documented limitation: started as root, cargo is probed in `<build home>/.cargo/bin` and root's `PATH`; a custom `CARGO_HOME` is not seen (run setup as the user instead).
- **m-G** Family PAM templates are compared by content (`cmp -s`).
- **m-H** The ADR carves out the read-only `tailscale version` call; SET3 asserts it is the only Tailscale command, and the behavioral tests assert no other `tailscale` call.
- **m-I** The ADR records that `setup.json` format 1 holds one user; another user is refused until the file is removed, which loses the record of the first run.

## 13. Revision 3 (candid review of 2026-10-13, CHANGES_REQUESTED) — normative

- **CR-1 (MAJOR) Desk-session fallback only for an unknown own session.** `session_is_local` returns 0 (local: `Remote=no` and a seat), 1 (positively not local: `Remote=yes`, or no seat) or 2 (unknown: invalid id, failed query, no `Remote` property). The user half falls back to `loginctl show-user <user> --property=Display` **only on 2**; an answer of 1 is final (for example an operator over SSH who runs `su - alice` or `sudo -u alice -i` while Alice is logged in at the desk). A root start never falls back. Test hook: the `loginctl` stub answers per session id (desk session `STUB_DISPLAY_SESSION`, own session otherwise).
- **CR-2 (MINOR) Stop marker.** `DAEMON_STOPPED` is cleared only after `systemctl start soos-daemon.service` succeeded; a signal between stop and a successful start leaves the restart to the EXIT trap.
- **CR-3 (MINOR) Root build environment.** Started as root, the build and `install_rustup.sh` run as `runuser -u <build user> -- env -i HOME USER LOGNAME PATH=<home>/.cargo/bin:$PATH` plus `LANG`, `LC_ALL`, `TERM` and the proxy variables when set; root's `CARGO_HOME`, `RUSTUP_HOME`, `RUSTFLAGS` and every other variable are dropped.

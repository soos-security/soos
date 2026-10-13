# Network Lock (opt-in polkit Wi-Fi lock)

GitHub #365 (Part A of #359). ADR "[2026-10-13] Opt-In polkit Wi-Fi Lock and `soos-admin network-lock`" in `AI/DECISIONS.md`. Matrix rows NLK1–NLK17 in `AI/VERIFICATION_MATRIX.md` (NLK13–NLK17: hardening of GitHub #370).

## 1. What it does

On a desktop with NetworkManager, any user of the active local session can usually switch to another Wi-Fi network, disconnect it or turn the Wi-Fi radio off without a password. The network lock is an optional polkit rule that makes these actions ask for an **administrator password on every request** (`polkit.Result.AUTH_ADMIN`, never cached, unlike `AUTH_ADMIN_KEEP`).

It is a speed bump against someone at an **unlocked** session. It is not protection against a physical attacker (see §6).

- Off by default. Nothing in `scripts/install.sh` or the distribution packages installs it.
- One file: `/etc/polkit-1/rules.d/49-soos-network-lock.rules` (shipped as `packaging/polkit/49-soos-network-lock.rules` and embedded in `soos-admin`).
- An optional second level (`--rfkill`, also off by default) closes the rfkill bypass (§5).

## 2. Locked actions

The scope is changing and disconnecting Wi-Fi only (owner decision of 2026-10-12). An action is listed only when it changes or disconnects Wi-Fi and its shipped default for the active local session is weaker than an administrator prompt on every request. The rule answers `AUTH_ADMIN` for exactly these NetworkManager polkit actions:

| Action id | Why it is locked |
|---|---|
| `org.freedesktop.NetworkManager.network-control` | activating, deactivating and switching connections (`nmcli connection up/down`, `nmcli device disconnect`, choosing another Wi-Fi network, starting a hotspot) |
| `org.freedesktop.NetworkManager.enable-disable-wifi` | the Wi-Fi radio switch (`nmcli radio wifi off`, the desktop Wi-Fi toggle) |
| `org.freedesktop.NetworkManager.enable-disable-network` | `nmcli networking off`, which disconnects Wi-Fi with every other interface |
| `org.freedesktop.NetworkManager.settings.modify.system` | creating or editing a system connection profile (a new Wi-Fi network, a changed password or autoconnect flag); an autoconnect profile would otherwise let NetworkManager switch networks on its own |
| `org.freedesktop.NetworkManager.settings.modify.own` | the same for a profile stored "only for this user" |

Not locked: hotspot sharing actions (`wifi.share.*`, already covered by `network-control`), mobile broadband (`enable-disable-wwan`), hostname and DNS (`settings.modify.hostname`, `settings.modify.global-dns`, `org.freedesktop.resolve1.*`), statistics, connectivity checks, scans, `sleep-wake` (already refused for every subject by default), `checkpoint-rollback` and `reload` (already `auth_admin_keep` by default), and `org.freedesktop.network1.*` (systemd-networkd does not join Wi-Fi networks and already asks for an administrator).

polkit actions are not per device type: Ethernet and VPN profile changes and connection switches also ask for the password.

The rule returns nothing for every other action, so the next rule or the distribution default decides. It never authorizes an action without authentication and never denies one outright. Root is never affected (NetworkManager and polkitd authorize uid 0 directly), so the lock never blocks a repair from a root shell. For a subject whose distribution default is a plain refusal (for example a remote SSH session on Ubuntu for `enable-disable-wifi`), the result becomes an administrator challenge; answering it needs the same administrator password that already allows `sudo nmcli`, so no new capability is granted.

## 3. Commands

All three commands are subcommands of `soos-admin`. `install` and `uninstall` need root when they act on `/`; `--root DIR` acts below a staging directory instead (tests, image builds and the future one-command installer #362).

```bash
sudo soos-admin network-lock install            # write the polkit rule (root:root 0644)
sudo soos-admin network-lock install --rfkill   # also write the rfkill udev rule (§5)
sudo soos-admin network-lock uninstall          # remove both files
sudo soos-admin network-lock status             # read-only diagnostic
soos-admin --format json network-lock status    # same, as JSON
```

`install` and `uninstall` also refuse when a directory on the path (`/etc`, `/etc/polkit-1`, `/etc/polkit-1/rules.d`, `/etc/udev`, `/etc/udev/rules.d`) is not owned by root or is group- or world-writable. `install` is declarative: the files on disk match the options, so `install` without `--rfkill` removes an rfkill rule written earlier by soos. It refuses (exit 1, nothing written) when `/etc/polkit-1/rules.d` (or, with `--rfkill`, `/etc/udev/rules.d`) does not exist, when any directory on the path is a symbolic link, or when the target name is a symbolic link or not a regular file. The file is written to a temporary name in the same directory (`O_NOFOLLOW`, created exclusively, mode `0600`, then `0644`, `root:root`), flushed, renamed onto the target and the directory flushed, so polkitd never reads a half-written rule. An interrupted install (power loss, `SIGKILL`) can leave the hidden temporary file `.<name>.<pid>.tmp` behind (polkitd and udev never load it); the next `install` or `uninstall` removes such a file when it is a regular file and no process has that pid any more (a file of a running process, for example a concurrent install, is kept; only processes of the caller's PID namespace are seen, so an install running at the same time from a container that shares `/etc` could lose its temporary file and would then fail without changing the rule). `scripts/uninstall.sh` also removes both files and the same stale temporary files; it keeps a file, with a warning, when any directory on its path (`/etc`, `/etc/polkit-1`, `/etc/polkit-1/rules.d`, `/etc/udev`, `/etc/udev/rules.d`) is a symbolic link.

The distribution packages never install the lock and do not remove it either: run `sudo soos-admin network-lock uninstall` before removing the package (a leftover file only keeps the prompts active and can be deleted by hand).

The rule takes effect immediately (polkitd watches its rules directories). Do not edit the installed file: `status` compares it with the copy shipped in `soos-admin` and reports `modified` for any difference.

### 3.1 `soos-admin network-lock status`

Read-only; it never changes anything and never prints an SSID, BSSID, Wi-Fi password, connection name, UUID, address, interface name or process id (it does not open the NetworkManager connection files at all).

| Field | Values |
|---|---|
| `state` | `installed`, `missing`, `modified`, `insecure_permissions`, `unreadable`, `unsupported_backend` (table: `unsupported_backend(iwd)` or `unsupported_backend(networkd)`) |
| `backend` | `network_manager`, `networkd`, `iwd`, `unknown` — from the names of running processes (`/proc/<pid>/comm`), counted only when `/proc/<pid>/status` names the same process and all four of its uids (real, effective, saved, filesystem) are root, or, for `systemd-networkd`, the `systemd-network` account of `/etc/passwd`. A process any user started under one of these names is ignored (GitHub #370) |
| `rules` / `rfkill_rule` | state of each file: `installed`, `missing`, `modified`, `insecure_permissions`, `unreadable` |
| `rfkill` | `uaccess` (the seat user can soft-block radios), `restricted`, `absent` (no rfkill device), `unknown` |
| `warnings` | fixed words, in this order: `rfkill_uaccess` when `rfkill` is `uaccess`; `rfkill_reboot_pending` when the rfkill rule was written after the last boot (an ACL granted before the reboot may still be in place); `iwd_dbus_bypass` when iwd runs next to NetworkManager (iwd's own D-Bus policy lets root and the `netdev` group drive Wi-Fi without polkit) |

- `insecure_permissions`: the file or any directory on its path (`/etc`, `/etc/polkit-1`, `/etc/polkit-1/rules.d`; for the rfkill rule `/etc`, `/etc/udev`, `/etc/udev/rules.d`) is not owned by root, is group- or world-writable, or is a symbolic link — the same check `install` makes before writing.
- `modified`: the file is not byte-identical (SHA-256) to the shipped copy, is empty or larger than 16 KiB.
- `unreadable`: the file could not be read. On Arch `/etc/polkit-1/rules.d` is `0750 root:polkitd`, so run the status with `sudo`.
- `unsupported_backend`: no NetworkManager runs; on an iwd-only or networkd host the rule cannot lock Wi-Fi (§6). The `systemd-network` uid is read from `/etc/passwd` only (not through NSS, which may block on a network directory): when that account exists only in another user database, or `/etc/passwd` is a symbolic link, a running networkd is not recognised and a networkd-only host reports `installed` or `missing` instead (exit code unchanged).
- Limits of the process check: a set-uid root program that switches all four uids to root without authentication, started through a link named like one of the services, would still be counted (no standard program does this).
- Precedence: `insecure_permissions`, `modified`, `unreadable`, `unsupported_backend`, then `installed` or `missing`.
- Exit code `1` when either file is `modified`, `insecure_permissions` or `unreadable`; `0` otherwise (`missing` is a valid opt-in state, and the `rfkill_uaccess` warning is a documented gap).

`soos-gui` also offers the lock as a switch in its feature tour (`Docs/GUI_APPLICATION.md` §1e,
GitHub #379). Off runs `pkexec soos-admin network-lock uninstall`. On runs
`pkexec soos-admin network-lock install` only when the rfkill rule is known to be absent (so an
installed rfkill rule is never dropped silently), and otherwise shows
`sudo soos-admin network-lock install --rfkill`.

## 4. Whose password?

`AUTH_ADMIN` asks for the password of an administrator identity: members of `wheel` on Arch and Fedora, of `sudo` on Debian and Ubuntu (polkit's `50-default.rules` / `49-ubuntu-admin.rules`). On a single-user desktop where you are that administrator, the lock asks for **your own password**, exactly like `sudo`. Anyone who knows your session password can therefore still change the network; the lock stops someone who only finds the session unlocked.

The owner chose not to create a separate admin account. To add one later:

1. Create an administrator account with its own password: `sudo useradd -m -G wheel netadmin && sudo passwd netadmin` (use `-G sudo` on Debian/Ubuntu).
2. Check that `sudo -iu netadmin` works, then remove your daily account from the group: `sudo gpasswd -d <you> wheel` (or `sudo`). From then on, `sudo` and polkit prompts both ask for the `netadmin` password, so keep it somewhere safe; log out and back in for the group change to apply.
3. Run `sudo soos-admin network-lock status` from the `netadmin` account (or with `su - netadmin`).

## 5. rfkill option (`--rfkill`, off by default)

systemd's `70-uaccess.rules` (and GNOME's `61-gnome-settings-daemon-rfkill.rules`) tag `/dev/rfkill` with `uaccess`, so the seat user can soft-block Wi-Fi through `/dev/rfkill` without polkit (desktop airplane mode does this). `install --rfkill` adds `/etc/udev/rules.d/72-soos-network-lock-rfkill.rules` with the single rule `SUBSYSTEM=="misc", KERNEL=="rfkill", TAG-="uaccess"`, ordered after the rules that add the tag and before `73-seat-late.rules`, which turns the tag into an ACL. Consequences:

- the desktop airplane-mode toggle stops working for the user (it keeps working for root);
- it takes effect after a **reboot** (an ACL already granted is not revoked by a udev reload);
- members of the group that owns `/dev/rfkill` keep write access (Arch: `60-rfkill.rules` sets `GROUP="rfkill", MODE="0664"`; do not put the daily user in that group).

`status` reports the gap as the `rfkill_uaccess` warning, from the udev database (`/run/udev/data/c<major>:<minor>`, current `Q:` tags, or `G:` tags on systemd older than 247) and a world-writable `/dev/rfkill`.

## 6. Gaps (what the lock does not cover)

- **rfkill**: without `--rfkill`, the seat user can still turn the radio off through `/dev/rfkill` (status warning `rfkill_uaccess`); with it, members of the `/dev/rfkill` group still can.
- **iwd and wpa_supplicant over D-Bus**: when NetworkManager uses iwd, iwd's D-Bus policy still lets root and members of `netdev` control Wi-Fi directly (`status` warns `iwd_dbus_bypass`); Debian's wpa_supplicant policy admits `netdev` too (Arch's admits root only). Keep the daily user out of `netdev`.
- **iwd**: iwd does not use polkit; its D-Bus policy decides (root and the `netdev` group by default). On an iwd-only host the lock does nothing and `status` reports `unsupported_backend(iwd)`. With NetworkManager on the iwd Wi-Fi backend, requests made through NetworkManager are locked, but iwd's own D-Bus interface stays reachable for root and `netdev` (previous item).
- **systemd-networkd** with wpa_supplicant or iwd: not covered (`unsupported_backend(networkd)`).
- **Root-equivalent groups and cached credentials**: members of `docker`, `lxd` or `libvirt` can become root without a password; a `sudo` timestamp still valid in a terminal, or a retained `auth_admin_keep` authorization (for example `org.freedesktop.systemd1.manage-units`, used by `run0` or `systemctl stop NetworkManager`), lets the session act without a new prompt. The owner's daily account is in `docker`, which is root-equivalent.
- **NetworkManager configuration**: `auth-polkit=false` in `NetworkManager.conf` disables polkit checks inside NetworkManager; only root can set it.
- **Suspend**: suspending the PC through logind takes the network down; it is not a network setting and is not locked.
- **Other polkit rules**: polkit evaluates `/etc/polkit-1/rules.d` and `/usr/share/polkit-1/rules.d` in file-name order and the first rule that returns a result wins. A rule that sorts before `49-soos-network-lock.rules` (for example `10-local.rules`) and answers one of these actions overrides the lock; `status` does not inspect other rules. GNOME's `20-gdm.rules` answers `settings.modify.system` with `YES` for the `gdm` greeter account only (the login screen network menu), not for your session. Rules that sort after it, such as `org.freedesktop.NetworkManager.rules` (Arch: `YES` for local `wheel` members on `settings.modify.system`) and `empower.rules`, are overridden by the lock.
- **Hardware**: a hardware kill switch, unplugging the Ethernet cable or the Wi-Fi adapter, and powering the PC off are physical actions that no software lock can stop.
- **root**: root is never prompted.
- **A locked screen is still required**: anyone at an unlocked session with your password can authenticate.

## 7. Face verification is not used for these prompts

polkit prompts always ask for the password, never for your face. With polkit ≥ 126, `polkit-agent-helper-1` runs as a socket-activated `polkit-agent-helper@.service` instance in `system.slice`, outside any login session and outside your user manager. The soos daemon only lets face answer a PAM request that it can tie to your active local session (ADR 2026-09-30 "Local Session Binding"); for the polkit helper it cannot, so `pam_soos.so` returns `PAM_IGNORE` and the password prompt follows (fail-closed by design, see `Docs/PAM_MODULE.md`). Using face here would need a separate decision on binding the helper to the requesting agent's session.

## 8. Testing

- Static invariants: `tests/invariants/src/network_lock_contract.rs` (exact action list, only `AUTH_ADMIN`, fall-through, rfkill rule, opt-in, uninstall, CI wiring, docs).
- Unit and integration tests: `crates/admin-cli/tests/network_lock_status_tests.rs`, `network_lock_install_tests.rs`, `network_lock_hardening_tests.rs` (GitHub #370: process uids and names, ancestor directories, stale temporary files), and the shell tests `network_lock_install_test.sh` and `network_lock_uninstall_script_test.sh` (run by `network_lock_shell_tests.rs`) against a temporary root, including `scripts/uninstall.sh --destdir`.
- Real polkitd: `tests/docker/network_lock_pkcheck_test.sh` (CI job `network-lock-polkit`) runs `pkcheck` in `ubuntu:24.04` without and with the rule. A container has no seat, so the subject there is not an active local session; the rule never looks at the subject, and the active-session case is the owner hardware check below.
- Owner hardware check (NLK12): on the Arch host with NetworkManager, after `sudo soos-admin network-lock install`, switching or disconnecting Wi-Fi from the desktop and `nmcli radio wifi off` ask for the administrator password every time; `sudo soos-admin network-lock status` reports `installed`; after `sudo soos-admin network-lock uninstall` no password is asked.

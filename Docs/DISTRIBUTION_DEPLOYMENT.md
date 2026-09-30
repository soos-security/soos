# Distribution-Specific Deployment & Validation Guide — soos

## 1. Architectural Scope & Purpose

This document provides complete, operational guidelines for deploying, validating, and rolling back the `soos` local facial biometric authentication subsystem across the Tier-1 Linux distribution families:
- **Debian 12 ("Bookworm") / Ubuntu 24.04 LTS ("Noble Numbat")**
- **Fedora 40 / Red Hat Enterprise Linux 9 (RHEL 9)**
- **Arch Linux**

All deployment workflows strictly adhere to the operational invariants and security specifications established in `AI/ARCHITECTURE.md` §5 (Distribution Adaptation), §9 (Biometric Storage), and §10 (Daemon Hardening).

---

## 2. Universal PAM Stack Ordering

Regardless of distribution, every PAM integration must preserve the universal 3-stage PAM stack ordering:

```pam
# 1. Primary Biometric Check (runs before pam_unix)
auth  [success=done default=ignore]  pam_soos.so

# 2. Standard Password Authentication Fallback
auth  [success=done default=bad]     pam_unix.so try_first_pass

# 3. Password Failure Event Notification (reached ONLY if pam_unix fails)
auth  optional                       pam_soos.so event=password-failed timeout_ms=20
```

### Safety Invariants
1. **Never bypass password authentication**: If `pam_soos.so` fails, times out, or encounters a missing daemon/socket, it **MUST** return `PAM_IGNORE`, yielding execution to `pam_unix`.
2. **Never convert error into `PAM_SUCCESS`**: Any internal panic or unhandled error strictly returns `PAM_IGNORE` via `catch_unwind`.
3. **No stream pollution**: The PAM module must never write to `stdout` or `stderr` (`println!`, `dbg!`), which would corrupt display manager (GDM, LightDM, SDDM) and screen locker (swaylock, hyprlock) communications.

### 2.1 GDM Login Integration (`soos-admin gdm`)

GDM authenticates through its own service file, `/etc/pam.d/gdm-password`, with a longer
2500 ms capture budget. `soos-admin` manages it (ADR 2026-09-30 "GDM PAM Stack Placement",
walkthrough 98):

```bash
sudo soos-admin gdm status                    # installed in PAM? disable flag present?
sudo soos-admin gdm enable                    # insert the managed block (below), remove the flag
sudo soos-admin gdm enable --pam-module-dir /usr/lib64/security   # explicit module directory
sudo soos-admin gdm disable                   # create /etc/soos/gdm.disable (PAM file untouched)
sudo soos-admin gdm restore                   # put back gdm-password.soos-backup, remove it
sudo soos-admin --format json gdm status      # machine-readable status
```

`--pam-file` (default `/etc/pam.d/gdm-password`) and `--disable-file` (default
`/etc/soos/gdm.disable`) select other paths. `gdm disable` is the immediate, lockout-free
switch: `pam_soos.so` reads `PAM_SERVICE` and returns `PAM_IGNORE` for every `gdm*` service
while the flag exists.

What `gdm enable` writes, e.g. on Fedora 40 (`authselect ... with-faillock`):

```pam
auth     [success=done ignore=ignore default=bad] pam_selinux_permit.so
# BEGIN soos-admin gdm enable (managed block, do not edit)
auth  required  pam_faillock.so preauth silent
auth  [success=done default=ignore]  pam_soos.so timeout_ms=2500
# END soos-admin gdm enable
auth        substack      password-auth
...
```

Placement rules:

1. The block goes immediately before the **anchor**: the first `auth` rule that verifies a
   credential or delegates to a shared stack. Credential modules are, exhaustively,
   `pam_unix.so`, `pam_sss.so`, `pam_ldap.so`, `pam_krb5.so`, `pam_winbind.so`,
   `pam_systemd_home.so` and `pam_fprintd.so` (`CREDENTIAL_MODULES` in
   `crates/admin-cli/src/pam_stack.rs`); delegations are `include`, `substack` and
   `@include`. The only rules allowed before the anchor are the pre-credential ones
   (`pam_access`, `pam_env`, `pam_faildelay`, `pam_faillock preauth`, `pam_listfile`,
   `pam_nologin`, `pam_securetty`, `pam_selinux_permit`, `pam_shells`, `pam_succeed_if`);
   the block goes after them. It is never inserted at the top.
2. When that anchor delegates (Ubuntu `@include common-auth`, Fedora `substack
   password-auth`, Arch `include system-local-login`), the gates the delegated stack runs
   before its first credential module (`pam_faillock.so preauth`, `pam_nologin.so`,
   `pam_shells.so`, `pam_succeed_if.so`, `pam_access.so`, `pam_listfile.so`,
   `pam_securetty.so` with a plain `required`/`requisite` control) are copied in front of
   `pam_soos.so`, following includes up to 4 levels. If the delegated stack ends without a
   credential module, the rules after the delegation in `gdm-password` are scanned the
   same way. A locked account therefore fails the auth phase even when the face matches;
   running `preauth` twice is harmless (it only reads the tally). A gate already present
   earlier in `gdm-password` is not copied again only when that earlier rule has a plain
   `required`/`requisite` control; an `optional` copy of the same module does not enforce
   anything, so the delegated gate is still copied. When any `[...=N]` jump precedes the
   anchor (e.g. `[success=1 default=ignore] pam_succeed_if.so user ingroup vip` jumping
   over `requisite pam_nologin.so`), the earlier copy may be skipped, so no delegated gate
   is de-duplicated at all (GitHub #278). As in libpam (`pam.conf(5)`), the type
   and control keywords (`auth`, `include`, `substack`, `required`, `requisite`,
   `optional`, `sufficient` and the bracketed `key=value` controls) are read
   case-insensitively; module paths and arguments are case-sensitive.
3. `enable` refuses, without changing anything (no backup, the `gdm.disable` flag stays),
   when `pam_soos.so` is not installed, the file has no anchor, an **unclassified** auth
   rule (any module not listed above, e.g. `pam_tally2.so`, `pam_group.so`, a vendor
   module, or a conditional/`sufficient` gate) runs before the credential module either in
   `gdm-password` or in a delegated stack, an include target cannot be resolved inside the
   PAM directory (absolute path, file missing from the directory, unreadable, or not a
   regular file: a FIFO, device or socket is refused at once, never waited on), the stack reaches no credential module, a `[...=N]` jump
   would change target, the file is not UTF-8, uses line continuations, exceeds 64 KiB or
   is a symlink. soos cannot tell whether an unknown module is a lockout or login gate, so
   it never guesses.

   **Resolving a refusal.** The error names the rule, e.g. `the PAM file runs the
   unclassified auth rule 'pam_tally2.so' (control 'required') before its credential
   module`. Either:
   - remove or move the rule if it is obsolete (for example migrate `pam_tally2` to
     `pam_faillock`, which soos recognizes), then run `gdm enable` again; or
   - write the rule yourself, after every gate the face must not bypass and immediately
     before the credential module or the `@include`/`substack` line, copying in front of
     it any `required`/`requisite` gate the shared stack runs before its credential module:

     ```pam
     auth    required    pam_tally2.so deny=5 onerr=fail
     auth    required    pam_faillock.so preauth          # copied from common-auth, if present
     auth    [success=done default=ignore]    pam_soos.so timeout_ms=2500
     @include common-auth
     ```

     `gdm enable` never touches a `pam_soos.so` rule you wrote, and `gdm status` reports
     it as installed. Keep a copy of the original file before editing it by hand
     (`gdm restore` only knows the `.soos-backup` written by `enable`).
4. The account phase is never edited: GDM calls `pam_acct_mgmt` after a successful
   `pam_authenticate`, so `pam_nologin`/`pam_faillock`/`pam_unix` account checks always run.
5. The first `enable` keeps the pristine file as `gdm-password.soos-backup` (restored by
   `gdm restore` and by `scripts/uninstall.sh`); the rewrite is atomic. A misplaced line
   written by older releases (`auth  sufficient  pam_soos.so timeout_ms=2500`) is moved to
   the safe position; a `pam_soos.so` rule you wrote yourself is left untouched.
6. **Known refusals.** These default stacks make `enable` refuse. The refusal is fail-closed
   and expected, not a bug; use the manual rule from item 3 instead:
   - **Fedora / RHEL / AlmaLinux / Rocky, authselect `sssd` profile**: `password-auth` runs
     `auth [default=1 ignore=ignore success=ok] pam_usertype.so isregular` and
     `auth [default=1 ignore=ignore success=ok] pam_localuser.so` before `pam_unix.so`.
     Neither module is classified, so the error names `pam_usertype.so`. Only the authselect `local` profile (optionally
     `with-faillock`) is placed automatically.
   - **openSUSE `common-auth`**: `auth optional pam_gnome_keyring.so` precedes
     `pam_unix.so`, so the error names `pam_gnome_keyring.so`.
   - **Vendor PAM directories** (Linux-PAM built with `--enable-vendordir`, e.g. openSUSE
     `/usr/lib/pam.d`): `soos-admin` only searches the directory of the edited file
     (`/etc/pam.d`) and never a vendor directory. An include whose target exists only in
     the vendor directory is refused (the error names the target and says vendor
     directories are not searched). Write the rule by hand after checking the vendor
     stack's gates.

   Manual recipe for these stacks: put the `pam_soos.so` rule in `gdm-password`
   immediately before the `substack`/`include`/`@include` line, and copy in front of it
   every `required`/`requisite` gate that the shared stack runs before its first
   credential module (for example `pam_faillock.so preauth`). A face match then still
   passes those gates, and the unclassified modules run only when the face check falls
   through to the password path:

   ```pam
   auth        required      pam_faillock.so preauth silent   # only if password-auth has it
   auth        [success=done default=ignore]  pam_soos.so timeout_ms=2500
   auth        substack      password-auth
   ```

   A face success then skips `pam_usertype`/`pam_localuser` (in the stock profile they only
   choose between `pam_unix` and `pam_sss`) or `pam_gnome_keyring`'s auth step (the keyring
   is not unlocked by a face login). Accept this only if those modules are not login gates
   on your system.

---

## 3. Debian 12 & Ubuntu 24.04 Deployment

### 3.1 Installation Methods

#### Option A: Native Debian Package (`.deb`)
```bash
# Build native package
./scripts/build_deb.sh

# Install package
sudo dpkg -i target/packages/soos_*.deb

# Verify installation invariants
sudo soos-admin status
```

The package contains no key material: `postinst` generates `/var/lib/soos/master.key` (mode `0600 root:root`) on this host at first install through `/usr/libexec/soos/provision-master-key`, and package upgrades or removal never touch it (see `Docs/PACKAGING_AND_PROVISIONING.md` §7.4).

#### Option B: Universal Installer
```bash
# Build dependencies (see Docs/PACKAGING_AND_PROVISIONING.md §3.1)
./scripts/check_build_deps.sh --print-packages build
# Build release artifacts, then install binaries, unit files, models and invariant directories
sudo ./scripts/install.sh --build
```

The installer fails closed (non-zero exit, nothing modified) when an artifact is missing or comes from a
debug build, and rolls back every change if a later step such as model verification fails.

### 3.2 PAM Stack Integration via `pam-auth-update`
Debian and Ubuntu dynamically manage `/etc/pam.d/common-auth` using `pam-auth-update`. `soos` provides two profiles in `/usr/share/pam-configs/`:
1. `/usr/share/pam-configs/soos` (`Primary`, Priority `260`, placed before `unix` at `256`)
2. `/usr/share/pam-configs/soos-notify` (`Primary`, Priority `12`, control `[default=ignore]`):
   placed after every standard primary method (`unix` 256, `sss` 128, ...) and **before**
   `pam_deny`. `pam-auth-update` rewrites the `success=N` jump of `pam_unix` so that a correct
   password skips the hook; a wrong one falls through to it, then to `pam_deny`. The hook is
   ignored whatever it returns. (An `Additional` profile would be emitted after
   `auth requisite pam_deny.so`, which ends the stack on a wrong password: the event would never fire.)

Enable the profiles non-interactively:
```bash
sudo pam-auth-update --package --enable soos soos-notify
```

Resulting `/etc/pam.d/common-auth` (Ubuntu 24.04, verified by `tests/docker/pam_rollback_test.sh`
D3 and the Docker matrix case T11):
```pam
auth  [success=done default=ignore]  pam_soos.so
auth  [success=2 default=ignore]     pam_unix.so nullok try_first_pass
auth  [default=ignore]               pam_soos.so event=password-failed timeout_ms=20
auth  requisite                      pam_deny.so
auth  required                       pam_permit.so
```

### 3.3 User Enrollment & Verification
```bash
# 1. Add user to soos system group
sudo soos-admin add-user alice

# 2. Enroll facial biometric vector
sudo soos-enroll enroll --username alice

# 3. Verify encrypted biometric template permissions (the store names the
#    template after the numeric UID: <uid>.cbor.enc)
sudo stat -c "%a %U:%G" "/var/lib/soos/biometrics/$(id -u alice).cbor.enc"
# Expected: 600 root:root
```

### 3.4 Operational Testing & Password Fallback
```bash
# Test nominal facial authentication (daemon active)
sudo pamtester common-auth alice authenticate

# Test password fallback (daemon stopped or face occluded)
sudo systemctl stop soos-daemon
sudo pamtester common-auth alice authenticate
# System prompts for password and succeeds with valid credentials
```

### 3.5 Rollback & Uninstallation
```bash
# Safe rollback preserving biometric templates
sudo ./scripts/uninstall.sh --keep-data

# Or remove package via dpkg (prerm runs pam-auth-update --package --remove soos soos-notify)
sudo dpkg -r soos
```

`scripts/uninstall.sh` deregisters both profiles, restores every `*.soos-backup` (for example the
`gdm-password` copy made by `soos-admin gdm enable`) and verifies the result against the
pre-install snapshot `/var/lib/soos/state/pam-backup` (see §7).

---

## 4. Fedora 40 & RHEL 9 Deployment with `authselect`

### 4.1 Custom `authselect` Profile
Fedora and RHEL mandate the use of `authselect` to manage `/etc/pam.d/system-auth` and `/etc/pam.d/password-auth`. Direct manual editing of PAM configuration files is prohibited.

`soos` deploys a **complete** custom `authselect` profile (`packaging/pam/fedora/soos/`, derived
from the Fedora 40 `local` profile) to `/etc/authselect/custom/soos/`. `authselect select`
regenerates *every* managed file from the profile directory, so a profile shipping only the PAM
stacks would empty `/etc/nsswitch.conf` on activation (review finding ONB-02, GitHub #145). The
profile therefore ships the full layout:
- `README`: Profile description and the **declared optional features** (`with-faillock`,
  `with-mkhomedir`, `with-fingerprint`, `with-silent-lastlog`, `with-pam-u2f`, ... — the same set as
  the `local` profile; a feature that is not declared here is rejected by `authselect select`)
- `system-auth`: Local and console services (e.g. `sudo`, `login`)
- `password-auth`: Display managers and graphical sessions (e.g. `gdm`)
- `nsswitch.conf`: Name service switch template (`passwd`, `group`, `shadow`, `hosts`, ...)
- `fingerprint-auth`, `smartcard-auth`, `postlogin`, `dconf-db`, `dconf-locks`: Inherited from `local`
- `REQUIREMENTS`: Operator notes printed on activation (`pam_soos.so` + `soos-daemon.service`)

The only lines that differ from the `local` profile are the two `pam_soos.so` lines in the `auth`
sections of `system-auth` and `password-auth`.

### 4.2 Preservation of `pam_faillock` Lockout Policy
To prevent brute-force attacks against user passwords, Fedora employs `pam_faillock`. The `soos`
custom profile preserves `pam_faillock` with strict hook placement, using the `authselect`
conditional syntax (`{include if "feature"}`, see `authselect-profiles(5)`); the lines are emitted
only when the profile is selected with `with-faillock`:

```pam
# Pre-authentication lockout check (denies locked accounts immediately)
auth        required                      pam_faillock.so preauth silent    {include if "with-faillock"}

# Primary facial biometric authentication
auth        [success=done default=ignore] pam_soos.so

# Standard password fallback
auth        sufficient                    pam_unix.so nullok

# Failure accounting hook (increments failure counter on wrong password)
auth        required                      pam_faillock.so authfail          {include if "with-faillock"}

# Intrusion detection notification
auth        optional                      pam_soos.so event=password-failed timeout_ms=20

auth        required                      pam_deny.so

account     required                      pam_faillock.so                   {include if "with-faillock"}
```

When biometric verification succeeds (`pam_soos.so` returns `[success=done]`), `pam_unix` and
`pam_faillock authfail` are bypassed cleanly. When biometric verification falls back (`PAM_IGNORE`),
`pam_faillock` continues counting consecutive password authentication failures, and the
`event=password-failed` notification is still reached because `authfail` is `required` (not `die`).

### 4.3 Activation and Service Verification
`scripts/install.sh` and the RPM `%post` scriptlet install the profile and record the currently
selected profile (`authselect current --raw`, e.g. `local with-silent-lastlog`) in
`/etc/soos/authselect.previous`; they never activate the profile themselves.

```bash
# Inspect the features currently enabled and carry them over
authselect current --raw

# Activate the custom profile with faillock enabled (add the other features from above)
sudo authselect select custom/soos with-faillock --force

# Verify authselect profile consistency and the generated stacks
sudo authselect check
grep -n 'pam_faillock\|pam_soos\|pam_unix' /etc/pam.d/system-auth

# Test sudo service integration
sudo pamtester sudo alice authenticate

# Test GDM display manager integration
sudo pamtester gdm-password alice authenticate
```

A dockerized validation of the whole sequence (activation, `authselect check`, generated
`system-auth`/`password-auth` ordering, `/etc/nsswitch.conf` preserved, password fallback,
rollback) runs with `./run_tests.sh authselect` (see section 6).

### 4.4 Rollback & Uninstallation
`scripts/uninstall.sh` and the RPM `%preun` scriptlet restore the profile recorded in
`/etc/soos/authselect.previous` (falling back to `local`, then `minimal`, then `sssd`) **before**
removing `/etc/authselect/custom/soos`; if no profile can be restored, the custom profile is kept
so that `authselect` never points at a deleted profile.

```bash
# Automatic rollback (restores the recorded profile, then removes the custom profile)
sudo ./scripts/uninstall.sh --keep-data      # or: sudo rpm -e soos

# Manual rollback: restore the recorded profile, or the stock local profile
sudo authselect select $(cat /etc/soos/authselect.previous) --force
sudo authselect select local --force
sudo authselect check
```

---

## 5. Arch Linux Deployment & Screen Lockers

### 5.1 Installation via PKGBUILD
```bash
# Build Arch package
./scripts/build_arch.sh

# Install package
sudo pacman -U target/packages/soos-*.pkg.tar.zst
```

### 5.2 `/etc/pam.d/system-auth` Integration
Arch Linux utilizes a modular `/etc/pam.d/system-auth` stack. The integration is a manual edit:
**first keep a backup** that `scripts/uninstall.sh` restores automatically:

```bash
sudo cp /etc/pam.d/system-auth /etc/pam.d/system-auth.soos-backup
```

The `soos` snippet (`packaging/pam/arch/system-auth.snippet`) is placed immediately prior to `pam_unix.so`
(adjust any `success=N` jump that crosses the inserted lines, e.g. the one of `pam_systemd_home.so`):

```pam
# Inserted into /etc/pam.d/system-auth:
auth  [success=done default=ignore]  pam_soos.so
auth  required                       pam_unix.so try_first_pass nullok
auth  optional                       pam_soos.so event=password-failed timeout_ms=20
```

### 5.3 Wayland Screen Locker Integration (`swaylock` & `hyprlock`)
Wayland compositors (Hyprland, Sway) rely on dedicated PAM service files located in `/etc/pam.d/`:
- `/etc/pam.d/swaylock`
- `/etc/pam.d/hyprlock`

Both configurations standardly include `system-auth`:
```pam
#%PAM-1.0
auth include system-auth
account include system-auth
```

#### Operational Guarantees
- **Instant Unlock**: Upon user face recognition, `pam_soos.so` returns `PAM_SUCCESS` within `< 150ms`, unlocking the screen locker without requiring Enter or keyboard input.
- **Graceful Fallback**: If the camera is occluded or the user is absent, the screen locker remains locked and immediately accepts the user's password.
- **Zero Lockup**: Because `pam_soos.so` forbids stdout/stderr writes, no escape sequences or debug messages corrupt Wayland client/compositor sockets.

### 5.4 Rollback & Uninstallation
```bash
# Remove pacman package
sudo pacman -R soos

# Restore system-auth from the backup made in §5.2 (or run scripts/uninstall.sh,
# which restores it; without a backup it falls back to the pre-install snapshot)
sudo cp /etc/pam.d/system-auth.soos-backup /etc/pam.d/system-auth
```

---

## 6. Automated Validation Test Harness

To execute the automated distribution validation suite:

```bash
# Live validation of one distribution (or all) in a disposable Docker container
bash tests/distro/run_distro_validation.sh ubuntu      # fedora | arch | all

# Print every distribution's plan; executes nothing privileged (no Docker needed)
bash tests/distro/run_distro_validation.sh --dry-run all

# Run Debian 12 / Ubuntu 24.04 test harness
bash tests/distro/debian_ubuntu_test.sh --dry-run

# Run Fedora 40 / RHEL 9 test harness
bash tests/distro/fedora_rhel_test.sh --dry-run

# Run Arch Linux test harness
bash tests/distro/arch_linux_test.sh --dry-run

# Fedora authselect profile: activation, generated PAM/NSS files, password
# fallback and rollback in a stock fedora:40 container (also a CI job)
./run_tests.sh authselect        # = tests/docker/authselect_profile_test.sh

# Debian stack order (password-failed hook before pam_deny) and byte-for-byte
# PAM rollback on ubuntu:24.04 and fedora:40 (also a CI job)
./run_tests.sh rollback          # = tests/docker/pam_rollback_test.sh
```

Safety rules of the harness (GitHub #163, #168):

- **Docker is the only live path.** `run_distro_validation.sh` maps each distribution to
  its script explicitly (`debian_ubuntu_test.sh`, `fedora_rhel_test.sh`,
  `arch_linux_test.sh`) and runs it in a disposable container built from
  `tests/docker/Dockerfile.<distro>`. Without Docker it fails and executes nothing; it
  never falls back to a live run on the host.
- **Explicit consent for host changes.** A live run of a distro script installs packages,
  rewrites PAM files and creates users, so each script refuses (exit code 2) unless given
  `--allow-host-changes`. The runner passes that flag only inside the container;
  `--skip-docker` requires the operator to pass `--allow-host-changes` to the runner.
- **Per-distribution artifacts.** Each container mounts a Docker volume
  (`soos-distro-target-<distro>`) over `/workspace/target`, so binaries built on one
  distribution are never installed on another, and no root-owned build output is written
  to the host's `target/` directory. Remove the volumes with
  `docker volume rm soos-distro-target-ubuntu soos-distro-target-fedora soos-distro-target-arch`.
- **Production socket modes.** The scripts create `/run/soos` with
  `install -d -m 0750 -o root -g soos` and assert `750 root:soos` for the directory and
  `660 root:soos` for the mock daemon socket (`tests/docker/mock_daemon.py` refuses any
  mode that grants a permission to "other").
- **Real enrollment.** Templates are created by `soos-enroll --mock enroll --username
  testuser --yes` (mock camera, synthetic inference, real encrypted store); the scripts
  assert `/var/lib/soos/biometrics/<uid>.cbor.enc` is `600 root:root` and that
  `soos-enroll --mock verify` accepts it.
- **Rollback matches the install.** After a native package install the scripts record the
  install mode (`deb`, `rpm`, `pkgbuild`), so rollback removes the package through the
  package manager instead of deleting its files behind its back.

---

## 7. Emergency Rescue Shell & Disaster Recovery

If a misconfiguration occurs during manual PAM adjustments:
1. **Always maintain an open root shell** (`sudo -s`) in a separate terminal before modifying `/etc/pam.d/`.
2. **Boot with systemd emergency target**: Append `systemd.unit=emergency.target` to the GRUB kernel command line.
3. **Restore PAM backup**:
   - Debian: `pam-auth-update --package --remove soos soos-notify` (or `pam-auth-update --force`)
   - Fedora: `authselect select $(cat /etc/soos/authselect.previous) --force` (or `authselect select local --force`)
   - Arch: `cp /etc/pam.d/system-auth.soos-backup /etc/pam.d/system-auth` (backup made in §5.2)
   - GDM: `soos-admin gdm restore`, or `cp /etc/pam.d/gdm-password.soos-backup /etc/pam.d/gdm-password` (made by `soos-admin gdm enable`; §2.1)
   - Any file: the pre-install copies recorded by `scripts/install.sh` live in
     `/var/lib/soos/state/pam-backup/pam.d/` with a `SHA256SUMS` manifest
     (`sudo ./scripts/pam_snapshot.sh verify` lists what differs).

### 7.1 Rollback Guarantees (`scripts/uninstall.sh`)
- `scripts/install.sh` records the pre-install PAM state (`/etc/pam.d/*`, `/etc/nsswitch.conf`,
  `authselect current --raw`) in `/var/lib/soos/state/pam-backup` (mode `0700`) through
  `scripts/pam_snapshot.sh`; an existing snapshot is never overwritten.
- Uninstall order: `pam-auth-update --package --remove`, `authselect select <recorded>`,
  restore of every `*.soos-backup`, then removal of residual `pam_soos.so` lines **only when safe**
  (the stripped file equals its snapshot copy, or it has no `success=N` jump). Every write is a
  temporary file + rename.
- The result is verified against `SHA256SUMS`; the snapshot is discarded only when identical.
- If a residual line cannot be removed safely, the file is left untouched, `pam_soos.so` is **kept**
  (it degrades to `PAM_IGNORE`, so password login keeps working) and the script exits `1`.
- Validation: `./run_tests.sh rollback` (`tests/docker/pam_rollback_test.sh`, ubuntu:24.04 and
  fedora:40): `sha256` of every `/etc/pam.d` entry and `authselect current` identical before
  installation and after uninstallation.

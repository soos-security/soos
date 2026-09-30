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
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250

# 2. Standard Password Authentication Fallback
auth  [success=done default=bad]     pam_unix.so try_first_pass

# 3. Password Failure Event Notification (reached ONLY if pam_unix fails)
auth  optional                       pam_soos.so event=password-failed timeout_ms=20
```

### Safety Invariants
1. **Never bypass password authentication**: If `pam_soos.so` fails, times out, or encounters a missing daemon/socket, it **MUST** return `PAM_IGNORE`, yielding execution to `pam_unix`.
2. **Never convert error into `PAM_SUCCESS`**: Any internal panic or unhandled error strictly returns `PAM_IGNORE` via `catch_unwind`.
3. **No stream pollution**: The PAM module must never write to `stdout` or `stderr` (`println!`, `dbg!`), which would corrupt display manager (GDM, LightDM, SDDM) and screen locker (swaylock, hyprlock) communications.

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
# Install binaries, unit files, and provision invariant directories
sudo ./scripts/install.sh
```

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
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250
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
sudo soos-enroll alice

# 3. Verify encrypted biometric template permissions
sudo stat -c "%a %U:%G" /var/lib/soos/biometrics/alice.bin
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
auth        [success=done default=ignore] pam_soos.so timeout_ms=250

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
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250
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
# Run multi-distribution test runner (auto-detects environment or uses Docker)
bash tests/distro/run_distro_validation.sh --help

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

---

## 7. Emergency Rescue Shell & Disaster Recovery

If a misconfiguration occurs during manual PAM adjustments:
1. **Always maintain an open root shell** (`sudo -s`) in a separate terminal before modifying `/etc/pam.d/`.
2. **Boot with systemd emergency target**: Append `systemd.unit=emergency.target` to the GRUB kernel command line.
3. **Restore PAM backup**:
   - Debian: `pam-auth-update --package --remove soos soos-notify` (or `pam-auth-update --force`)
   - Fedora: `authselect select $(cat /etc/soos/authselect.previous) --force` (or `authselect select local --force`)
   - Arch: `cp /etc/pam.d/system-auth.soos-backup /etc/pam.d/system-auth` (backup made in §5.2)
   - GDM: `cp /etc/pam.d/gdm-password.soos-backup /etc/pam.d/gdm-password` (made by `soos-admin gdm enable`)
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

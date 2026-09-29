# System Provisioning, Packaging & Distribution Guidelines — soos

## 1. Architectural Scope & Purpose

This document details the system provisioning, installation, uninstallation, distribution-specific PAM integrations, and user group management for the `soos` local facial biometric authentication subsystem.

All installation and provisioning workflows adhere strictly to the threat model and operational invariants defined in `AI/ARCHITECTURE.md` §5 (Distribution Adaptation) and §10 (Daemon Hardening).

---

## 2. Directory Hierarchy and Security Invariants

The `soos` installation script (`scripts/install.sh`) guarantees the following filesystem hierarchy and permissions:

| Path | Mode | Owner:Group | Purpose | Invariant Reference |
|---|---|---|---|---|
| `/usr/libexec/soos/soos-daemon` | `0755` | `root:root` | Privileged background daemon binary | §10 Daemon Hardening |
| `/usr/bin/soos-enroll` | `0755` | `root:root` | Biometric enrollment CLI | §8 Monorepo Structure |
| `/usr/bin/soos-admin` | `0755` | `root:root` | Non-biometric diagnostic & admin CLI | §8 Monorepo Structure |
| `/lib/security/pam_soos.so` (or arch/distro equiv) | `0644` | `root:root` | PAM shared object module | §5 PAM Crate & ABI |
| `/etc/systemd/system/soos-daemon.service` | `0644` | `root:root` | Hardened systemd service unit (`Group=soos`, `StateDirectory=soos`) | §10 Systemd Sandboxing |
| `/var/lib/soos/` | `0755` | `root:root` | Persistent base state directory | §9 Privacy & Persistence |
| `/var/lib/soos/biometrics/` | `0700` | `root:root` | Encrypted biometric vector templates | §9 Biometric Templates |
| `/var/lib/soos/evidence/` | `0700` | `root:root` | Opt-in encrypted intrusion snapshots | §9 Evidence Snapshots |
| `/var/lib/soos/models/` | `0755` | `root:root` | Attested ONNX machine learning models | §7 Models & Pipeline |
| `/usr/libexec/soos/provision-master-key` | `0755` | `root:root` | First-install master key provisioning helper (shared by `install.sh` and all package scriptlets) | §9 Biometric Templates |
| `/var/lib/soos/master.key` | `0600` | `root:root` | 32-byte cryptographic AES master key, generated **on the target host only** (never inside a package or staging tree) | §9 Biometric Templates |
| `/run/soos/` | `0750` | `root:soos` | Runtime Unix Domain Socket directory | §4 IPC Protocol & Sockets |

---

## 3. Installation Script (`scripts/install.sh`)

### Features & Capabilities
- **System Group Provisioning**: Creates the `soos` system group if absent (`groupadd -r soos` or `addgroup --system soos`).
- **Cryptographic Master Key Generation (live install only)**: On a live install (no `--destdir`), delegates to `scripts/provision_master_key.sh`, which generates a 32-byte key from `openssl rand 32` or `/dev/urandom` with mode `0600` from inception if `/var/lib/soos/master.key` is absent. In staging mode (`--destdir`) **no key material is ever generated**: the tree is package content, and the key is created on the target host at first install by the shipped helper (see §7.4).
- **Binary & PAM Shared Library Deployment**: Locates built release artifacts and deploys them to target locations. Auto-detects target architecture and PAM module paths (`/lib/x86_64-linux-gnu/security`, `/usr/lib64/security`, etc.).
- **Systemd Integration**: Deploys `packaging/soos-daemon.service`, invokes `systemctl daemon-reload`, and enables the service unit.
- **Model Download & Attestation**: Invokes `scripts/download_models.sh` to download and cryptographically verify all ONNX models cataloged in `models/manifest.toml`.

### Supported CLI Flags
```bash
./scripts/install.sh [OPTIONS]

Options:
  -d, --destdir <DIR>      Staging destination directory (default: /)
  --prefix <DIR>           Installation prefix (default: /usr)
  --sysconfdir <DIR>       Configuration directory (default: /etc)
  --localstatedir <DIR>    State directory (default: /var)
  --runstatedir <DIR>      Runtime directory (default: /run)
  --pam-dir <DIR>          Explicit PAM module directory (auto-detected if omitted)
  --skip-models            Skip model download and verification
  --skip-systemd           Skip systemctl reload and enable invocations
  --dry-run                Print plan without modifying filesystem
  -h, --help               Display help message and exit
```

---

## 4. Universal PAM Stack Ordering

As mandated by `AI/ARCHITECTURE.md` §5, all distribution configurations enforce the following universal PAM stack ordering:

```pam
# 1. Primary biometric check: called before pam_unix
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250

# 2. Standard password verification fallback
auth  [success=done default=bad]     pam_unix.so try_first_pass

# 3. Reached strictly if pam_unix fails: intrusion detection event notification
auth  optional                       pam_soos.so event=password-failed timeout_ms=20
```

### Distribution Adaptation Matrix

#### Debian / Ubuntu (`pam-auth-update`)
- **Primary Profile**: `packaging/pam/debian/soos` installed to `/usr/share/pam-configs/soos` (Priority `260`, placed before `unix` Priority `256`).
- **Notification Profile**: `packaging/pam/debian/soos-notify` installed to `/usr/share/pam-configs/soos-notify` (Priority `128`, placed after `unix`).
- Enable command: `pam-auth-update --enable soos soos-notify`

#### Fedora / RHEL (`authselect`)
- Custom `authselect` profile template located in `packaging/pam/fedora/soos/`.
- Deployed to `/etc/authselect/custom/soos/`.
- Enable command: `authselect select custom/soos`

#### Arch Linux
- Universal snippet in `packaging/pam/arch/system-auth.snippet`.
- Full `/etc/pam.d/system-auth` configuration in `packaging/pam/arch/system-auth`.

---

## 5. Safe Uninstallation & Rollback (`scripts/uninstall.sh`)

The uninstallation script guarantees that removing `soos` will **never lock an administrator out of their system**.

### Capabilities
- **Systemd Teardown**: Stops and disables `soos-daemon.service`, removes the unit file, and issues `daemon-reload`.
- **PAM Configuration Rollback**: Restores original PAM configurations from backup (`*.soos-backup`), deregisters profiles from `pam-auth-update`, or removes custom `authselect` profiles.
- **Binary Cleanup**: Removes `soos-daemon`, `soos-enroll`, `soos-admin`, and `pam_soos.so`.
- **Data Protection**:
  - By default (or with `--keep-data`): strictly retains `/var/lib/soos/biometrics`, `/var/lib/soos/evidence`, and `/var/lib/soos/master.key`.
  - With `--purge-data`: permanently wipes all biometric data and keys.

```bash
./scripts/uninstall.sh [OPTIONS]

Options:
  --keep-data              Preserve biometric templates, evidence, and master key (DEFAULT)
  --purge-data             Permanently remove /var/lib/soos and cryptographic keys
  --destdir <DIR>          Staging destination directory (default: /)
  --skip-systemd           Skip systemctl operations
  --dry-run                Display plan without disk changes
```

---

## 6. User Group Provisioning (`soos-admin add-user`)

To grant a local user facial authentication access, the user must belong to the `soos` system group:

```bash
sudo soos-admin add-user <username>
```

- Validates username against strict POSIX syntax (`^[a-z_][a-z0-9_-]*\$?$`, maximum 32 characters).
- Verifies user existence via `nix::unistd::User::from_name`.
- Dispatches `usermod -aG soos <username>`.
- Fails closed on invalid usernames, unknown users, or permission failures.

---

## 7. Native Distribution Packages (deb, rpm, PKGBUILD)

`soos` provides native distribution packaging specifications and automated builder scripts supporting the three primary Linux distribution ecosystems.

### 7.1 Package Architectures & Specifications

| Ecosystem | Specification Files | Target Package Format | Installation Tool | PAM Integration Method |
|---|---|---|---|---|
| **Debian / Ubuntu** | `packaging/debian/control`<br>`packaging/debian/rules`<br>`packaging/debian/postinst`<br>`packaging/debian/prerm`<br>`packaging/debian/postrm` | `soos_<version>_<arch>.deb` | `dpkg -i` / `apt` | `pam-auth-update` profiles in `/usr/share/pam-configs/` |
| **Fedora / RHEL** | `packaging/rpm/soos.spec` | `soos-<version>-<release>.<arch>.rpm` | `rpm -i` / `dnf` | `authselect` custom profile in `/etc/authselect/custom/soos/` |
| **Arch Linux** | `packaging/arch/PKGBUILD`<br>`packaging/arch/soos.install` | `soos-<version>-<release>-<arch>.pkg.tar.zst` | `pacman -U` / `makepkg -si` | Snippet in `/etc/pam.d/soos.snippet` |

### 7.2 Building Distribution Packages

Distribution packages can be built individually or collectively via the master unified package builder:

```bash
# Build all supported distribution packages
./scripts/build_packages.sh all

# Build specific package format
./scripts/build_packages.sh deb
./scripts/build_packages.sh rpm
./scripts/build_packages.sh arch

# Fast packaging using pre-compiled release artifacts
./scripts/build_packages.sh deb --skip-build
```

Generated packages are placed in `target/packages/`.

### 7.3 Security and Filesystem Invariants Enforced by Packages

Every distribution package enforces the following invariant properties during post-installation:
1. **Dedicated System Group**: Creates `soos` system group if absent (`groupadd -r soos` / `addgroup --system soos`).
2. **Persistence Hardening**:
   - `/var/lib/soos/` mode `0755` (`root:root`)
   - `/var/lib/soos/biometrics/` mode `0700` (`root:root`)
   - `/var/lib/soos/evidence/` mode `0700` (`root:root`)
   - `/var/lib/soos/models/` mode `0755` (`root:root`)
   - `/run/soos/` mode `0750` (`root:soos`)
3. **Master Key Generation on the Target Host**: The post-install scriptlet (`postinst configure`, `%post`, `post_install`) runs `/usr/libexec/soos/provision-master-key`, which generates a 32-byte AES key at `/var/lib/soos/master.key` (mode `0600 root:root`) if absent and never overwrites an existing key, so upgrades keep enrolled templates decryptable. The key is host state, not package content: it is never listed in the archive (`%ghost` in RPM) and survives package removal.
4. **Service Management**: Installs `/usr/lib/systemd/system/soos-daemon.service` (or `/etc/systemd/system/`), triggers `systemctl daemon-reload`, and enables the service unit.
5. **Fail-Closed Teardown**: Pre-removal scriptlets (`prerm`, `%preun`, `pre_remove`) stop and disable `soos-daemon.service` before removing binaries, and remove runtime sockets while preserving biometric data at rest.

### 7.4 Key Material Is Never Packaged (GitHub #144)

A package is a distributable artifact: any key inside it would be shared by every machine installed from it and would be overwritten on upgrade and deleted on removal by the package manager. The following layers enforce that `/var/lib/soos/master.key` (or any `*.key` file) never enters a package:

| Layer | Location | Behavior |
|---|---|---|
| Installer | `scripts/install.sh` | Skips key generation whenever `--destdir` is set; installs `scripts/provision_master_key.sh` as `/usr/libexec/soos/provision-master-key` (mode `0755`). |
| Build guard | `scripts/check_no_key_material.sh` | Run by `scripts/build_deb.sh`, `scripts/build_arch.sh` and `packaging/debian/rules` on the staged tree; aborts the build if any `*.key` file exists at any depth (the `find` result is captured in a variable, never piped to `grep -q`, to avoid a `pipefail` false negative). |
| Target-host generation | `/usr/libexec/soos/provision-master-key [--state-dir <DIR>]` | Called by `packaging/debian/postinst` (`configure`), `packaging/arch/soos.install` (`post_install`), `packaging/rpm/soos.spec` (`%post`) and by `install.sh` on a live install. Refuses symlinks and non-regular files, creates the key in a private temporary file under `umask 077` (mode `0600` from inception), verifies it is exactly 32 bytes, publishes it with an atomic hard link (a concurrently created key is never clobbered), tightens an existing key to `0600 root:root`, and prints paths only, never key bytes. |
| Recipes | `packaging/arch/PKGBUILD`, `packaging/rpm/soos.spec` | Install the helper; the RPM keeps `master.key` as `%ghost %attr(0600, root, root)`. `scripts/uninstall.sh` removes the helper and keeps the key unless `--purge-data` is given. |
| Tests | `tests/invariants/src/lib.rs`, `tests/docker/test_packages.sh` | Invariants PMK1–PMK5 (`AI/VERIFICATION_MATRIX.md`); the Docker package test asserts the archive listing has no `.key` entry, the installed key is `0600` and 32 bytes, it survives removal, and two fresh installs produce distinct keys. |

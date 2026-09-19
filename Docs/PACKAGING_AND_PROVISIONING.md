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
| `/etc/systemd/system/soos-daemon.service` | `0644` | `root:root` | Hardened systemd service unit | §10 Systemd Sandboxing |
| `/var/lib/soos/` | `0755` | `root:root` | Persistent base state directory | §9 Privacy & Persistence |
| `/var/lib/soos/biometrics/` | `0700` | `root:root` | Encrypted biometric vector templates | §9 Biometric Templates |
| `/var/lib/soos/evidence/` | `0700` | `root:root` | Opt-in encrypted intrusion snapshots | §9 Evidence Snapshots |
| `/var/lib/soos/models/` | `0755` | `root:root` | Attested ONNX machine learning models | §7 Models & Pipeline |
| `/var/lib/soos/master.key` | `0600` | `root:root` | 32-byte cryptographic AES master key | §9 Biometric Templates |
| `/run/soos/` | `0750` | `root:soos` | Runtime Unix Domain Socket directory | §4 IPC Protocol & Sockets |

---

## 3. Installation Script (`scripts/install.sh`)

### Features & Capabilities
- **System Group Provisioning**: Creates the `soos` system group if absent (`groupadd -r soos` or `addgroup --system soos`).
- **Cryptographic Master Key Generation**: If `/var/lib/soos/master.key` is absent, automatically generates a 32-byte cryptographically secure key from `/dev/urandom` or `openssl rand 32` with mode `0600`.
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

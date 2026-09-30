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

### 3.1 Build & Runtime Dependencies (GitHub #165)

`scripts/check_build_deps.sh` is the single source of truth for these lists (`--print-packages <build|gui|models> [--distro <id>]`) and, without arguments, runs a read-only build host preflight (`cargo`, `rustc`, `cc`, `pkg-config`, OpenSSL headers via `pkg-config openssl`, `security/pam_appl.h`, `libclang`). The lists were verified by `cargo build --release --locked --workspace` in bare `ubuntu:24.04`, `fedora:40` and `archlinux` containers with only these packages plus rustup.

| Group | Debian 12 / Ubuntu 24.04 | Fedora 40 / RHEL 9 | Arch Linux |
|---|---|---|---|
| **build** | `build-essential pkg-config libpam0g-dev libclang-dev clang libssl-dev` | `gcc gcc-c++ make pkgconf-pkg-config pam-devel clang-devel openssl-devel` | `base-devel clang openssl pkgconf pam` |
| **gui** (runtime, `soos-gui` only) | `libxkbcommon0 libwayland-client0 libwayland-egl1 libegl1 libgl1 libx11-6 libxcursor1 libxi6 libxrandr2` | `libxkbcommon libwayland-client libwayland-egl mesa-libEGL mesa-libGL libX11 libXcursor libXi libXrandr` | `libxkbcommon wayland libglvnd libx11 libxcursor libxi libxrandr` |
| **models** (`scripts/download_models.sh`) | `curl ca-certificates coreutils` | `curl ca-certificates coreutils` | `curl ca-certificates coreutils` |

Why each build package is needed (from the locked dependency graph):

- **C/C++ toolchain**: linking; the ONNX Runtime static library pulled by `ort-sys` needs `libstdc++`.
- **PAM headers**: `pam-bindings` (`crates/pam`).
- **libclang + clang**: `bindgen`, a build-dependency of `v4l2-sys-mit` (`crates/camera-v4l`).
- **OpenSSL headers + pkg-config**: `openssl-sys` ← `native-tls` ← `ureq`, used by the `ort-sys` build script. `openssl-src` is not vendored, so the system headers are required at build time only; no shipped binary links OpenSSL.
- **Rust toolchain**: install rustup from <https://rustup.rs>; `rust-toolchain.toml` selects the channel.

**ONNX Runtime download**: the `ort-sys` build script downloads prebuilt ONNX Runtime binaries during `cargo build` (network required). For offline or air-gapped builds, point `ORT_LIB_LOCATION` at a local ONNX Runtime build before running cargo.

**GUI runtime libraries** are loaded with `dlopen()` by `winit`/`glutin`, so `dpkg-shlibdeps` and friends cannot detect them. The `.deb` (which ships `soos-gui`) declares them as `Recommends:`; `scripts/build_deb.sh` declares the linked runtime libraries (`libc6`, `libgcc-s1`, `libstdc++6`, `libpam0g`) explicitly because it does not run `dpkg-shlibdeps`.

**Model tools**: `scripts/download_models.sh` needs only `bash` (>= 4), `sha256sum` (coreutils) and, for `https://` sources, `curl` with `ca-certificates`. **Python is not required**: the manifest is parsed in bash against its fixed schema (GitHub #167). Packages do not download models; fetch them from a source checkout with `sudo ./scripts/download_models.sh`.

**Download hardening (GitHub #208)**: `models/manifest.toml` is the only source of download URLs (the former per-model fallback table is gone; the dry run prints exactly the manifest `source_url`). `curl` runs with `--proto '=https' --proto-redir '=https' --tlsv1.2 --connect-timeout 15 --max-time 900 --max-filesize`. Each model is staged in an unpredictable `mktemp` file created under `umask 077` in the target directory, its size is checked against `MAX_MODEL_BYTES` (256 MiB; the largest attested model is 136,619,444 bytes) **before** it is hashed, and a failed or interrupted run removes it. `SOOS_MODEL_MAX_BYTES` may only tighten the cap (1..`MAX_MODEL_BYTES`); any other value fails before a write.

### 3.2 Features & Capabilities
- **Fail-closed preflight (GitHub #164)**: before any change, `install.sh` checks that a live install (no `--destdir`) runs as root, that every artifact (`soos-daemon`, `soos-admin`, `soos-enroll`, `soos-gui`, `libpam_soos.so`) exists in the artifact directory, that the directory is not a cargo `debug` profile directory, and that the model manifest and tools pass `download_models.sh --preflight`. Any failure exits non-zero (exit `2`) with nothing modified. `target/debug` is never searched implicitly.
- **Explicit release build**: `install.sh --build` runs `scripts/check_build_deps.sh` and then `cargo build --release --locked --workspace` (as `$SUDO_USER` when run through sudo, never as root in a user's checkout); a failed build exits `40`. Without `--build`, artifacts are taken from `${CARGO_TARGET_DIR:-target}/release` or `--artifact-dir`.
- **Transactional install with rollback**: every created file and directory and every overwritten file (saved to a private `mktemp -d` backup directory) is journaled. Files are written atomically (temporary file + rename in the target directory). If any later step fails, or the script is interrupted, the journal is replayed backwards: overwritten files are restored, created files and directories are removed, a group created by the run is deleted, the unit is disabled if the run enabled it, and the script reports `Installation FAILED ... all changes were rolled back` with a non-zero exit.
- **System Group Provisioning**: Creates the `soos` system group if absent (`groupadd -r soos` or `addgroup --system soos`). The unit runs with `Group=soos`, so a live install without the group and without either tool fails the preflight (GitHub #211) instead of warning.
- **Cryptographic Master Key Generation (live install only)**: On a live install (no `--destdir`), delegates to `scripts/provision_master_key.sh`, which generates a 32-byte key from `openssl rand 32` or `/dev/urandom` with mode `0600` from inception if `/var/lib/soos/master.key` is absent. In staging mode (`--destdir`) **no key material is ever generated**: the tree is package content, and the key is created on the target host at first install by the shipped helper (see §7.4). A key created by a run that later fails is removed by the rollback (nothing was encrypted with it yet); an existing key is never touched.
- **Binary & PAM Shared Library Deployment**: Deploys the release artifacts to their target locations. Auto-detects target architecture and PAM module paths (`/lib/x86_64-linux-gnu/security`, `/usr/lib64/security`, etc.). Pre-existing system directories (`/usr/bin`, the PAM module directory, `/etc/systemd/system`, ...) keep their mode; only directories created by the run, and soos-owned directories (§2), get their mode set.
- **Model Download & Attestation before enabling the unit**: Invokes `scripts/download_models.sh` to download and verify (SHA-256) all ONNX models cataloged in `models/manifest.toml` (or `--manifest`). A verification failure exits `60` and rolls back; the unit is never enabled without verified models.
- **Systemd Integration**: Deploys `packaging/soos-daemon.service`, and as the last step invokes `systemctl daemon-reload` and enables the unit. The unit bounds restarts (`StartLimitIntervalSec=60`, `StartLimitBurst=5`) so a daemon that cannot start does not crash-loop forever, and carries `ConditionPathExists=/var/lib/soos/models/manifest.toml` (GitHub #211): until `download_models.sh` has deployed the manifest, a start is skipped (`condition failed`) instead of failing and restarting.
- **Distribution PAM template (GitHub #209)**: exactly one template family is installed, chosen by `--distro` (`auto` by default, `debian`, `fedora`, `arch`, `none`). `auto` parses (never sources) `ID` then `ID_LIKE` from the target `os-release` (`debian`/`ubuntu` → Debian `pam-auth-update` profiles, `fedora`/`rhel`/`centos` → authselect profile, `arch` → Arch snippet); under `--destdir` only the stage's `etc/os-release` / `usr/lib/os-release` is read, and with none (or an unsupported distribution) no template is installed and a warning asks for `--distro`. An unknown `--distro` value fails the preflight. `build_deb.sh` / `debian/rules` pass `--distro debian`, `build_arch.sh` passes `--distro arch`; the RPM spec installs only the authselect profile. No file is ever installed in `/etc/pam.d` (every file there is a PAM service): the Arch snippet lives in `/usr/share/soos/pam/system-auth.snippet`.
- **Start and readiness (GitHub #211)**: `--start` (live install, systemd running) starts the unit after enabling it and runs `scripts/wait_daemon_ready.sh`, which fails immediately when the models manifest is missing, waits at most `--timeout` seconds (default 30, 1..300) for the daemon socket, then prints `soos-admin --format json --socket-path <sock> status`. A daemon that does not become ready exits `70` without rolling back the (committed) install. Operators can run the helper on its own after `systemctl start soos-daemon`.
- `--allow-missing` (developer use only) installs a partial artifact set and ends with an `INCOMPLETE` banner instead of the success message.

### 3.3 Supported CLI Flags
```bash
./scripts/install.sh [OPTIONS]

Options:
  -d, --destdir <DIR>      Staging destination directory (default: /)
  --prefix <DIR>           Installation prefix (default: /usr)
  --sysconfdir <DIR>       Configuration directory (default: /etc)
  --localstatedir <DIR>    State directory (default: /var)
  --runstatedir <DIR>      Runtime directory (default: /run)
  --pam-dir <DIR>          Explicit PAM module directory (auto-detected if omitted;
                           with --destdir only the stage is probed, then
                           /usr/lib/security is used with a warning)
  --artifact-dir <DIR>     Built artifacts directory (default: target/release)
  --build                  Check build dependencies, then run
                           'cargo build --release --locked --workspace'
  --allow-missing          Install a partial artifact set (developer use only)
  --allow-debug-artifacts  Accept artifacts from a cargo 'debug' profile directory
  --manifest <PATH>        Model manifest (default: models/manifest.toml)
  --skip-models            Skip model download and verification
  --skip-systemd           Skip systemctl reload and enable invocations
  --distro <FAMILY>        PAM template to install: auto (default), debian,
                           fedora, arch or none (auto reads the target os-release)
  --start                  Start the unit after enabling it and wait until the
                           daemon answers (live install only)
  --dry-run                Run the read-only preflight and print the plan
  -h, --help               Display help message and exit
```

PAM module directory under `--destdir`: only directories that already exist inside the stage are probed; the build host is never consulted (its `/usr/lib64` says nothing about the target). With no match, `/usr/lib/security` is used and a warning asks for `--pam-dir`. Packaging always passes it explicitly: `/usr/lib/<DEB_HOST_MULTIARCH>/security` (Debian/Ubuntu), `/usr/lib/security` (Arch), `%{_libdir}/security` (RPM).

Exit codes: `0` success, `1` usage error, `2` preflight failure (nothing modified), `40` release build failed (nothing installed), `60` model deployment or verification failed (rolled back), `70` installed but `--start` did not reach readiness (not rolled back); any other non-zero code is the failing step's own status (rolled back).

---

## 4. Universal PAM Stack Ordering

As mandated by `AI/ARCHITECTURE.md` §5, all distribution configurations enforce the following universal PAM stack ordering:

```pam
# 1. Primary biometric check: called before pam_unix
auth  [success=done default=ignore]  pam_soos.so

# 2. Standard password verification fallback
auth  [success=done default=bad]     pam_unix.so try_first_pass

# 3. Reached strictly if pam_unix fails: intrusion detection event notification
auth  optional                       pam_soos.so event=password-failed timeout_ms=20
```

### Distribution Adaptation Matrix

#### Debian / Ubuntu (`pam-auth-update`)
- **Primary Profile**: `packaging/pam/debian/soos` installed to `/usr/share/pam-configs/soos` (Priority `260`, placed before `unix` Priority `256`).
- **Notification Profile**: `packaging/pam/debian/soos-notify` installed to `/usr/share/pam-configs/soos-notify` (`Auth-Type: Primary`, Priority `12`, control `[default=ignore]`): emitted after `pam_unix` and every other standard primary method but **before** `auth requisite pam_deny.so`, so it is reached on a wrong password only (an `Additional` profile would sit after `pam_deny` and never fire). The line is ignored whatever it returns.
- Enable command: `pam-auth-update --package --enable soos soos-notify`
- The `postinst` never passes `--force`: a locally modified stack is left untouched, and the skip (or a `pam-auth-update` failure) is reported with a `soos: WARNING:` on stderr instead of being silenced (GitHub #281, `Docs/DISTRIBUTION_DEPLOYMENT.md` §3.2).

#### Fedora / RHEL (`authselect`)
- Complete custom `authselect` profile in `packaging/pam/fedora/soos/` (derived from the Fedora 40
  `local` profile: `README` with the declared features, `REQUIREMENTS`, `system-auth`,
  `password-auth`, `nsswitch.conf`, `fingerprint-auth`, `smartcard-auth`, `postlogin`, `dconf-db`,
  `dconf-locks`). Conditionals use the `{include if "with-faillock"}` syntax of `authselect-profiles(5)`.
- Deployed to `/etc/authselect/custom/soos/`; the previously selected profile is recorded in
  `/etc/soos/authselect.previous` for rollback. The profile is never activated automatically.
- Enable command: `authselect select custom/soos with-faillock --force` then `authselect check`
  (append the other features listed by `authselect current --raw`).
- Validation: `./run_tests.sh authselect` (fedora:40 container, see `Docs/DISTRIBUTION_DEPLOYMENT.md` §4).

#### Arch Linux
- Universal snippet in `packaging/pam/arch/system-auth.snippet`, installed as reference material to `/usr/share/soos/pam/system-auth.snippet` (never `/etc/pam.d/soos.snippet`, which Linux-PAM would treat as a service named `soos.snippet`; `uninstall.sh` still removes that legacy file).
- Full `/etc/pam.d/system-auth` configuration in `packaging/pam/arch/system-auth`.

---

## 5. Safe Uninstallation & Rollback (`scripts/uninstall.sh`)

The uninstallation script guarantees that removing `soos` will **never lock an administrator out of their system**.

### Capabilities
- **Systemd Teardown**: Stops and disables `soos-daemon.service`, removes the unit file, and issues `daemon-reload`.
- **Pre-install Snapshot**: `scripts/install.sh` records `/etc/pam.d/*`, `/etc/nsswitch.conf` and `authselect current --raw` in `/var/lib/soos/state/pam-backup` (mode `0700`, `SHA256SUMS` manifest) through `scripts/pam_snapshot.sh` before any PAM template is installed; an existing snapshot is never overwritten. A snapshot that fails part-way leaves no `state/.pam-backup.*` temporary directory, and a failed install discards a snapshot it created (it is journaled before the helper runs), so the rollback can remove `/var/lib/soos`.
- **PAM Configuration Rollback**: Restores original PAM configurations from backup (`*.soos-backup`, e.g. the `gdm-password` copy written by `soos-admin gdm enable`, which edits the file atomically), deregisters profiles from `pam-auth-update`, or — when `custom/soos` is the selected `authselect` profile — re-selects the profile recorded in `/etc/soos/authselect.previous` (fallback `local`, `minimal`, `sssd`) before removing the custom profile; the profile is kept if no restoration succeeds. Residual `pam_soos.so` lines are removed only when provably safe (the stripped file equals its snapshot copy, or it has no `success=N` jump); all writes are temporary file + rename. The final state is verified against the snapshot, which is discarded only when identical. If a residual line cannot be removed safely, the file and `pam_soos.so` are kept (the module degrades to `PAM_IGNORE`, password login keeps working) and the script exits `1`.
- **Binary Cleanup**: Removes `soos-daemon`, `soos-enroll`, `soos-admin`, and `pam_soos.so`.
- **Data Protection**:
  - By default (or with `--keep-data`): strictly retains `/var/lib/soos/biometrics`, `/var/lib/soos/evidence`, and `/var/lib/soos/master.key`.
  - With `--purge-data`: permanently wipes all biometric data and keys. If the PAM rollback is incomplete or unverified, `/var/lib/soos/state/pam-backup` (the only pre-install PAM copy) is kept and the script says so; delete it manually once the PAM stack is restored.

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
| **Arch Linux** | `packaging/arch/PKGBUILD`<br>`packaging/arch/soos.install` | `soos-<version>-<release>-<arch>.pkg.tar.zst` | `pacman -U` / `makepkg -si` | Snippet in `/usr/share/soos/pam/system-auth.snippet` (integrated by hand into `/etc/pam.d/system-auth`) |

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

### 7.2.1 Package Metadata Source of Truth (GitHub #210)

| Field | Source | Consumers |
|---|---|---|
| Version | `version` in `[workspace.package]` of the root `Cargo.toml` | `scripts/build_deb.sh`, `scripts/build_rpm.sh`, `scripts/build_arch.sh` read it through `scripts/lib/pkg_meta.sh` (`soos_pkg_version`); `packaging/rpm/soos.spec` `Version:` and `packaging/arch/PKGBUILD` `pkgver=` are literals that must equal it |
| License | `license` in `[workspace.package]` (SPDX `AGPL-3.0-or-later`) | `.PKGINFO` of `build_arch.sh` (`soos_pkg_license`); `soos.spec` `License:` and PKGBUILD `license=()` must equal it |

- `scripts/lib/pkg_meta.sh` parses the manifest statically (no cargo, no network, no Python) and fails
  closed when the table or the key is missing. It can also be run directly:
  `bash scripts/lib/pkg_meta.sh [--manifest <Cargo.toml>] <version|license>`.
- `scripts/build_rpm.sh` refuses to package (exit `1`) when the spec `Version:` / `License:` differ
  from `Cargo.toml`; `tests/invariants/src/onboarding_packaging_contract.rs` enforces the same equality
  for the spec and the PKGBUILD on every `cargo test`.
- Every packaging cargo invocation (`packaging/debian/rules`, `soos.spec` `%build`, PKGBUILD `build()`,
  `scripts/build_*.sh`) uses `--locked`, so a package is always built from the committed `Cargo.lock`.
- `soos-gui` ships in all three formats (`.deb` through `install.sh`, `%{_bindir}/soos-gui` in the RPM,
  `/usr/bin/soos-gui` in the PKGBUILD). Its `dlopen()`ed runtime libraries are `Recommends:` in the
  `.deb` and the RPM and `optdepends` in the PKGBUILD (lists from `check_build_deps.sh --print-packages gui`).
- The PKGBUILD has no release tarball yet (`source=()`): run `makepkg` from `packaging/arch/` in a
  checkout, or point `SOOS_SRC_DIR` at one; `build()` and `package()` `cd` into that tree.

### 7.3 Security and Filesystem Invariants Enforced by Packages

Every distribution package enforces the following invariant properties during post-installation:
1. **Dedicated System Group**: Creates `soos` system group if absent (`groupadd -r soos` / `addgroup --system soos`).
2. **Persistence Hardening**:
   - `/var/lib/soos/` mode `0755` (`root:root`)
   - `/var/lib/soos/biometrics/` mode `0700` (`root:root`)
   - `/var/lib/soos/evidence/` mode `0700` (`root:root`)
   - `/var/lib/soos/models/` mode `0755` (`root:root`)
   - `/run/soos/` mode `0750` (`root:soos`)
3. **Master Key Generation on the Target Host**: The post-install scriptlet (`postinst configure`, `%post`, `post_install`) runs `/usr/libexec/soos/provision-master-key`, which generates a 32-byte AES key at `/var/lib/soos/master.key` (mode `0600 root:root`) if absent and never overwrites an existing key, so upgrades keep enrolled templates decryptable. The key is host state, not package content: it is never listed in the archive nor owned by any package (not even as `%ghost` in RPM, which RPM would delete on erase; an upgrade from an older `%ghost` build is protected by a `%pre` copy restored in `%posttrans`, proven by the `rpm -U` case of `tests/docker/test_packages.sh`; `%posttrans` compares the copy with `cmp`, hence `Requires(posttrans): diffutils`, and a comparison that cannot run (exit status 2 or 127) keeps both files with a "could not compare" warning instead of claiming the keys differ) and survives package removal.
4. **Service Management**: Installs `/usr/lib/systemd/system/soos-daemon.service` (or `/etc/systemd/system/`), triggers `systemctl daemon-reload`, and enables the service unit.
5. **Fail-Closed Teardown**: Pre-removal scriptlets (`prerm`, `%preun`, `pre_remove`) stop and disable `soos-daemon.service` before removing binaries, and remove runtime sockets while preserving biometric data at rest.

### 7.4 Key Material Is Never Packaged (GitHub #144)

A package is a distributable artifact: any key inside it would be shared by every machine installed from it and would be overwritten on upgrade and deleted on removal by the package manager. The following layers enforce that `/var/lib/soos/master.key` (or any `*.key` file) never enters a package:

| Layer | Location | Behavior |
|---|---|---|
| Installer | `scripts/install.sh` | Skips key generation whenever `--destdir` is set; installs `scripts/provision_master_key.sh` as `/usr/libexec/soos/provision-master-key` (mode `0755`). |
| Build guard | `scripts/check_no_key_material.sh` | Run by `scripts/build_deb.sh`, `scripts/build_arch.sh` and `packaging/debian/rules` on the staged tree; aborts the build if any `*.key` file exists at any depth (the `find` result is captured in a variable, never piped to `grep -q`, to avoid a `pipefail` false negative). |
| Target-host generation | `/usr/libexec/soos/provision-master-key [--state-dir <DIR>]` | Called by `packaging/debian/postinst` (`configure`), `packaging/arch/soos.install` (`post_install`), `packaging/rpm/soos.spec` (`%post`) and by `install.sh` on a live install. Refuses symlinks and non-regular files, creates the key in a private temporary file under `umask 077` (mode `0600` from inception), verifies it is exactly 32 bytes, publishes it with an atomic hard link (a concurrently created key is never clobbered), tightens an existing key to `0600 root:root`, and prints paths only, never key bytes. |
| Recipes | `packaging/arch/PKGBUILD`, `packaging/rpm/soos.spec` | Install the helper; the RPM never lists `master.key` in `%files` (the helper enforces mode 0600). `scripts/uninstall.sh` removes the helper and keeps the key unless `--purge-data` is given. |
| Tests | `tests/invariants/src/lib.rs`, `tests/docker/test_packages.sh` | Invariants PMK1–PMK5 (`AI/VERIFICATION_MATRIX.md`); the Docker package test asserts the archive listing has no `.key` entry, the installed key is `0600` and 32 bytes, it survives removal, and two fresh installs produce distinct keys. |

---

## 8. Daemon Service Sandbox and Configuration Validation (GitHub #199, #202, #203)

### 8.1 Least-privilege systemd unit (`packaging/soos-daemon.service`)

The daemon runs as `root:soos` but inside a bounded sandbox:

| Directive | Why |
|---|---|
| `CapabilityBoundingSet=CAP_IPC_LOCK CAP_CHOWN CAP_FOWNER CAP_DAC_OVERRIDE`, `AmbientCapabilities=` | `mlockall` (`CAP_IPC_LOCK`), socket `fchownat` / `fchmodat` (`CAP_CHOWN`, `CAP_FOWNER`); `CAP_DAC_OVERRIDE` is kept until dropping it is validated on a real host |
| `SystemCallFilter=@system-service`, `SystemCallErrorNumber=EPERM` | `@system-service` includes `@memlock`, `@chown`, `ioctl` (V4L2) and `sched_setaffinity` (ONNX Runtime threads) |
| `ProtectKernelTunables/Modules/Logs=yes`, `ProtectControlGroups=yes`, `ProtectClock=yes`, `ProtectHostname=yes`, `RestrictNamespaces=yes`, `RestrictRealtime=yes` | No kernel, cgroup, clock, namespace or realtime access is needed |
| `PrivateNetwork=yes`, `IPAddressDeny=any`, `RestrictAddressFamilies=AF_UNIX` | No network at all; the filesystem socket `/run/soos/daemon.sock` works inside a private network namespace |
| `DevicePolicy=closed`, `DeviceAllow=char-video4linux rw` | systemd does not expand globs in `DeviceAllow=` node paths, so `/dev/video*` alone matches nothing; the device group allows every V4L2 node |
| `Before=display-manager.service`, `Type=notify`, `NotifyAccess=main`, `TimeoutStartSec=60` | The daemon sends `READY=1` (`soos_daemon::sd_notify`) only after `/run/soos/daemon.sock` is bound, so the greeter waits until PAM requests can be served; a start that never reports readiness fails after 60 s (GitHub #203) |

Deliberately **not** set: `ProtectProc=invisible` / `ProcSubset=pid` (they would hide
`/proc/<pid>/cgroup` of other users' peers, which the session policy reads), `PrivateDevices=yes`
(removes `/dev/video*`). The contract is `crates/daemon/tests/systemd_hardening_tests.rs`.
`systemd-analyze security --offline=yes` reports an exposure of 1.7 (7.5 before). Check the
installed unit with `systemd-analyze security soos-daemon` on the target host.

### 8.2 `daemon.toml` is validated fail-closed

`DaemonConfig::validate()` runs whenever `/etc/soos/daemon.toml` (or `--config`) is loaded; any
invalid value stops the daemon with an error naming the setting:

| Setting | Accepted values |
|---|---|
| `[socket] socket_mode` | `432` (0o660) or `384` (0o600) only |
| `[dispatcher] connection_timeout_ms` | 100 to 10000 |
| `[dispatcher] max_concurrent_connections` | at least 1 |
| `[pipeline.thresholds]` | finite, within (0, 1], at or above the policy security floor |
| `[pipeline.rate_limit] max_attempts`, `max_tracked_uids`, `window_duration_secs` | at least 1 |
| `[pipeline.evidence] retention_days` | at least 1 |
| `log_level` | non-blank `tracing` filter directive of at most 256 bytes |

### 8.3 The daemon never creates the `soos` group

The group is provisioned by the packages and `scripts/install.sh` only. If it is missing, the
daemon fails to bind with `Group 'soos' not found` instead of running `groupadd`. On C libraries
without `fchmodat(AT_SYMLINK_NOFOLLOW)` support, the socket mode is set through an
`O_PATH | O_NOFOLLOW` descriptor, never through a path that could follow a symlink.

# Walkthrough 44 — Native Distribution Packages (deb, rpm, PKGBUILD)

## Context & Objectives

- **Issue**: Issue #27 (`feat(packaging): Distribution packages (deb, rpm, PKGBUILD)`) / GitHub #66
- **Branch**: `feat/distro-packages`
- **Mission**:
  1. Author Debian `.deb` package specification (`packaging/debian/control`, `packaging/debian/rules`, `packaging/debian/postinst`, `packaging/debian/prerm`, `packaging/debian/postrm`) and automated builder script `scripts/build_deb.sh`.
  2. Author RPM `.spec` file (`packaging/rpm/soos.spec`) and automated builder script `scripts/build_rpm.sh` for Fedora and RHEL.
  3. Author Arch Linux package recipe (`packaging/arch/PKGBUILD`), install scriptlet (`packaging/arch/soos.install`), and builder script `scripts/build_arch.sh`.
  4. Provide a master unified builder (`scripts/build_packages.sh`) and container-based package test harness (`tests/docker/test_packages.sh`).
  5. Author contractual invariant unit tests in `tests/invariants/src/lib.rs` enforcing packaging specifications and permissions.

---

## 1. Architectural Design & Compliance (Phase 1 & Phase 1.5)

- **Plan Evaluator Sub-Agent**:
  Evaluated the packaging implementation across the 6 core pillars in `AI/plan_evaluator_report.md` and issued `VALIDATION_VERDICT: APPROVED`.
- **System Provisioning Invariants Enforced across All Package Formats**:
  - `soos` dedicated system group created idempotently if missing.
  - Sensitive directories created with mode `0700` (`/var/lib/soos/biometrics`, `/var/lib/soos/evidence`).
  - Persistent base state directory `/var/lib/soos` mode `0755` and model directory `/var/lib/soos/models` mode `0755`.
  - Master key `/var/lib/soos/master.key` generated with 32 bytes mode `0600` (`root:root`).
  - Runtime socket directory `/run/soos` mode `0750` (`root:soos`).
  - Executable binaries deployed to `/usr/libexec/soos/soos-daemon`, `/usr/bin/soos-admin`, `/usr/bin/soos-enroll`.
  - PAM shared library `pam_soos.so` installed with mode `0644` in system security modules directory.
  - Systemd service unit `soos-daemon.service` installed with reload and enable hooks.

---

## 2. Test Contracts (Phase 2 - TDD Red Phase)

Contractual tests were authored in `tests/invariants/src/lib.rs` before creating package files and scripts:
1. `test_debian_packaging_specification`: Asserts existence, executable bits, control dependencies (`pam`, `systemd`, `adduser`), and postinst security permissions (`0700`, `0750`, `0600`).
2. `test_rpm_packaging_specification`: Asserts existence, package name, `%pre` group creation, `%post` master key generation, and `%files` mode attributes (`0700` biometrics/evidence, `0600` master key, `0750` runtime dir).
3. `test_arch_packaging_specification`: Asserts existence, package dependencies (`pam`, `systemd`), build and package functions, and `soos.install` scriptlet hooks.
4. `test_package_build_scripts_executable_and_help`: Asserts all build scripts (`build_deb.sh`, `build_rpm.sh`, `build_arch.sh`, `build_packages.sh`) exist, are executable (`0755`), and support `--help` without error.

All 4 invariant tests failed as expected during Phase 2.

---

## 3. Implementation (Phase 3 & Phase 4)

- **Debian / Ubuntu Packaging**:
  - `packaging/debian/control`: Package metadata, binary architecture, dependencies on PAM, systemd, and adduser.
  - `packaging/debian/rules`: Executable build recipe using cargo release build and `scripts/install.sh`.
  - `packaging/debian/postinst`: Post-install scriptlet provisioning `soos` group, directory hierarchy, master key, and PAM registration.
  - `packaging/debian/prerm`: Pre-removal scriptlet stopping and disabling `soos-daemon.service` and deregistering PAM profile.
  - `packaging/debian/postrm`: Post-removal scriptlet cleaning up `/run/soos` and invoking systemd daemon-reload.
  - `scripts/build_deb.sh`: Script generating binary `DEBIAN/control` and invoking `dpkg-deb --build --root-owner-group`.
- **Fedora / RHEL Packaging**:
  - `packaging/rpm/soos.spec`: RPM spec with `%prep`, `%build`, `%install`, `%pre` (groupadd `soos`), `%post` (key & dir provisioning), `%preun` and `%postun` (`%systemd` scriptlets), and `%files` with explicit `%attr` modes.
  - `scripts/build_rpm.sh`: Creates RPM build directory tree, prepares source tarball, and runs `rpmbuild -bb`.
- **Arch Linux Packaging**:
  - `packaging/arch/PKGBUILD`: Arch build recipe compiling release binaries and packaging to `$pkgdir`.
  - `packaging/arch/soos.install`: Scriptlet configuring `soos` group, permissions, master key, and systemd reload.
  - `scripts/build_arch.sh`: Creates Arch package archive with `.PKGINFO` and `.INSTALL`.
- **Unified Tooling & Docker Harness**:
  - `scripts/build_packages.sh`: Master packaging script supporting `deb`, `rpm`, `arch`, and `all` targets.
  - `tests/docker/test_packages.sh`: Distribution-aware container test harness validating installation and removal via `dpkg -i`, `rpm -i`, and `pacman -U`.

---

## 4. Candid Review & Quality Gates (Phase 5)

- Ran `./scripts/candid_review.sh` to ensure strict conformance with zero-trust invariants, panic safety, and English language policy.
- Authored independent audit report in `AI/candid_review_report.md` with `VERDICT: APPROVED`.
- Formatted all code (`cargo fmt --all -- --check`) and verified zero warnings (`cargo clippy --all-targets --all-features -- -D warnings`).
- Validated supply chain licenses and advisories (`cargo deny check`).

---

## 5. Traceability Synchronization (Phase 6)

- Synchronized `AI/BACKLOG.md` sub-issues:
  - `- [x] **#27.1**` (Debian spec & builder)
  - `- [x] **#27.2**` (RPM spec & builder)
  - `- [x] **#27.3**` (Arch PKGBUILD & builder)
- Automated dual synchronization executed via `python3 scripts/sync_issue.py --auto`:
  - Updated GitHub Issue #66 with completed sub-issues and progress comment.
- Updated `AI/VERIFICATION_MATRIX.md` with formal criteria PK5, PK6, and PK7.
- Updated `Docs/PACKAGING_AND_PROVISIONING.md` with distribution packaging architecture and CLI commands.

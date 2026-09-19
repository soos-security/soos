# Plan Evaluation Report — Issue #27: Distribution Packages (deb, rpm, PKGBUILD)

- **Date**: 2026-09-19
- **Evaluator**: Independent Plan Evaluator Sub-Agent (`plan-evaluator`)
- **Target**: Issue #27 (`feat(packaging): Distribution packages (deb, rpm, PKGBUILD)`) / GitHub #66
- **Branch**: `feat/distro-packages`

---

## Executive Summary

The proposed implementation plan addresses all three sub-issues of Issue #27:
1. `#27.1`: Debian `.deb` package specification (`packaging/debian/control`, `packaging/debian/rules`, `packaging/debian/postinst`, `packaging/debian/prerm`, `packaging/debian/postrm`, and `scripts/build_deb.sh`) with post-install group creation, directory hierarchy provisioning, master key generation, and PAM configuration.
2. `#27.2`: RPM `.spec` file (`packaging/rpm/soos.spec` and `scripts/build_rpm.sh`) with `%pre`, `%post`, `%preun`, `%postun`, `%files` with strict file permissions, and Fedora `authselect` custom profile integration.
3. `#27.3`: Arch Linux `PKGBUILD` and `soos.install` (`packaging/arch/PKGBUILD`, `packaging/arch/soos.install`, and `scripts/build_arch.sh`) with `build()`, `package()`, and post-install system group and permission setup.

The plan also integrates invariant tests in `tests/invariants/src/lib.rs` and Dockerized package verification in `tests/docker/test_packages.sh`.

---

## 6-Pillar Compliance Assessment

### Pillar 1: Architectural Alignment & Threat Model
- **Filesystem Hierarchy**: The daemon binary is installed to `/usr/libexec/soos/soos-daemon` (mode `0755 root:root`). The command-line utilities are installed to `/usr/bin/soos-admin` and `/usr/bin/soos-enroll` (mode `0755 root:root`). PAM shared library `pam_soos.so` is placed in distribution PAM module paths (`0644 root:root`).
- **Permissions and Ownership**: All package specs enforce `/var/lib/soos/` mode `0755`, `/var/lib/soos/{biometrics,evidence}` mode `0700 root:root`, `/var/lib/soos/master.key` mode `0600 root:root` (32 bytes), and `/run/soos/` mode `0750 root:soos`.
- **System Group**: Every package spec registers the dedicated system group `soos` prior to file deployment or in post-install hooks.
- **Compliance**: **PASS**

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Non-Interference**: Packaging files deploy distribution PAM templates strictly matching the universal stack ordering in `AI/ARCHITECTURE.md` §5:
  1. `auth [success=done default=ignore] pam_soos.so timeout_ms=250`
  2. `auth required pam_unix.so`
  3. `auth optional pam_soos.so event=password-failed timeout_ms=20`
- **Zero Runtime Bloat**: Zero asynchronous runtimes or background daemon threads are introduced into PAM pathways.
- **Compliance**: **PASS**

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Failure Resilience**: Package install and removal scripts use strict `set -euo pipefail` and conditional checks (`getent group`, `command -v`).
- **PAM Safety**: In the event of daemon failure or package removal, `default=ignore` ensures the PAM stack falls back to password verification without causing lockout or crashing display managers.
- **Compliance**: **PASS**

### Pillar 4: Dependency Isolation & Banned Crates
- **Banned Crates**: Neither `opencv` nor `nokhwa` are used or referenced.
- **Clean Tooling**: Package building scripts rely on standard distribution packaging tools (`dpkg-deb`, `rpmbuild`, `makepkg`, `cargo`), without introducing forbidden crates or unvetted foreign dependencies.
- **Compliance**: **PASS**

### Pillar 5: Data Confidentiality & Zeroization
- **Key Protection**: Master key generation ensures `0600` permissions immediately upon creation via `openssl rand 32` or `/dev/urandom`.
- **Zero Credential Exposure**: Package installation scripts and spec files never log, transmit, or expose passwords, raw frames, or biometric vectors.
- **Compliance**: **PASS**

### Pillar 6: Test Integrity & TDD Contracts
- **Test Authoring**: Invariant unit tests covering Debian, RPM, and Arch Linux package specs and scripts are authored in Phase 2 before production packaging scripts and specs are completed.
- **Zero Test Weakening**: Existing invariant and pipeline tests in `tests/` remain immutable and intact.
- **Acceptance Criteria**: Directly mirrors acceptance criteria defined in `AI/BACKLOG.md` #27.1, #27.2, and #27.3.
- **Compliance**: **PASS**

---

## Formal Evaluation Verdict

The implementation plan is exhaustive, fully compliant with `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, and `AI/BACKLOG.md`, and satisfies all 6 architectural pillars.

**VALIDATION_VERDICT: APPROVED**

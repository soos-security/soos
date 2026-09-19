# Plan Evaluation Report — Issue #26: Installation Script & System Provisioning

- **Date**: 2026-09-19
- **Evaluator**: Independent Plan Evaluator Sub-Agent (`plan-evaluator`)
- **Target**: Issue #26 (`feat(packaging): Installation script and system provisioning`) / GitHub #65
- **Branch**: `feat/install-script`

---

## Executive Summary

The proposed implementation plan addresses all four sub-issues of Issue #26:
1. `#26.1`: `scripts/install.sh` system provisioning, directory hierarchy, permissions, binary placement, master key generation, and model verification.
2. `#26.2`: Distribution-specific PAM configurations (Debian `pam-auth-update`, Fedora `authselect`, Arch direct snippet) enforcing universal PAM stack ordering from `AI/ARCHITECTURE.md` §5.
3. `#26.3`: `scripts/uninstall.sh` safe rollback, PAM restoration, service teardown, and granular data preservation (`--keep-data` / `--purge-data`).
4. `#26.4`: `soos-admin add-user <username>` subcommand provisioning users to the `soos` system group via `usermod -aG soos`.

---

## 6-Pillar Compliance Assessment

### Pillar 1: Architectural Alignment & Threat Model
- **Boundary Preservation**: The daemon binary is installed to `/usr/libexec/soos/soos-daemon` and managed via systemd sandboxing (`packaging/soos-daemon.service`). PAM shared object `pam_soos.so` is installed into system security modules directories (`/lib/security`, `/usr/lib64/security`, etc.).
- **Permissions and Ownership**: `/var/lib/soos/{biometrics,evidence}` are provisioned with mode `0700` owned by `root:root`. Master key `/var/lib/soos/master.key` is created with mode `0600` owned by `root:root`. Runtime directory `/run/soos` is assigned mode `0750` owned by `root:soos`.
- **System Group**: System group `soos` is created without login shell or home directory, strictly for group access control to `/run/soos/daemon.sock`.
- **Compliance**: **PASS**

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Zero Asynchronous Runtime**: The installation scripts and PAM configuration templates introduce zero async runtimes or background daemon threads into the PAM pathway.
- **Stack Ordering**: The ordering specified in `AI/ARCHITECTURE.md` §5 is strictly preserved across all distribution templates:
  1. `pam_soos.so` before `pam_unix` with `timeout_ms=250` and `[success=done default=ignore]`.
  2. `pam_unix` password fallback.
  3. `pam_soos.so` with `event=password-failed timeout_ms=20` after `pam_unix`.
- **Compliance**: **PASS**

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Non-Interference**: If PAM module is uninstalled or absent, PAM control flags (`default=ignore`) guarantee transparent fallback to password authentication without locking out users.
- **Command Safety in `admin-cli`**: Username input for `add-user` is strictly validated against POSIX username specifications (`^[a-z_][a-z0-9_-]*\$?$`, <= 32 chars) prior to argument dispatch, preventing shell injection or malformed execution.
- **Error Propagation**: All errors in `admin-cli` are mapped to typed `AdminCliError` variants; zero `unwrap()` or `expect()` in production code.
- **Compliance**: **PASS**

### Pillar 4: Dependency Isolation & Banned Crates
- **Banned Crates**: Neither `opencv` nor `nokhwa` are used or introduced.
- **Crate Scope**: `crates/admin-cli` maintains `#![forbid(unsafe_code)]` and consumes only authorized workspace dependencies (`clap`, `nix`, `thiserror`).
- **Standard Tooling**: `install.sh` and `uninstall.sh` utilize standard POSIX shell constructs and utilities (`groupadd`, `usermod`, `chmod`, `install`, `openssl` / `dd`), avoiding external dependencies.
- **Compliance**: **PASS**

### Pillar 5: Data Confidentiality & Zeroization
- **Credential Protection**: Zero passwords, biometric templates, or key material are logged, printed to stdout, or exposed during installation, uninstallation, or group management.
- **Safe Rollback**: `uninstall.sh` defaults to preserving encrypted biometric templates and master key unless `--purge-data` is explicitly instructed.
- **Compliance**: **PASS**

### Pillar 6: Test Integrity & TDD Contracts
- **Contractual Tests**: The plan incorporates all four required TDD tests from `AI/BACKLOG.md`:
  - `test_install_script_creates_required_directories`
  - `test_pam_config_ordering_matches_spec`
  - `test_uninstall_restores_pam_config`
  - `test_add_user_to_soos_group`
- **Zero Weakening**: Pre-existing tests in `tests/invariants` and `crates/admin-cli` will remain completely untouched.
- **Compliance**: **PASS**

---

## Formal Evaluation Verdict

The implementation plan is exhaustive, fully compliant with `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, and `AI/BACKLOG.md`, and satisfies all 6 architectural pillars.

**VALIDATION_VERDICT: APPROVED**

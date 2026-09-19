# Walkthrough 49 — Distribution-Specific Deployment Validation

## Overview

This walkthrough documents the design, implementation, and automated validation of the distribution-specific deployment test suite for **Issue #32 / GitHub Issue #71**:
`test(integration): Distribution-specific deployment validation`.

The suite covers the three Tier-1 Linux distribution families supported by `soos`:
1. **Debian 12 / Ubuntu 24.04** (`#32.1`)
2. **Fedora 40 / RHEL 9** (`#32.2`)
3. **Arch Linux** (`#32.3`)

---

## Changes Made

### 1. Multi-Distribution Test Harness (`tests/distro/`)
- **`tests/distro/debian_ubuntu_test.sh`**:
  - Validates `.deb` package installation via `dpkg -i` and `scripts/install.sh`.
  - Asserts directory permissions: `/var/lib/soos/{biometrics,evidence}` (`0700`), `master.key` (`0600`), `/run/soos` (`0750`).
  - Verifies Debian `pam-auth-update` profiles (`soos` and `soos-notify`) and `/etc/pam.d/common-auth` ordering.
  - Tests user enrollment and encrypted template creation (`0600` `root:root`).
  - Tests nominal facial authentication (`Verdict::Allow` -> `PAM_SUCCESS`, 0 password prompts).
  - Tests password fallback (`Verdict::Deny` / offline daemon -> `PAM_IGNORE` -> password authentication).
  - Verifies safe rollback and uninstallation via `scripts/uninstall.sh --keep-data` or `dpkg -r`, preserving biometric data for recovery.
- **`tests/distro/fedora_rhel_test.sh`**:
  - Validates `.rpm` package installation via `rpm -i` and `scripts/install.sh`.
  - Asserts custom `authselect` profile template deployment in `/etc/authselect/custom/soos/`.
  - Strictly verifies the preservation of `pam_faillock.so` (`preauth` before `pam_soos.so` and `authfail` after `pam_unix.so`), ensuring lockout accounting is never bypassed.
  - Verifies PAM service stack integration for `sudo` and `gdm` (`gdm-password`).
  - Tests nominal facial auth and password fallback on Fedora stacks.
  - Verifies safe profile rollback via `authselect select local` and package uninstallation (`rpm -e`).
- **`tests/distro/arch_linux_test.sh`**:
  - Validates Arch Linux `PKGBUILD` packaging and `pacman -U` installation.
  - Verifies snippet placement in `/etc/pam.d/system-auth`.
  - Tests Wayland screen locker integration (`swaylock`, `hyprlock`), verifying instant facial unlock and graceful password fallback without stdout/stderr pollution.
  - Verifies package removal via `pacman -R` and `scripts/uninstall.sh`.
- **`tests/distro/run_distro_validation.sh`**:
  - Unified multi-distribution orchestrator supporting auto-detection of host OS (`/etc/os-release`), direct local execution, and Dockerized container runs (`ubuntu`, `fedora`, `arch`, `all`).

### 2. PAM Templates Hardening
- **`packaging/pam/fedora/soos/system-auth` & `password-auth`**:
  - Enhanced custom profile templates with `{?with-faillock:auth required pam_faillock.so preauth silent}` and `{?with-faillock:auth [default=die] pam_faillock.so authfail}` to guarantee account lockout protection.

### 3. Comprehensive Deployment Documentation
- **`Docs/DISTRIBUTION_DEPLOYMENT.md`**:
  - Step-by-step guides for Debian 12 / Ubuntu 24.04, Fedora 40 / RHEL 9 (`authselect`), and Arch Linux (`swaylock`/`hyprlock`).
  - Emergency rescue shell and disaster recovery instructions.

### 4. Architectural Invariant Tests
- **`tests/invariants/src/lib.rs`**:
  - Added `test_distro_validation_suite_spec`:
    - Enforces script existence, `0755` executable permissions, and strict bash flags (`set -euo pipefail`).
    - Enforces `--help` CLI flag execution with exit code 0.
    - Validates required distribution test scenarios, `pam_faillock`, `authselect`, `swaylock`, and rollback coverage.

---

## Verification & Test Results

### 1. Invariant Test Contract
```bash
cargo test -p soos-invariants -- test_distro_validation_suite_spec
```
Output:
```text
running 1 test
test tests::test_distro_validation_suite_spec ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 20 filtered out; finished in 0.03s
```

### 2. Multi-Distribution Test Harness Dry-Run
```bash
bash tests/distro/run_distro_validation.sh --dry-run all
```
Output:
```text
[OK]  Debian 12 / Ubuntu 24.04 Deployment Validation completed successfully.
[OK]  Fedora 40 / RHEL 9 Deployment Validation completed successfully.
[OK]  Arch Linux Deployment Validation completed successfully.
[OK]  DISTRIBUTION DEPLOYMENT VALIDATION COMPLETED SUCCESSFULLY!
```

### 3. Workspace-Wide Test Suite
```bash
cargo test --workspace
```
Output:
```text
100% of tests passed across all 11 workspace crates and test suites.
```

### 4. Code Quality & Formatting
```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```
Output:
```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.47s
Zero warnings, formatting compliant.
```

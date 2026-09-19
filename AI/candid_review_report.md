# Candid Review Report

- **Date**: 2026-09-19
- **Target Branch / Commit**: `feat/install-script`
- **Audited Files**:
  - `crates/admin-cli/src/args.rs`
  - `crates/admin-cli/src/error.rs`
  - `crates/admin-cli/src/lib.rs`
  - `crates/admin-cli/src/main.rs`
  - `crates/admin-cli/src/user.rs`
  - `crates/admin-cli/tests/add_user_tests.rs`
  - `packaging/pam/debian/soos`
  - `packaging/pam/debian/soos-notify`
  - `packaging/pam/fedora/soos/README`
  - `packaging/pam/fedora/soos/REQUIREMENTS`
  - `packaging/pam/fedora/soos/system-auth`
  - `packaging/pam/fedora/soos/password-auth`
  - `packaging/pam/arch/system-auth`
  - `packaging/pam/arch/system-auth.snippet`
  - `scripts/install.sh`
  - `scripts/uninstall.sh`
  - `scripts/sync_issue.py`
  - `tests/invariants/src/lib.rs`

## 1. Executive Summary

This pull request implements comprehensive installation, provisioning, and administration tooling for `soos` local biometric PAM, addressing Issue #26 (GitHub #65). The changes include:
1. `scripts/install.sh`: Fully featured installer provisioning strict directory permissions (`0700` for biometrics/evidence, `0600` for master key, `0750` for runtime socket), binary deployments, master key generation, model attestation, and distribution PAM helpers.
2. Distribution PAM configurations: Debian `pam-auth-update` profiles, Fedora `authselect` custom profile, and Arch Linux `/etc/pam.d/system-auth` configs strictly preserving universal PAM stack ordering (`pam_soos.so` before `pam_unix`, and `event=password-failed` after `pam_unix`).
3. `scripts/uninstall.sh`: Safe teardown restoring PAM configuration backups, stopping systemd units, removing binaries, and preserving biometric data by default unless `--purge-data` is requested.
4. `soos-admin add-user <username>`: Strict POSIX username validation and execution of `usermod -aG soos <username>`.
5. Comprehensive contractual tests satisfying all acceptance criteria in `AI/BACKLOG.md` and `AI/VERIFICATION_MATRIX.md`.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**: State transitions, directory hierarchies, binary locations, and fallback logic conform strictly to `AI/ARCHITECTURE.md` §5 and §10.
- All scripts handle `--destdir`, `--dry-run`, and error conditions fail-closed with `set -euo pipefail`.
- PAM stack ordering places `pam_soos.so` with `timeout_ms=250` before `pam_unix`, and `event=password-failed timeout_ms=20` after `pam_unix`.

### PAM Concurrency & Deadlines
- **Pass**: Zero asynchronous runtimes or threads introduced into the PAM module pathway.
- PAM configuration rules use synchronous bounded deadlines (`timeout_ms=250` and `timeout_ms=20`).

### Panic Safety & Fallback
- **Pass**: Zero `unwrap()`, `expect()`, or `panic!()` in `crates/admin-cli` production code.
- User input is validated before execution; errors are safely returned via `AdminCliError`.
- PAM configurations configure `[success=done default=ignore]`, ensuring that module absence, failure, or daemon crash gracefully falls back to password authentication without locking out users.

### Test Integrity & Anti-Weakening
- **Pass**: Pre-existing tests in `soos-invariants` and `soos-admin-cli` remain completely untouched.
- Four new contractual tests (`test_install_script_creates_required_directories`, `test_pam_config_ordering_matches_spec`, `test_uninstall_restores_pam_config`, `test_add_user_to_soos_group`) were authored in the TDD Red phase and rigorously test nominal and failure paths.

### Memory & Secret Bounds
- **Pass**: Zero passwords, biometric templates, or cryptographic keys are logged or printed to stdout.
- Master key generation ensures `0600` permissions and exact 32-byte cryptographic entropy.
- Sensitive directories `/var/lib/soos/biometrics` and `/var/lib/soos/evidence` are restricted to `0700` (`root:root`).

## 3. Detailed Findings & Action Items
- None. All checks passed with zero warnings.

## 4. Final Verdict
**VERDICT: APPROVED**

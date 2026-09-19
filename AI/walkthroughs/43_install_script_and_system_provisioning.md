# Walkthrough 43 — Installation Script & System Provisioning

## Context & Objectives

- **Issue**: Issue #26 (`feat(packaging): Installation script and system provisioning`) / GitHub #65
- **Branch**: `feat/install-script`
- **Mission**:
  1. Author `scripts/install.sh` system provisioning script (`soos` system group, directory hierarchy with strict permissions, binary and PAM module deployments, systemd unit, master key generation, model attestation).
  2. Create distribution-specific PAM configuration profiles in `packaging/pam/` for Debian (`pam-auth-update`), Fedora (`authselect`), and Arch Linux (`system-auth`), strictly preserving universal PAM stack ordering from `AI/ARCHITECTURE.md` §5.
  3. Author `scripts/uninstall.sh` safe uninstallation script restoring PAM configuration backups, stopping systemd units, removing binaries, and supporting `--keep-data` and `--purge-data`.
  4. Implement `soos-admin add-user <username>` subcommand provisioning users to the `soos` system group via `usermod -aG soos <username>` with strict POSIX username validation.
  5. Author contractual TDD tests for all sub-issues and integrate into automated CI invariant checks.

---

## 1. Architectural Design & Types (Phase 1 & Phase 1.5)

- **Plan Evaluator Sub-Agent**:
  Evaluated the architecture across 6 core pillars in `AI/plan_evaluator_report.md` and issued `VALIDATION_VERDICT: APPROVED`.
- **Directory Hierarchy & Permissions**:
  - `/var/lib/soos/biometrics`: mode `0700`, owner `root:root`
  - `/var/lib/soos/evidence`: mode `0700`, owner `root:root`
  - `/var/lib/soos/models`: mode `0755`, owner `root:root`
  - `/var/lib/soos/master.key`: mode `0600`, owner `root:root`, 32 bytes
  - `/run/soos`: mode `0750`, owner `root:soos`
- **Distribution PAM Configuration Ordering**:
  Preserved universal PAM stack ordering across all distributions:
  1. `auth [success=done default=ignore] pam_soos.so timeout_ms=250`
  2. `auth [success=done default=bad] pam_unix.so try_first_pass`
  3. `auth optional pam_soos.so event=password-failed timeout_ms=20`

---

## 2. Test Contracts (Phase 2 - TDD Red Phase)

Contractual tests were authored before production implementation:
1. `tests/invariants/src/lib.rs`:
   - `test_pam_config_ordering_matches_spec`: Validates that Debian, Fedora, and Arch PAM templates exist and maintain `pam_soos.so` before `pam_unix`, and `event=password-failed` after `pam_unix`.
   - `test_install_script_creates_required_directories`: Executes `scripts/install.sh --destdir <tmpdir> --skip-models --skip-systemd`, asserting exact mode `0700` for biometrics/evidence, mode `0600` for 32-byte `master.key`, and directory presence.
   - `test_uninstall_restores_pam_config`: Simulates installation, active PAM modification with backup, runs `scripts/uninstall.sh --destdir <tmpdir> --keep-data`, asserting binary removal, biometric retention, and PAM backup restoration.
2. `crates/admin-cli/tests/add_user_tests.rs`:
   - `test_add_user_to_soos_group`: Tests POSIX username validation (rejecting empty, spaces, shell metacharacters, names > 32 chars) and dispatches `usermod -aG soos <username>`.

All tests failed initially during Phase 2 as required.

---

## 3. Implementation (Phase 3 & Phase 4)

- **`crates/admin-cli`**:
  - Added `AddUser(AddUserArgs)` subcommand to `Commands`.
  - Created `crates/admin-cli/src/user.rs` with `validate_username`, `add_user_to_group_with_runner`, and `add_user_to_soos_group`.
  - Added typed error variants `InvalidUsername`, `UserNotFound`, `CommandFailed` to `AdminCliError`.
- **Distribution PAM Configurations**:
  - `packaging/pam/debian/soos`: `pam-auth-update` Primary profile (Priority 260).
  - `packaging/pam/debian/soos-notify`: `pam-auth-update` Additional profile (Priority 128).
  - `packaging/pam/fedora/soos/`: Custom `authselect` profile template (`system-auth`, `password-auth`, `README`, `REQUIREMENTS`).
  - `packaging/pam/arch/system-auth` and `system-auth.snippet`: Arch Linux drop-in configurations.
- **`scripts/install.sh`**:
  - Fully optioned POSIX/bash installer supporting `--destdir`, `--prefix`, `--pam-dir`, `--skip-models`, `--skip-systemd`, and `--dry-run`.
  - Provisions directories, generates master key if absent, installs binaries/PAM module, configures systemd, and triggers model download.
- **`scripts/uninstall.sh`**:
  - Safe rollback restoring PAM backups (`*.soos-backup`), disabling systemd units, cleaning binaries, and preserving biometric data by default unless `--purge-data` is requested.

---

## 4. Candid Review & Auditing (Phase 5)

- Ran `scripts/candid_review.sh`: All 7 architectural audits passed cleanly.
- Authored formal review in `AI/candid_review_report.md` with `VERDICT: APPROVED`.

---

## 5. Verification Results

- `cargo test -p soos-invariants`: 15 passed, 0 failed.
- `cargo test -p soos-admin-cli`: 23 passed, 0 failed.
- `cargo test --workspace`: 100% test suites passed across all workspace crates.
- `cargo clippy --all-targets --all-features -- -D warnings`: 0 warnings.
- `cargo fmt --check`: 100% compliant.

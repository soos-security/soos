# Walkthrough 52 — CLI Hardening: Root Bypass Elimination, Subcommand Privilege Enforcement, Path Sanitization, and Systemd StateDirectory

## Context & Objectives

- **Issue**: Issue #35 (`fix(cli): Remove root bypass and enforce path validation`) / GitHub Issue #74
- **Branch**: `fix/cli-security`
- **Mission**:
  1. Remove `--skip-root-check` and enforce `check_privileges` on all subcommands (#35.1).
  2. Validate `PathBuf` arguments against FHS paths or sanitize them (#35.2).
  3. Fix systemd `StateDirectory` and correct `Group=soos` ownership in `soos-daemon.service` (#35.3).

---

## 1. Architectural Design & Compliance (Phase 1 & Phase 1.5)

- **Plan Evaluator Sub-Agent**:
  Audited the implementation plan across the 6 architectural pillars in `AI/plan_evaluator_report.md` with explicit verdict: `VALIDATION_VERDICT: APPROVED`.
- **Architectural & Security Invariants Honored**:
  - `AI/ARCHITECTURE.md` §11 (Installation & CLI): `soos-enroll` operates on root-restricted persistent state (`/var/lib/soos/`). Hidden CLI flags bypassing privilege checks represent a security violation and must be eliminated.
  - Privilege Separation & Least Privilege: Information enumeration via `list` and template verification via `verify` must strictly require root privileges (EUID 0) to prevent unprivileged local users from probing biometric assets or querying enrolled users.
  - Path Traversal Prevention: User-supplied path parameters must not escape permitted FHS directories. Traversal components (`..`), relative paths, or disallowed directories must fail closed immediately.
  - Daemon Sandbox Alignment (`AI/ARCHITECTURE.md` §10): The privileged daemon runs with `Group=soos`, matching the ownership of `/run/soos/` (mode `0750`, owner `root:soos`). Systemd unit `packaging/soos-daemon.service` must declare `Group=soos` and `StateDirectory=soos` (`StateDirectoryMode=0755`) for automated directory provisioning.

---

## 2. Test Contracts (Phase 2 — TDD Red Phase)

Contractual tests were authored before updating production code:
1. `crates/daemon/tests/systemd_test.rs`:
   - Enforced that `soos-daemon.service` contains mandatory directives `Group=soos` and `StateDirectory=soos`.
   - Explicitly asserted that `Group=root` is absent.
   - **Initial Failure (RED)**: Failed with `SECURITY INVARIANT VIOLATION: systemd service missing mandatory sandbox directive 'Group=soos'`.
2. `crates/enrollment-cli/tests/root_check_tests.rs`:
   - `test_verify_subcommand_enforces_root_privileges`: Confirms that `verify()` returns `Err(RootRequired)` when invoked without root privileges.
   - `test_list_subcommand_enforces_root_privileges`: Confirms that `list()` returns `Err(RootRequired)` when invoked without root privileges.
   - `test_delete_subcommand_enforces_root_privileges`: Confirms that `delete()` returns `Err(RootRequired)` when invoked without root privileges.
   - **Initial Failure (RED)**: `test_list_subcommand_enforces_root_privileges` returned `Ok([])` and `test_verify_subcommand_enforces_root_privileges` returned `Err(NotEnrolled(1000))` instead of `Err(RootRequired)`.
3. `crates/enrollment-cli/tests/path_validation_tests.rs`:
   - `test_sanitize_path_blocks_parent_dir_traversal`: Validates that paths containing `..` are rejected.
   - `test_sanitize_path_blocks_relative_paths`: Validates that relative paths are rejected.
   - `test_validate_fhs_path_allows_valid_system_directories`: Validates that standard FHS hierarchies (`/var`, `/run`, `/etc`, `/usr`, `/tmp`, `/dev`, `/opt`, `/home`) are accepted and normalized.
   - `test_validate_fhs_path_rejects_non_fhs_locations`: Validates that non-FHS paths (`/root`, `/boot`, `/srv`, `/proc`, `/sys`) are rejected.
   - `test_validate_camera_device_path_requires_dev`: Enforces that camera devices must be nodes inside `/dev/`.
   - `test_cli_rejects_skip_root_check_argument`: Asserts that clap rejects `--skip-root-check`.
   - `test_build_store_only_rejects_traversal_paths` & `test_build_full_service_rejects_non_dev_camera`: Assert that service builders fail closed on malformed paths.

---

## 3. Implementation (Phase 3 & Phase 4)

- **Root Privilege Enforcement (`crates/enrollment-cli/src/main.rs` & `service.rs`)**:
  - Removed `skip_root_check` argument from `Cli` struct in `crates/enrollment-cli/src/args.rs`.
  - Added `check_privileges(true)?;` at the start of `run()` in `main.rs`.
  - Enforced `check_privileges(self.require_root)?;` in `EnrollmentService::verify()` and `EnrollmentService::list()`.
  - Set `require_root = true` by default in `build_store_only()` and `build_full_service()`.
- **Path Sanitization & FHS Validation (`crates/enrollment-cli/src/args.rs` & `service.rs`)**:
  - Implemented `sanitize_path(&Path) -> Result<PathBuf, EnrollmentCliError>`:
    - Asserts `path.is_absolute()`.
    - Scans components for `Component::ParentDir` (`..`) and fails with `EnrollmentCliError::InvalidPath`.
    - Normalizes path using canonical component assembly.
  - Implemented `validate_fhs_path(&Path) -> Result<PathBuf, EnrollmentCliError>`:
    - Verifies path component starts with `ALLOWED_FHS_PREFIXES` (`/var`, `/run`, `/etc`, `/usr`, `/tmp`, `/dev`, `/opt`, `/home`).
  - Implemented `validate_camera_device_path(&Path) -> Result<PathBuf, EnrollmentCliError>`:
    - Enforces that camera path resides under `/dev/` and is not `/dev` itself.
  - Added `EnrollmentCliError::InvalidPath(String)` in `crates/enrollment-cli/src/error.rs`.
  - Integrated path validation in `build_store_only` and `build_full_service` before opening file descriptors or loading models.
- **Systemd Hardening (`packaging/soos-daemon.service`)**:
  - Corrected group ownership from `Group=root` to `Group=soos`.
  - Added `StateDirectory=soos` with `StateDirectoryMode=0755` for `/var/lib/soos` provisioning.
  - Updated architectural references in `AI/ARCHITECTURE.md` and `Docs/PACKAGING_AND_PROVISIONING.md`.

---

## 4. Candid Review & Verification Matrix (Phase 5 & Phase 6)

- **Candid Review Report**:
  - Conducted cold diff review against `origin/main` across the 5 pillars.
  - Authored `AI/candid_review_report.md` with **`VERDICT: APPROVED`**.
- **Dual Issue Synchronization**:
  - Synchronized sub-issues #35.1, #35.2, and #35.3 in `AI/BACKLOG.md` and GitHub Issue #74 via `scripts/sync_issue.py`.
  - Executed `python3 scripts/sync_issue.py --auto` to mark all issue tasks complete.
- **Formal Verification Matrix (`AI/VERIFICATION_MATRIX.md`)**:
  - Added criterion `EN10` (Path validation & traversal prevention) -> `✅ Verified`.
  - Added criterion `EN11` (Subcommand privilege enforcement & bypass removal) -> `✅ Verified`.
  - Updated criterion `H3` (Systemd sandboxing validation) to include `Group=soos` and `StateDirectory=soos` -> `✅ Verified`.

---

## 5. Automated Verification Results

```bash
cargo test -p soos-enrollment-cli
```
- 38 unit and integration tests passed cleanly (including 8 path validation tests and 6 root check tests).

```bash
cargo test -p soos-daemon --test systemd_test
```
- `test_systemd_unit_file_sandboxing_directives` passed cleanly.

```bash
cargo test --workspace
```
- All tests across all workspace crates passed (100% green).

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
- Zero Clippy warnings across all workspace crates.

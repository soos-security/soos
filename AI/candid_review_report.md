# Candid Review Report

- **Date**: 2026-09-20
- **Target Branch / Commit**: `fix/cli-security`
- **Audited Files**:
  - `crates/enrollment-cli/src/args.rs`
  - `crates/enrollment-cli/src/error.rs`
  - `crates/enrollment-cli/src/lib.rs`
  - `crates/enrollment-cli/src/main.rs`
  - `crates/enrollment-cli/src/service.rs`
  - `crates/enrollment-cli/tests/path_validation_tests.rs`
  - `crates/enrollment-cli/tests/root_check_tests.rs`
  - `crates/enrollment-cli/tests/scaffold_tests.rs`
  - `crates/enrollment-cli/tests/delete_tests.rs`
  - `crates/enrollment-cli/tests/list_tests.rs`
  - `crates/daemon/tests/systemd_test.rs`
  - `packaging/soos-daemon.service`
  - `AI/ARCHITECTURE.md`
  - `scripts/sync_issue.py`

## 1. Executive Summary

This patch addresses Issue #35 by removing the hidden unprivileged CLI bypass flag (`--skip-root-check`) from `soos-enroll`, systematically enforcing root privileges across all subcommands (`enroll`, `verify`, `delete`, `list`), introducing rigorous path sanitization and FHS hierarchy boundary validation, and hardening `soos-daemon.service` with `Group=soos` and `StateDirectory=soos`. The changes are robust, defensive, panic-free, and thoroughly validated with contractual automated tests.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**:
  - `Cli` struct no longer accepts `--skip-root-check`, preventing runtime bypasses of administrative privilege checks.
  - `check_privileges` is now called at the entry points of `service.verify` and `service.list`, as well as at binary entry in `main.rs`, completing privilege enforcement across all four subcommands.
  - `sanitize_path`, `validate_fhs_path`, and `validate_camera_device_path` enforce zero path traversal (`..`), mandate absolute paths, and restrict assets to permitted FHS hierarchies and camera devices strictly to `/dev/`.
  - Service builders `build_store_only` and `build_full_service` validate all paths up front before initializing disk access or neural pipelines.
  - `soos-daemon.service` declares `Group=soos` and `StateDirectory=soos` with `StateDirectoryMode=0755`, matching IPC directory permissions and systemd best practices.

### PAM Concurrency & Deadlines
- **Pass**:
  - No changes in `crates/pam`.
  - No asynchronous runtimes or threads introduced into synchronous modules.

### Panic Safety & Fallback
- **Pass**:
  - Zero `unwrap()` or `expect()` in production code.
  - All path validation, camera device checks, and privilege checks return typed `Result<T, EnrollmentCliError>`.
  - Fail-closed behavior on all validation failures.

### Test Integrity & Anti-Weakening
- **Pass**:
  - Dedicated contractual test suites `path_validation_tests.rs` and `root_check_tests.rs` author rigorous positive and negative test cases.
  - Pre-existing tests in `delete_tests.rs`, `list_tests.rs`, and `scaffold_tests.rs` were updated only to remove the obsolete `skip_root_check` struct field while preserving their underlying contractual assertions.
  - Contractual test `systemd_test.rs` updated to strictly require `Group=soos`, `StateDirectory=soos`, and explicitly forbid `Group=root`.

### Memory & Secret Bounds
- **Pass**:
  - Path normalization operates on bounded path components without unbounded buffer growth.
  - Zero sensitive cryptographic keys or biometric templates exposed in error messages or logs.
  - `#![forbid(unsafe_code)]` maintained in `crates/enrollment-cli`.

## 3. Detailed Findings & Action Items
- None. All security invariants, architectural requirements, and test contracts are satisfied.

## 4. Final Verdict
**VERDICT: APPROVED**

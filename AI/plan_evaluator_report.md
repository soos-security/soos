# Plan Evaluation Report — Issue #35: fix(cli): Remove root bypass and enforce path validation

- **Date**: 2026-09-19
- **Target Issue**: Issue #35 (Sub-issues #35.1, #35.2, #35.3)
- **Target Branch**: `fix/cli-security`
- **Evaluator**: Plan Evaluator Sub-Agent (Autonomous Gate)

---

## 1. Executive Summary

This plan addresses critical security hygiene in `soos-enroll` (`crates/enrollment-cli`) and daemon systemd sandboxing (`packaging/soos-daemon.service`). Specifically:
1. Completely removes the hidden CLI bypass flag `--skip-root-check` from argument parsing (`Cli`), eliminating arbitrary unprivileged elevation pathways in production.
2. Systematically enforces EUID 0 root privilege verification (`check_privileges`) across all 4 administrative subcommands: `enroll`, `verify`, `delete`, and `list` (previously omitted on `verify` and `list`).
3. Introduces strict path validation and sanitization (`sanitize_path`, `validate_fhs_path`, `validate_camera_device_path`), rejecting directory traversal (`..`), relative paths, and non-FHS boundaries for biometric directory, master key, neural models, and camera device arguments.
4. Hardens `packaging/soos-daemon.service` by assigning daemon group ownership to `Group=soos` (aligning with `/run/soos/` mode 0750 IPC socket directory) and declaring `StateDirectory=soos` with `StateDirectoryMode=0755` for automated `/var/lib/soos` provisioning.

---

## 2. Six-Pillar Architectural Audit

### Pillar 1: Architectural Alignment & Threat Model
- **Evaluation**: PASS
- **Rationale**:
  - `soos-enroll` is an administrative tool operating directly on root-restricted persistent state in `/var/lib/soos/` (biometric vectors, cryptographic master keys). An unprivileged attacker must not be able to bypass root checks using hidden CLI flags or traverse outside permitted FHS hierarchies.
  - Enforcing EUID 0 on `verify` and `list` closes the information disclosure hole where non-root callers could enumerate enrolled UIDs or verify against enrolled templates.
  - Setting `Group=soos` in `soos-daemon.service` complies with `AI/ARCHITECTURE.md` §4 and §10 where IPC socket files are owned by `root:soos` (mode 0660).
  - Adding `StateDirectory=soos` leverages standard systemd directory lifecycle management for persistent state under `/var/lib/soos`.

### Pillar 2: PAM Real-Time Deadlines & Concurrency
- **Evaluation**: PASS
- **Rationale**:
  - The changes are strictly confined to `enrollment-cli`, `packaging/soos-daemon.service`, and `daemon/tests/systemd_test.rs`.
  - Zero modifications to `crates/pam`, ensuring zero impact on PAM real-time deadline budgets (200–250ms) or asynchronous runtime prohibition.

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Evaluation**: PASS
- **Rationale**:
  - All path sanitization and privilege checking functions return typed `Result<T, EnrollmentCliError>`.
  - No `unwrap()` or `expect()` is introduced into library or binary production code.
  - Path traversal attempts, relative paths, or disallowed FHS prefixes fail closed with typed `EnrollmentCliError::InvalidPath`.
  - Missing root privileges fail closed immediately with `EnrollmentCliError::RootRequired`.

### Pillar 4: Dependency Isolation & Banned Crates
- **Evaluation**: PASS
- **Rationale**:
  - No new external crates are introduced. Path validation uses standard library `std::path::{Component, Path, PathBuf}`.
  - Business crate invariant `#![forbid(unsafe_code)]` remains strictly enforced in `crates/enrollment-cli`.
  - Prohibition against OpenCV and nokhwa remains 100% intact.

### Pillar 5: Data Confidentiality & Zeroization
- **Evaluation**: PASS
- **Rationale**:
  - Biometric embeddings and master keys remain strictly protected under `/var/lib/soos/` with mode `0600` / `0700`.
  - Path sanitization prevents attackers from directing key creation or template deletion to unintended system paths (e.g. `/etc/shadow`, `/boot`).
  - Sensitive buffers continue to use `Zeroizing` as established in previous phases.

### Pillar 6: Test Integrity & Acceptance Criteria
- **Evaluation**: PASS
- **Rationale**:
  - Contractual test suite in Phase 2 will author explicit negative tests:
    - Rejection of `--skip-root-check` CLI argument.
    - Privilege enforcement on all 4 subcommands (`enroll`, `verify`, `delete`, `list`).
    - Traversal rejection (`..`), non-absolute path rejection, and camera path restriction.
    - Systemd unit verification for `Group=soos` and `StateDirectory=soos`.
  - Acceptance criteria EN1, EN2, EN8, and H3 are fully respected and strengthened.

---

## 3. Formal Conclusion & Autonomous Clearance

The implementation plan satisfies all zero-trust architectural invariants, security guidelines, and backlog requirements for Issue #35.

**VALIDATION_VERDICT: APPROVED**

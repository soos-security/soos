# Candid Review Report: `admin-cli` Diagnostics Tool (#18)

- **Date**: 2026-09-14
- **Target Branch**: feat/admin-cli
- **Base Reference**: origin/main
- **Reviewer**: Candid Reviewer Sub-Agent (`.agents/skills/candid-reviewer`)
- **Status**: Complete

---

## 1. Executive Summary

An independent, cold diff audit was performed on the `feat/admin-cli` branch implementing Issue #11 / GitHub Issue #18 (`crates/admin-cli`). The implementation provides the non-biometric diagnostic CLI tool (`soos-admin`) supporting `status`, `test-pam`, and `logs` subcommands. The changeset includes crate scaffolding, protocol types (`RequestKind::Status`, `StatusResponse`), daemon dispatcher integration, comprehensive log redaction, and 22 contractual unit and integration tests.

---

## 2. Five-Pillar Deep Reasoning Audit

### Pillar 1: Logic & Architecture
- **Evaluation**: **PASS**
- **Findings**:
  - `soos-admin-cli` provides clear separation of responsibilities across `status`, `test_pam`, `logs`, and `redact` modules.
  - The `status` command queries both the local Unix Domain Socket for component readiness (`socket_ready`, `camera_ready`, `models_verified`, PID, uptime) and the systemd manager for unit execution state (`ActiveState`, `SubState`).
  - When the daemon is offline, `query_status` gracefully reports offline status and systemd unit state rather than terminating prematurely or panicking.
  - Protocol extension: `RequestKind::Status` cleanly routes to daemon component readiness snapshots, serializing `StatusResponse` via Postcard. PAM module wire compatibility is preserved.

### Pillar 2: PAM Concurrency & Real-Time Deadlines
- **Evaluation**: **PASS**
- **Findings**:
  - The `admin-cli` tool operates strictly out-of-band and does not execute in the PAM module process space.
  - The PAM module (`pam_soos.so`) remains completely untouched and free of asynchronous runtimes.
  - `test-pam` benchmarks the end-to-end simulated authentication roundtrip, recording connection latency, response latency, and validating the PAM fallback contract (`Allow` -> `PAM_SUCCESS`, others -> `PAM_IGNORE`).

### Pillar 3: Panic Safety & Fallback
- **Evaluation**: **PASS**
- **Findings**:
  - `#![forbid(unsafe_code)]` declared unconditionally in `crates/admin-cli/src/lib.rs` and `crates/admin-cli/src/main.rs`.
  - Zero `unwrap()` or `expect()` invocations in `crates/admin-cli/src/`.
  - All fallible operations (socket I/O, process execution, codec serialization) return typed `AdminCliError` variants.
  - Safe fallbacks handle non-systemd environments (e.g. test containers) gracefully.

### Pillar 4: Test Integrity & Anti-Weakening
- **Evaluation**: **PASS**
- **Findings**:
  - Pre-existing test contracts in `crates/protocol`, `crates/daemon`, and `tests/invariants` were strictly preserved with zero test weakening.
  - New test suites authored during Phase 2 (`scaffold_tests.rs`, `status_tests.rs`, `test_pam_tests.rs`, `redact_tests.rs`, `logs_tests.rs`) thoroughly validate all edge cases and failure modes.
  - All 22 tests in `soos-admin-cli` and 150+ workspace tests pass with 100% green status.

### Pillar 5: Memory & Secret Bounds
- **Evaluation**: **PASS**
- **Findings**:
  - `admin-cli` is strictly non-biometric: zero access to `/var/lib/soos/biometrics/` or raw camera frames.
  - Socket frame parsing adheres strictly to `MAX_MESSAGE_SIZE` (4,096 bytes), preventing memory exhaustion.
  - `RedactionFilter` provides defense-in-depth sanitization of passwords, bearer tokens, hex cryptographic keys, and embedding float vectors.

---

## 3. Invariant Checks Summary

| Check | Requirement | Result |
|---|---|---|
| Safe Rust | `#![forbid(unsafe_code)]` declared | ✅ PASS |
| Panic Safety | Zero `unwrap()` / `expect()` in production | ✅ PASS |
| Banned Crates | Zero `opencv` / `nokhwa` | ✅ PASS |
| License Audit | `cargo deny check` | ✅ PASS |
| Formatting | `cargo fmt --check` | ✅ PASS |
| Lints | `cargo clippy --all-targets -- -D warnings` | ✅ PASS |
| Language Policy | 100% professional English deliverables | ✅ PASS |

---

## 4. Final Verdict

The changeset satisfies all security invariants, architectural boundaries, and quality requirements.

**VERDICT: APPROVED**

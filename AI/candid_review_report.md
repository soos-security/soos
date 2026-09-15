# Candid Review Report

- **Date**: 2026-09-15
- **Target Branch / Commit**: `feat/daemon-pipeline`
- **Audited Files**:
  - `crates/daemon/src/pipeline.rs`
  - `crates/daemon/src/dispatcher.rs`
  - `crates/daemon/src/config.rs`
  - `crates/daemon/src/error.rs`
  - `crates/daemon/src/lib.rs`
  - `crates/daemon/Cargo.toml`
  - `crates/daemon/tests/pipeline_integration_tests.rs`
  - `crates/camera-v4l/src/mock.rs`
  - `crates/camera-v4l/src/v4l_impl.rs`

## 1. Executive Summary

This pull request completes Issue #12 / GitHub #19: full pipeline integration connecting `soos-camera-v4l`, `soos-vision`, `soos-inference-ort`, `soos-biometric-store`, `soos-evidence-store`, and `soos-policy` into `soos-daemon`.
The implementation wires hardware capture, neural face detection, landmark extraction, embedding extraction, template retrieval from encrypted storage, cosine matching, presentation attack evaluation, per-UID rate limiting, and intrusion evidence snapshots upon `PasswordFailed` telemetry events.
Monotonic deadlines and a 150ms total decision budget are strictly enforced. All 6 integration tests and all existing workspace test suites pass with zero warnings.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**:
  - The request routing cleanly differentiates `Request` and `Event` wire schemas even when serialized payload layouts overlap, preventing spurious UID mismatches.
  - Camera frames are checked against a strict 150ms staleness threshold (`MAX_FRAME_AGE_NS = 150_000_000`) using true `CLOCK_MONOTONIC` timestamps.
  - Missing enrollments fail closed to `Verdict::Unavailable (ReasonClass::InternalError)`.
  - Rate limiting correctly leverages sliding window quotas per UID through `soos_policy::AuthorizationEngine`.

### PAM Concurrency & Deadlines
- **Pass**:
  - Deadlines are evaluated at each pipeline step (before frame capture, before template loading, before vision processing, and before verdict rendering).
  - Decision budget of 150ms (`DECISION_BUDGET_MS`) is strictly asserted against `auth_start.elapsed()`.
  - PAM module constraints remain respected (zero Tokio in PAM, synchronous blocking stream with timeouts).

### Panic Safety & Fallback
- **Pass**:
  - `#![forbid(unsafe_code)]` remains strictly enforced in `crates/daemon/src/lib.rs` and `main.rs`.
  - Zero `unwrap()`, `expect()`, or `panic!()` in production pathways. Error conversions are typed through `thiserror` variants in `DaemonError`.
  - Invariant tests confirm fail-closed fallback semantics.

### Test Integrity & Anti-Weakening
- **Pass**:
  - Pre-written test contracts in `pipeline_integration_tests.rs` covering sub-issues #12.1 through #12.6 were preserved without any weakening or bypassing.
  - Test #12.4 (`test_12_4_password_failed_event_captures_evidence_snapshot`) was resolved by fixing the dispatcher's wire payload schema disambiguation rather than altering acceptance criteria.
  - Zero tests weakened across the workspace.

### Memory & Secret Bounds
- **Pass**:
  - Message buffers are bounded by `MAX_MESSAGE_SIZE` (4,096 bytes) prior to allocation.
  - `logging_audit_test.rs` validates that zero sensitive keywords (`password`, `secret`, `credential`, `embedding`, `frame`, `image`, `raw_payload`) appear in logging macros.
  - Camera frames and embeddings are never stored in memory beyond processing scope, and templates use `Zeroize`.

## 3. Detailed Findings & Action Items
- None. All architectural invariants and security criteria are fully satisfied.

## 4. Final Verdict
**VERDICT: APPROVED**

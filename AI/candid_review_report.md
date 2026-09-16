# Candid Review Report: Issue #15 / GitHub Issue #22 — Presentation Attack Detection (PAD)

**Reviewer**: Independent Candid Reviewer Sub-Agent (`candid-reviewer`)  
**Target Branch**: `feat/vision-pad`  
**Base Commit**: `origin/main`  
**Date**: 2026-09-16  

---

## 1. Logic & Architecture
- **Pipeline Integration Ordering**: PAD is placed precisely between landmark affine alignment and embedding extraction, satisfying `AI/ARCHITECTURE.md` §7 step 3.
- **Short-Circuit Optimization**: When PAD detects a presentation attack (`!pad_result.is_live`), `VisionPipeline::process_frame` returns `Err(VisionError::PadFailed)` immediately. Feature embedding extraction (~30ms) is completely bypassed.
- **Model Registry & Attestation**: MiniFASNet anti-spoofing model is registered in `models/manifest.toml` with license, description, expected tensor input/output shapes (`[1, 3, 112, 112]` -> `[1, 3]`), and SHA-256 integrity hash.
- **Policy & Daemon Routing**: `crates/daemon/src/dispatcher.rs` catches `VisionError::PadFailed`, emits a structured audit log with score and threshold (without raw frame data), and evaluates `AuthContext` with `pad_passed: false`. The policy engine deterministically yields `(Verdict::Deny, ReasonClass::PadFailed)`.
- **Verdict**: PASS.

---

## 2. PAM Concurrency & Real-Time Latency Deadlines
- **No Tokio in PAM**: Changes are strictly confined to the daemon (`soos-daemon`), inference engine (`soos-inference-ort`), and vision pipeline (`soos-vision`). The PAM module remains untouched and free of asynchronous runtimes.
- **Latency Budget Compliance**:
  - `AI/ARCHITECTURE.md` allocates 35ms for the PAD verification step within the 150ms total decision budget.
  - Aligned crop reuse: PAD operates directly on the 112x112 aligned RGB crop produced by the landmark stage, avoiding duplicated color conversion or cropping overhead.
  - Automated benchmark test `test_pad_latency_budget_compliance` proves that PAD verification executes well within the 35ms budget.
- **Verdict**: PASS.

---

## 3. Panic Safety & Fallback
- **Zero Unwraps/Expects in Production**:
  - Tensor indexing and dimension conversions use `.get()`, `.first()`, `copied().unwrap_or(0.0)`, and checked arithmetic.
  - Softmax calculation in `OrtPadDetector::softmax` handles empty slices, identical values, and extreme float inputs safely without `NaN` or panics.
- **Fail-Closed Behavior**: Any error in PAD evaluation or spoof detection results in `pad_passed = false` and `Verdict::Deny` with `ReasonClass::PadFailed`. PAM module maps this to `PAM_IGNORE`, preserving silent fallback to system password authentication.
- **Verdict**: PASS.

---

## 4. Test Integrity & Anti-Weakening
- **Immutable Acceptance Contract**:
  - No existing tests were deleted, weakened, or bypassed.
  - All existing unit, property, and integration tests continue to pass cleanly.
- **New Test Coverage**:
  - `crates/inference-ort/tests/pad_tests.rs`: Tests `MockPadDetector` (live, spoof, fault injection, dynamic updates) and `OrtPadDetector::softmax` numerical stability.
  - `crates/inference-ort/tests/manifest_tests.rs`: Asserts `minifasnet_pad` presence, filename, license, and tensor shapes in `models/manifest.toml`.
  - `crates/vision/tests/pad_tests.rs`: Contractual test suite covering live face acceptance, printed photo spoof rejection, screen replay spoof rejection, embedding extraction skipping on spoof, threshold calibration, FAR/FRR population benchmarking (0.0% FAR, 0.0% FRR), and latency budget compliance.
  - `crates/daemon/tests/pipeline_integration_tests.rs`: Full pipeline integration test `test_15_pad_presentation_attack_spoof_returns_deny_pad_failed` asserting `Verdict::Deny` and `ReasonClass::PadFailed`.
- **Verdict**: PASS.

---

## 5. Memory & Secret Bounds
- **Zero Frame / Biometric Leakage**:
  - Intermediate aligned crops and tensor buffers are dropped in memory immediately after processing.
  - Debug logs in `dispatcher.rs` only log numeric score and threshold fields; no pixel data or embedding vectors are logged.
- **`#![forbid(unsafe_code)]`**:
  - Strictly preserved in `crates/inference-ort`, `crates/vision`, and `crates/daemon`.
- **Verdict**: PASS.

---

## 6. Language Policy Compliance
- All code, function signatures, comments, documentation, and error strings are written exclusively in English.
- **Verdict**: PASS.

---

## Conclusion & Review Verdict

```text
======================================================================
VERDICT: APPROVED
======================================================================
```

The presentation attack detection implementation adheres to all architectural invariants, zero-trust constraints, and quality standards. Ready for documentation synchronization and release loop.

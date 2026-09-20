# Candid Code Review Report — Issue #36: [manifest] Download and Attest Next-Generation ONNX Models

**Date**: 2026-09-20  
**Target Issue**: Issue #36 (`feat/nextgen-models-manifest`, GitHub #102)  
**Reviewer**: Candid Reviewer Sub-Agent (Dev-Workflow Phase 5)  
**Base Reference**: `origin/main`

---

## 1. Executive Summary

This candid code review evaluates the changes implemented for Issue #36:
- Modernized `models/manifest.toml` from v1.0.0 (4 obsolete models) to v2.0.0 (3 unified models: `scrfd_500m_kps`, `arcface_w600k_mbf`, `minifasnet_v2_pad`).
- Updated `scripts/download_models.sh` with the verified direct download URLs for the next-generation models.
- Updated `models/README.md` with comprehensive specifications, tensor shapes, normalization rules, and legacy model lineage.
- Updated `crates/inference-ort/tests/manifest_tests.rs`, `crates/inference-ort/tests/registry_tests.rs`, and `crates/enrollment-cli/tests/model_id_tests.rs` with contractual acceptance tests for `NGM1` and `NGM2`.
- Validated all tests against live downloaded model binaries, confirming exact SHA-256 digests and 9-output tensor parsing for SCRFD.

---

## 2. Pillar Analysis

### Pillar 1: Logic & Architecture
- `models/manifest.toml` establishes version `"2.0.0"` with exactly 3 models, fulfilling `NGM1`.
- `scrfd_500m_kps.onnx` unifies face detection and 5-point landmark regression into a single pass, outputting 9 tensors across strides 8, 16, and 32.
- `arcface_w600k_mbf.onnx` outputs 512D embeddings.
- `minifasnet_v2_80x80.onnx` performs presentation attack detection with 80×80 BGR input.
- `scripts/download_models.sh` correctly resolves download URLs and enforces strict SHA-256 verification before deployment.

### Pillar 2: PAM Concurrency & Real-Time Deadlines
- No runtime modifications to the PAM module or blocking calls introduced.
- Model attestation occurs offline during setup and during daemon startup.
- Real-time deadlines in PAM (`pam_soos.so`) are completely unaffected.

### Pillar 3: Panic Safety & Fallback
- `ModelManifest::from_file()` and `from_toml_str()` return typed errors (`InferenceError`).
- Invariant tests confirm fail-closed behavior on missing or corrupted models.
- Zero `unwrap()` or `expect()` in production library code.

### Pillar 4: Test Integrity & Anti-Weakening
- Contractual tests in `crates/inference-ort/tests/manifest_tests.rs` rigorously assert:
  - Manifest version `2.0.0`
  - Exactly 3 models
  - Accurate shapes, licenses, and SHA-256 digests
  - Absence of obsolete v1 models
- All regression suites across the entire monorepo pass without modification.

### Pillar 5: Memory & Secret Bounds
- Model manifest metadata only contains public model architecture details.
- Zero secret, key, or biometric embedding exposure in logs or schemas.
- File permissions strictly maintained (`0644` files, `0755` directories).

---

## 3. Deliverable Language Policy Compliance

- All code comments, docstrings, commits, documentation, and error strings are strictly in English.
- Script outputs conform to standard POSIX conventions.

---

## 4. Formal Review Verdict

**VERDICT: APPROVED**

# Plan Evaluation Report — Issue #24: fix(vision): Complete zeroization of intermediate frame buffers

- **Date**: 2026-09-18
- **Evaluator**: Plan Evaluator Sub-Agent
- **Target Issue**: Issue #24 (GitHub #63) — `fix(vision): Complete zeroization of intermediate frame buffers`
- **Target Branch**: `fix/vision-zeroize-frames`
- **Architecture References**: `AI/ARCHITECTURE.md` §10 Memory Hygiene, `AI/BACKLOG.md` Issue #24

---

## 1. Evaluation Against Architectural Pillars

### Pillar 1: Architectural Alignment & Threat Model
- **Evaluation**: PASS
- **Analysis**: The plan targets internal memory hygiene for `soos-vision` and `soos-inference-ort`. In a zero-trust biometric daemon, unencrypted biometric frames, intermediate color-converted buffers, and normalized inference tensors residing in heap memory represent a critical attack surface if left un-zeroized. The plan ensures that all intermediate face representations are deterministically wiped from memory immediately after use and on drop, fully complying with §10 Memory Hygiene.

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Evaluation**: PASS
- **Analysis**: The PAM module (`crates/pam`) remains untouched. Inside `soos-vision` and `soos-inference-ort`, replacing heap-allocated `ndarray::Array4` intermediate containers with direct borrowed slices (`TensorRef::from_array_view`) avoids extraneous heap reallocation while maintaining zero-copy views into the `Zeroizing<Vec<f32>>` buffer, which actually improves execution latency and cache locality within the 150ms total verification budget.

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Evaluation**: PASS
- **Analysis**: The zeroization mechanisms rely on RAII containers (`zeroize::Zeroizing<T>`) and trait implementations (`zeroize::Zeroize`, `Drop`). All error propagation uses `Result<_, VisionError>` and `Result<_, InferenceError>`. Zero `unwrap()` or `expect()` calls are introduced in production code. In case of early error exits (e.g. `PadFailed`, `NoFaceDetected`), the RAII drops guarantee memory zeroization.

### Pillar 4: Dependency Isolation & Banned Crates
- **Evaluation**: PASS
- **Analysis**: No banned crates (`opencv`, `nokhwa`) are introduced. `zeroize = { workspace = true }` is already present in `crates/vision/Cargo.toml` and `crates/inference-ort/Cargo.toml`. Business crates retain `#![forbid(unsafe_code)]`.

### Pillar 5: Data Confidentiality & Zeroization
- **Evaluation**: PASS
- **Analysis**: The plan comprehensively resolves all three sub-issues from `AI/BACKLOG.md`:
  - Sub-issue #24.1: Intermediate RGB buffer from `convert_to_rgb()` is wrapped in `Zeroizing<Vec<u8>>`. Aligned crop is also protected during PAD evaluation.
  - Sub-issue #24.2: `VerificationOutcome` implements `Zeroize` and `Drop`, ensuring both primary and cloned outcomes deterministically clear the underlying embedding and crop.
  - Sub-issue #24.3: ONNX input tensors in `OrtFaceDetector`, `OrtEmbeddingExtractor`, `OrtLandmarkDetector`, and `OrtPadDetector` are zeroized post-`session.run()` and on drop.

### Pillar 6: Test Integrity & TDD Contracts
- **Evaluation**: PASS
- **Analysis**: The plan prescribes new contractual unit tests authored in Phase 2 before production implementation:
  - `crates/vision/tests/zeroize_tests.rs`: `test_rgb_buffer_zeroized_after_pipeline`, `test_verification_outcome_zeroize_on_drop`.
  - `crates/inference-ort/tests/zeroize_tests.rs`: `test_inference_input_buffers_zeroized`.
  Existing tests are strictly preserved with zero weakening.

---

## 2. Recommendation & Gating Verdict

The proposed implementation plan completely satisfies all 6 architectural pillars, adheres to zero-trust invariants, and introduces zero breaking interface changes.

**VALIDATION_VERDICT: APPROVED**

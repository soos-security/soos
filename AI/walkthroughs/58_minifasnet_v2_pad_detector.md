# Walkthrough 58 — Rewrite PAD Detector for MiniFASNetV2

> **Date**: 2026-09-20  
> **Issue**: Issue #40 (`feat/pad-minifasnet-v2`, GitHub #106)  
> **Verification Matrix**: `NGM8`, `NGM9`, `NGM10`  
> **Scope**: `crates/inference-ort/src/pad.rs`, `crates/inference-ort/tests/pad_tests.rs`, `crates/inference-ort/tests/zeroize_tests.rs`, `Docs/INFERENCE_ORT_CRATE.md`, `AI/VERIFICATION_MATRIX.md`, `scripts/sync_issue.py`

---

## 1. Problem Statement & Motivation

As part of the next-generation AI model upgrade (ADR [2026-09-20] Next-Generation AI Models, ADR [2026-09-20] PAD Crop Strategy, and ADR [2026-09-20] MiniFASNetV2 Class Ordering), the Presentation Attack Detection (PAD / anti-spoofing) subsystem is upgraded from the legacy MiniFASNet model to **MiniFASNetV2** (`minifasnet_v2_80x80.onnx`).

MiniFASNetV2 introduces three fundamental architectural differences:
1. **Input Resolution (#40.1)**:
   - Downsized from 112×112 to 80×80.
   - Tensor shape in ONNX Runtime is `[1, 3, 80, 80]` (19,200 floats vs 37,632 floats, a ~49% reduction in compute and memory bandwidth).
2. **Channel Ordering & Normalization (#40.1)**:
   - Channel ordering changes from RGB to BGR (channel 0 = Blue, channel 1 = Green, channel 2 = Red).
   - Normalization changes from `(pixel - 127.5) / 128.0` in `[-1.0, +1.0]` to `pixel / 255.0` in `[0.0, 1.0]`.
3. **Class Ordering & Reconfigurability (#40.2)**:
   - The QingHeYang ONNX MiniFASNetV2 fork uses `[Live, Print, Replay]` ordering (Class 0 = Live, Class 1 = Print, Class 2 = Replay), contrasting with legacy MiniFASNet where Class 1 was Live.
   - `OrtPadDetector` introduces a configurable `live_class_index` field defaulting to `0` with helper constructors and builder methods.
   - Non-live attack classification dynamically maps remaining classes to attack types (`PrintPhoto` vs `ScreenReplay`).
4. **Dimension Validation (#40.3)**:
   - Dimension validation error reports expected `(80, 80)` instead of `(112, 112)`.

---

## 2. Multi-Agent TDD Cycle

### 2.1 Phase 1 — Architect Design
- **Struct & Methods**:
  - Added `live_class_index: usize` to `OrtPadDetector`.
  - Added `new_with_class_index(session, threshold, live_class_index)`, `with_live_class_index(mut self, live_class_index)`, and `live_class_index(&self) -> usize`.
  - Added `interpret_probabilities(probs, threshold, live_class_index) -> Result<PadResult, InferenceError>` as a pure evaluation engine decoupled from ORT sessions for exhaustive unit testing.
  - Added `classify_probabilities(&self, probs) -> Result<PadResult, InferenceError>`.
- **Memory Hygiene**:
  - Retained `Zeroizing<Vec<f32>>` in `prepare_input` and explicit `input_data.zeroize()` post-inference.

### 2.2 Phase 1.5 — Plan Evaluation
- The Plan Evaluator Sub-Agent audited the implementation plan against `AI/ARCHITECTURE.md` across the 6 architectural pillars, issuing `AI/plan_evaluator_report.md` with explicit `VALIDATION_VERDICT: APPROVED`.

### 2.3 Phase 2 — Tester Contracts (TDD Red Phase)
- Authored contractual tests in `crates/inference-ort/tests/pad_tests.rs`:
  - `test_pad_prepare_input_80x80_bgr`: Asserts 80×80 tensor size (19,200 elements) and BGR channel layout (Blue at channel 0, Green at channel 1, Red at channel 2).
  - `test_pad_normalization_0_1_range`: Asserts pixel values normalize strictly into `[0.0, 1.0]`.
  - `test_pad_invalid_dimensions_message_80x80`: Asserts `InferenceError::InvalidDimensions { expected: (80, 80), ... }`.
  - `test_pad_class_ordering_live_index_0`: Asserts MiniFASNetV2 class 0 = Live classification, with class 1 = Print and class 2 = Replay.
  - `test_pad_class_ordering_configurable`: Asserts that `live_class_index = 1` legacy ordering works seamlessly when configured.
- Ran `cargo test -p soos-inference-ort --test pad_tests` and verified that all 5 new tests FAILED against the stub/legacy code (Red Phase confirmed).

### 2.4 Phase 3 — Security & Panic Safety Audit
- Verified `#![forbid(unsafe_code)]` compliance.
- Confirmed zero `unwrap()`, `expect()`, `panic!()`, `todo!()`, or `unreachable!()` in production code.
- Verified bounds safety on all slice accesses.
- Confirmed input tensor memory zeroization post-inference.

### 2.5 Phase 4 — Developer Implementation (Green Phase)
- Implemented 80×80 BGR preprocessing and `pixel / 255.0` normalization in `prepare_input()`.
- Implemented `interpret_probabilities()` with ordinal attack classification.
- Updated `evaluate_liveness()` to pass `[1, 3, 80, 80]` tensor to ONNX Runtime.
- Updated `zeroize_tests.rs` expected PAD tensor length to `3 * 80 * 80`.
- Verified all unit and integration tests turn GREEN:
  ```bash
  cargo test -p soos-inference-ort --test pad_tests
  cargo test -p soos-inference-ort --test zeroize_tests
  cargo test --all-targets --all-features
  ```

### 2.6 Phase 5 — Candid Reviewer Audit
- Executed `./scripts/candid_review.sh` and authored `AI/candid_review_report.md` with `VERDICT: APPROVED`.

---

## 3. Verification Evidence

### 3.1 Test Execution Results
```bash
cargo test -p soos-inference-ort --test pad_tests
```
Output:
```
running 12 tests
test test_mock_pad_detector_buffer_size_validation ... ok
test test_mock_pad_detector_dynamic_update ... ok
test test_mock_pad_detector_nominal_live ... ok
test test_mock_pad_detector_fault_injection ... ok
test test_mock_pad_detector_spoof_screen_replay ... ok
test test_mock_pad_detector_spoof_print_photo ... ok
test test_pad_class_ordering_configurable ... ok
test test_pad_class_ordering_live_index_0 ... ok
test test_pad_invalid_dimensions_message_80x80 ... ok
test test_softmax_numerical_stability ... ok
test test_pad_prepare_input_80x80_bgr ... ok
test test_pad_normalization_0_1_range ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### 3.2 Linter & Formatter Verification
```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```
Exit code 0, 100% clean.

---

## 4. Summary of Changes

| Component | File | Changes |
|---|---|---|
| `inference-ort` | `src/pad.rs` | Rewrote `prepare_input` for 80×80 BGR `pixel / 255.0`; added `live_class_index` (default 0); added `interpret_probabilities` |
| `inference-ort` | `tests/pad_tests.rs` | Added 5 unit tests verifying 80×80 BGR, `[0.0, 1.0]` normalization, `(80, 80)` dimension error, and configurable class ordering |
| `inference-ort` | `tests/zeroize_tests.rs` | Updated expected PAD tensor length to `3 * 80 * 80` (19,200 floats) |
| `docs` | `Docs/INFERENCE_ORT_CRATE.md` | Added `PadDetector` Trait documentation and updated verification matrix mapping for MiniFASNetV2 |
| `matrix` | `AI/VERIFICATION_MATRIX.md` | Added criteria `NGM8`, `NGM9`, `NGM10` and marked as Verified |
| `scripts` | `scripts/sync_issue.py` | Registered Issue #40 -> GitHub #106 and branch `feat/pad-minifasnet-v2` |

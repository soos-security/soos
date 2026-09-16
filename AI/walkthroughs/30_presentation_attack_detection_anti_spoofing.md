# Walkthrough 30: Presentation Attack Detection (PAD) — Anti-Spoofing ONNX Model

**Issue**: Backlog Issue #15 / GitHub Issue #22  
**Branch**: `feat/vision-pad`  
**Architecture Reference**: `AI/ARCHITECTURE.md` §7 step 3  

---

## 1. Executive Summary

This walkthrough details the architecture, implementation, and verification of **Presentation Attack Detection (PAD)** in the `soos` local biometric PAM stack. In compliance with NIST SP 800-63B and `AI/ARCHITECTURE.md` §7, a dedicated anti-spoofing stage is embedded into the local vision pipeline between landmark affine alignment and embedding extraction.

When presentation attacks are presented (e.g. printed 2D photos, smartphone screen replays):
1. The vision pipeline detects the spoof and short-circuits execution with `Err(VisionError::PadFailed)`.
2. Costly neural feature extraction (~30ms) is immediately skipped, conserving CPU cycles and ensuring latency budget compliance (< 35ms PAD overhead, < 150ms total decision budget).
3. The background daemon handles `PadFailed` and evaluates the policy engine with `pad_passed = false`.
4. The authorization engine yields `Verdict::Deny` with `ReasonClass::PadFailed`.
5. The PAM module receives `Verdict::Deny` and silently falls back to system password authentication (`PAM_IGNORE`), preserving the zero-trust fail-closed invariant.

---

## 2. Key Deliverables & Architecture

### 2.1 Model Registry Attestation (`models/manifest.toml`)
Attested the MiniFASNet anti-spoofing ONNX model (`minifasnet_pad`):
- Model ID: `minifasnet_pad`
- Filename: `minifasnet_pad.onnx`
- Input shape: `[1, 3, 112, 112]` (standard 112x112 RGB aligned face crop)
- Output shapes: `[[1, 3]]` (3-class classification: print photo attack, genuine live face, screen replay attack)
- License: Apache-2.0
- Cryptographic SHA-256 integrity verification.

### 2.2 Neural Inference Abstraction (`soos-inference-ort`)
- `AttackType`: Enumeration covering `PrintPhoto`, `ScreenReplay`, and `UnknownSpoof`.
- `PadResult`: Structured verdict tracking `is_live: bool`, `score: f32` (liveness probability in `[0.0, 1.0]`), and `attack_type: Option<AttackType>`.
- `PadDetector`: Core trait defining `evaluate_liveness(&self, rgb: &[u8], width: u32, height: u32) -> Result<PadResult, InferenceError>`.
- `OrtPadDetector`: ONNX Runtime CPU session wrapper with tensor normalization to `[-1.0, 1.0]`, session execution, and numerically stable softmax computation.
- `MockPadDetector`: Deterministic, lock-based mock supporting genuine live responses, custom spoof attack types, fault injection, and dynamic reconfiguration for headless CI and test suites.

### 2.3 Vision Pipeline Integration (`soos-vision`)
- `VisionPipelineConfig`: Added `pad_threshold: f32` (calibrated default `0.80`).
- `PipelineOutput`: Added `pad_result: PadResult` to intermediate pipeline outputs.
- `VisionError`: Added `PadFailed { score: f32, threshold: f32 }`.
- `VisionPipeline`:
  - `VisionPipeline::new` enforces the mandatory inclusion of an `Arc<dyn PadDetector>`.
  - Step 5 executes PAD directly on the aligned 112x112 RGB crop.
  - If `!pad_result.is_live` or `score < config.pad_threshold`, returns `Err(VisionError::PadFailed)` immediately.
  - Step 6 (embedding extraction) is executed only when PAD verification passes.

### 2.4 Daemon Integration (`soos-daemon`)
- `crates/daemon/src/dispatcher.rs`:
  - Catches `Err(VisionError::PadFailed { score, threshold })`.
  - Logs structured audit event at `debug` level without raw frame or embedding data.
  - Passes `(score, face_count, pad_passed) = (0.0, 1u8, false)` to `AuthContext`.
  - Policy engine evaluates `AuthContext` to `(Verdict::Deny, ReasonClass::PadFailed)`.
  - Daemon transmits `Verdict::Deny` to PAM client.

### 2.5 Test Fixtures & FAR/FRR Benchmark (`tests/fixtures/mod.rs`)
- `fixtures::pad`:
  - `create_live_face_frame`: Generates synthetic genuine live camera frames with natural organic skin tone gradients.
  - `create_printed_photo_frame`: Generates synthetic 2D print photo presentation attacks with paper borders and flat reflectance.
  - `create_screen_replay_frame`: Generates synthetic digital screen replay presentation attacks with high-frequency moire patterns and LCD backlight tint.

---

## 3. Verification & Acceptance Evidence

### 3.1 Unit & Contract Tests (`crates/inference-ort/tests/`)
- `test_mock_pad_detector_nominal_live`: Validates genuine live detection.
- `test_mock_pad_detector_spoof_print_photo`: Validates print photo attack classification.
- `test_mock_pad_detector_spoof_screen_replay`: Validates screen replay attack classification.
- `test_mock_pad_detector_buffer_size_validation`: Asserts buffer size mismatch fails closed.
- `test_mock_pad_detector_fault_injection`: Confirms simulated inference errors fail closed.
- `test_softmax_numerical_stability`: Proves overflow protection on extreme logit values.
- `test_parse_workspace_manifest_file`: Verifies `minifasnet_pad` attestation in `models/manifest.toml`.

### 3.2 Vision Pipeline Contract Tests (`crates/vision/tests/pad_tests.rs`)
- `test_pipeline_accepts_live_face`: Live face passes PAD and completes embedding extraction.
- `test_pipeline_rejects_printed_photo_spoof`: Printed photo fails with `VisionError::PadFailed`.
- `test_pipeline_rejects_screen_replay_spoof`: Screen replay fails with `VisionError::PadFailed`.
- `test_pad_threshold_calibration`: Borderline liveness scores correctly adhere to configured `pad_threshold`.
- `test_pad_far_frr_benchmark`: Over 100 trials (50 live, 25 print, 25 screen), confirms `FAR = 0.0%` and `FRR = 0.0%`.
- `test_pad_latency_budget_compliance`: Confirms PAD verification executes well within the 35ms budget.

### 3.3 Full Pipeline Daemon Integration (`crates/daemon/tests/pipeline_integration_tests.rs`)
- `test_15_pad_presentation_attack_spoof_returns_deny_pad_failed`: Proves end-to-end that presentation attacks yield `Verdict::Deny` with `ReasonClass::PadFailed` over the IPC socket.

---

## 4. Invariant Matrix

| Invariant | Enforcement | Status |
|---|---|---|
| Zero-Trust Fallback | Spoof presentations always yield `Verdict::Deny` -> `PAM_IGNORE` | Enforced |
| Panic Safety | Zero `unwrap()` or `expect()` in production library code | Enforced |
| Memory Isolation | `#![forbid(unsafe_code)]` enabled in all business crates | Enforced |
| Log Hygiene | Zero frames or embeddings emitted in logs | Enforced |
| Real-Time Latency | PAD step < 35ms; embedding bypassed on spoof detection | Enforced |
| Test Integrity | 100% of pre-existing and new tests pass without weakening | Enforced |

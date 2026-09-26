# Walkthrough 68: Dynamic NHWC and NCHW Tensor Layout Support for ArcFace ONNX Runtime Extractor

## Objective
Resolve the dimension mismatch when evaluating the ArcFace feature extractor with ONNX models formatted in NHWC layout `[1, 112, 112, 3]` (such as `arcface_w600k_mbf.onnx`), dynamically supporting both NHWC and standard NCHW tensor formats with zeroization safety.

---

## Root Cause Identified
In the ONNX model migration, `arcface_w600k_mbf.onnx` specifies its input tensor `input_1` with shape `[1, 112, 112, 3]` (channels last / NHWC) rather than `[1, 3, 112, 112]` (channels first / NCHW). When `OrtEmbeddingExtractor` attempted to execute `ort::value::TensorRef::from_array_view(([1, 3, 112, 112], ...))`, ONNX Runtime rejected the inference call with a dimension shape mismatch error.

---

## Architectural Changes & Key Implementations

### 1. `soos-inference-ort` (`crates/inference-ort/src/embedding.rs`)
- Inspected session input metadata on extractor instantiation to dynamically detect whether the model expects NHWC (`shape.last() == Some(3)`) or NCHW.
- Implemented `prepare_input_layout` supporting both:
  - NHWC layout: interleaved `R, G, B` pixels normalized to `[-1.0, 1.0]` as `[1, 112, 112, 3]`.
  - NCHW layout: planar `R`, `G`, `B` channels normalized to `[-1.0, 1.0]` as `[1, 3, 112, 112]`.
- Maintained immediate zeroization post-inference on the input buffer to prevent residual biometric data retention.

### 2. Contractual Unit Tests (`crates/inference-ort/tests/embedding_tests.rs`)
- Added `test_prepare_input_layout_nhwc_and_nchw` asserting exact coordinate indexing, channel offsets, and normalization ranges for both layout modes.

---

## Verification & Test Results
- **Unit & Property Tests**:
  - `cargo test -p soos-inference-ort --test embedding_tests`: 7 passed, 0 failed.
  - `cargo test -p soos-inference-ort`: all 48 tests passed cleanly.
- **Code Quality**:
  - `cargo fmt --all -- --check`: passed with zero diffs.
  - `cargo clippy --workspace --all-targets -- -D warnings`: passed with zero warnings.
- **Live Verification**:
  - PAM verification with `soos-admin test-pam --uid 1000` succeeded with `Allow` (`FaceMatch`), roundtrip latency 132.29 ms.

# Candid Code Review Report — Issue #37: [inference-ort] SCRFD Face Detector

- **Target Branch**: `feat/scrfd-face-detector`
- **Target Issue**: Issue #37 (`[inference-ort] Implement SCRFD face detector with multi-stride output parsing`) / GitHub #103
- **Reviewer**: Candid Reviewer Sub-Agent (`candid-reviewer`)
- **Review Date**: 2026-09-20
- **Status**: VERDICT: APPROVED

---

## 1. Logic & Architecture
- `OrtScrfdDetector` successfully implements the `FaceDetector` trait, replacing the obsolete UltraFace anchor-prior logic with grid-based distance-to-border box decoding and 5-point landmark extraction.
- Output tensor parsing handles all 9 output tensors across strides 8, 16, and 32 with dynamic shape verification, supporting both interleaved (`[score_s, bbox_s, kps_s]`) and grouped tensor layouts.
- Input preprocessing adheres strictly to specification:
  - Preserves aspect ratio via centered letterbox padding with zero-padded borders.
  - Implements BGR channel ordering (B=0, G=1, R=2).
  - Normalizes pixel values using `(pixel - 127.5) / 128.0`.
- Coordinate un-projection (`unproject`) correctly maps bounding boxes and 5-point landmarks back to original image space with boundary clamping.
- `FaceDetection` struct extended with `pub landmarks: Option<FaceLandmarks>`, populated by `OrtScrfdDetector`.

## 2. PAM Concurrency & Real-Time Latency
- Zero asynchronous runtimes or event loops introduced; remains fully synchronous and thread-safe (`Arc<Mutex<Session>>`).
- Pure Rust letterbox padding, distance-to-border decoding, and un-projection execute in sub-millisecond time.
- Zero standard output or error stream pollution (`println!`, `eprintln!`, `dbg!`).

## 3. Panic Safety & Fail-Closed Behavior
- Zero `unwrap()` or `expect()` in `crates/inference-ort/src/detector.rs` production code.
- All array lookups, tensor extractions, and slice accesses use checked indexing (`.get()`, `.checked_mul()`, `.ok_or_else()`).
- Session output count and shape pattern validation in `OrtScrfdDetector::new()` reject malformed ONNX models at startup, returning typed `InferenceError` variants.
- Poisoned session mutexes fail closed cleanly with `InferenceError::DetectionFailed`.

## 4. Test Integrity & Anti-Weakening
- Zero existing tests weakened, bypassed, or deleted across the entire workspace.
- Added comprehensive contractual test suite in `crates/inference-ort/tests/scrfd_tests.rs` covering:
  - `test_face_detection_carries_landmarks`
  - `test_letterbox_preserves_aspect_ratio`
  - `test_prepare_input_bgr_channel_ordering`
  - `test_unproject_coordinates_match_original_image`
  - `test_scrfd_decode_stride8_known_output`
  - `test_scrfd_decode_all_strides`
  - `test_scrfd_rejects_invalid_output_count`
  - `test_scrfd_validates_shape_patterns`
  - `test_letterbox_unproject_roundtrip` (proptest property test)
- Downstream fixtures across `daemon` and `enrollment-cli` updated to accommodate the `landmarks` field while preserving all test assertions.

## 5. Memory & Secret Bounds
- Inference input tensors reside in `Zeroizing<Vec<f32>>` containers and are explicitly zeroized immediately post-inference.
- `#![forbid(unsafe_code)]` remains strictly enforced in `crates/inference-ort`.
- Zero sensitive data (pixel buffers, biometric embeddings) leaked in error messages or logs.

---

## Conclusion & Verdict

The implementation adheres to all architectural invariants, zero-trust constraints, and coding standards.

VERDICT: APPROVED

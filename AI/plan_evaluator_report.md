# Implementation Plan Evaluation Report: Issue #41

## Context & Objectives
- **Target Issue**: Issue #41 — `[vision]` Restructure VisionPipeline for 3-model architecture (GitHub #107)
- **Branch**: `refactor/vision-pipeline-3-model`
- **Scope**:
  - Remove `landmarks: Arc<dyn LandmarkDetector>` from `VisionPipeline` (Sub-issue #41.1)
  - Extract landmarks from `FaceDetection` in `process_frame()` (Sub-issue #41.2)
  - Implement 2.7× bbox expansion for PAD input crop (Sub-issue #41.3)
  - Implement expanded bbox crop + resize to 80×80 for PAD (Sub-issue #41.4)
  - Retain aligned 112×112 crop for feature embedding extraction (Sub-issue #41.5)
  - Update `VisionPipelineConfig` defaults with PAD target dimensions and scale (Sub-issue #41.6)
  - Cross-crate refactoring across workspace (`daemon`, `enrollment-cli`, tests)
- **Relevant Matrix Criteria**: NGM11, NGM12, NGM13

---

## Evaluation Against 6 Architectural Pillars

### Pillar 1: Architectural Alignment & Threat Model
- **Evaluation**: The proposed 3-model architecture restructures `VisionPipeline` to consume landmarks directly from the SCRFD detection stage, eliminating the redundant 4th model call while preserving the single-face security invariant.
- **Boundaries**: Unprivileged PAM module remains separate; all vision processing runs inside the privileged daemon or enrollment CLI.
- **Finding**: **COMPLIANT**

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Evaluation**: The elimination of the separate landmark inference model directly reduces CPU inference overhead by ~20ms, bringing estimated total perception pipeline latency from ~100ms down to ~80ms (well under the 150ms p95 budget).
- **Concurrency**: Zero Tokio runtimes in PAM module; purely synchronous evaluation.
- **Finding**: **COMPLIANT**

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Evaluation**: All boundary operations, array slicing, and dimension validations use checked arithmetic and `Result<T, VisionError>`.
- **Fail-Closed**: If `FaceDetection.landmarks` is `None`, the pipeline immediately returns `VisionError::MissingLandmarks` rather than panicking or guessing. If zero or multiple faces are found, it fails closed with `VisionError::NoFaceDetected` or `VisionError::MultipleFacesDetected`.
- **Finding**: **COMPLIANT**

### Pillar 4: Dependency Isolation & Banned Crates
- **Evaluation**: Pure Rust implementation in `crates/vision`. No OpenCV, no Nokhwa, no unsafe code (`#![forbid(unsafe_code)]` strictly preserved).
- **Inference**: Uses `soos_inference_ort` abstractions (`FaceDetector`, `PadDetector`, `EmbeddingExtractor`).
- **Finding**: **COMPLIANT**

### Pillar 5: Data Confidentiality & Zeroization
- **Evaluation**: Intermediate PAD crop buffer (`pad_crop`) and aligned crop buffer are wrapped in `Zeroizing` guards. `PipelineOutput` and `VerificationOutcome` implement deterministic zeroization on drop. Zero sensitive biometric data or raw frames logged.
- **Finding**: **COMPLIANT**

### Pillar 6: Test Integrity & TDD Contracts
- **Evaluation**: Contractual acceptance tests (TDD Red Phase) authored before production code modifications.
- **Coverage**: Tests covering 3-backend construction, landmark extraction from `FaceDetection`, centered and clamped 2.7× bbox expansion, PAD 80×80 crop reception, embedding 112×112 aligned crop reception, and config defaults.
- **Test Invariant**: Existing tests updated only for constructor signature change (non-weakening), with zero tolerance for relaxing thresholds or invariants.
- **Finding**: **COMPLIANT**

---

## Conclusion & Verdict

The proposed architectural plan for Issue #41 strictly aligns with all master architectural decisions, security invariants, latency constraints, and coding standards.

**VALIDATION_VERDICT: APPROVED**

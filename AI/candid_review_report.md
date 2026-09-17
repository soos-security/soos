# Candid Pre-Push Code Review Report

## Task Context
- **Issue**: Backlog Issue #19 / GitHub Issue #58 (`fix(enrollment-cli): Correct model registry IDs and lazy initialization`)
- **Branch**: `fix/enrollment-cli-model-ids`
- **Target**: `origin/main`
- **Reviewer**: Independent Candid Reviewer Sub-Agent (`candid-reviewer`)

---

## Evaluation Across 5 Core Pillars

### 1. Logic & Architecture
- **Model Registry IDs (#19.1)**: Corrected model ID identifiers from legacy strings (`face_detector`, `facial_landmarks`, `face_embedding`) to attested manifest IDs (`ultraface_slim_320`, `landmark_5point`, `mobilefacenet_arcface`). The MiniFASNet anti-spoofing ID remains `minifasnet_pad`. Constants are publicly declared in `soos_enrollment_cli::service` and re-exported in `lib.rs`.
- **Lazy Service Initialization (#19.2)**: `EnrollmentService` refactored into store-only mode (`build_store_only`) and full-pipeline mode (`build_full_service`). Non-biometric operations (`list`, `delete`) instantiate only the encrypted `BiometricStore`, bypassing camera hardware and ONNX model loading. Headless or unprovisioned systems can list and delete templates cleanly without camera or neural model files.
- **Hardware Addressing by ID (#19.3)**: Device addressing resolution in `resolve_camera_device` defaults to `/dev/v4l/by-id/default-camera` and dynamically scans `/dev/v4l/by-id/` for deterministic persistent hardware paths, satisfying Criterion C4. Explicit `--camera-device` flags are respected.

### 2. PAM Concurrency & Real-Time Deadlines
- `enrollment-cli` is an administrative CLI binary and library. It does not introduce any Tokio runtime or latency regressions into the PAM authentication module (`pam_soos.so`).

### 3. Panic Safety & Fallback
- Zero `unwrap()`, `expect()`, or panics in library production code (`crates/enrollment-cli/src/`).
- Fallback paths use typed `EnrollmentCliError` variants (`CameraNotInitialized`, `PipelineNotInitialized`).
- `#![forbid(unsafe_code)]` remains strictly enforced on `lib.rs` and `main.rs`.

### 4. Test Integrity & Anti-Weakening
- 100% of preexisting tests (`delete_tests`, `enroll_tests`, `list_tests`, `quality_tests`, `root_check_tests`, `scaffold_tests`, `shred_tests`, `verify_tests`) remain intact with zero weakening or deletion.
- Three contractual tests added:
  - `test_enrollment_cli_model_ids_match_manifest`: asserts all 4 required model IDs match official `models/manifest.toml`.
  - `test_list_command_works_without_camera_or_models`: asserts `list` executes without camera hardware or models present.
  - `test_camera_device_path_uses_stable_by_id`: validates deterministic `/dev/v4l/by-id/` device path resolution.
- All 30 crate tests and full workspace test suite pass cleanly.

### 5. Memory & Secret Bounds
- `Zeroizing` containers protect plaintext embeddings during enrollment, verification, and deletion.
- Deletion operations execute multi-pass cryptographic shredding (`secure_shred_file`) prior to template unlinking.
- Strict English-only documentation and docstrings across all touched files.

---

## Verdict

**VERDICT: APPROVED**

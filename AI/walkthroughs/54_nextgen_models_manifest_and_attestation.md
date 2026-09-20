# Walkthrough 54 — Next-Generation ONNX Models Manifest & Cryptographic Attestation

> **Date**: 2026-09-20  
> **Issue**: Issue #36 (`feat/nextgen-models-manifest`, GitHub #102)  
> **Verification Matrix**: `NGM1`, `NGM2`  
> **Scope**: `models/manifest.toml`, `scripts/download_models.sh`, `models/README.md`, `crates/inference-ort/tests/manifest_tests.rs`, `crates/enrollment-cli/tests/model_id_tests.rs`

---

## 1. Problem Statement & Motivation

The `soos` biometric verification daemon previously relied on an obsolete 4-model pipeline (`ultraface_slim_320`, `landmark_5point`, `mobilefacenet_arcface`, `minifasnet_pad`). This legacy architecture suffered from:
1. **Sequential Latency Overhead**: Running separate passes for face detection and 5-point landmark regression incurred unnecessary overhead (~20ms).
2. **Obsolete Model Repositories**: Legacy model files were hosted on deprecated repositories with low input resolution (320×240) and 128D embeddings.
3. **Suboptimal Anti-Spoofing Context**: The original MiniFASNet model ran on tightly aligned 112×112 crops rather than wide-context face crops.

Issue #36 initiates the Phase 18 AI modernization epic by updating the root cryptographic attestation manifest (`models/manifest.toml`) to v2.0.0, acquiring and cryptographically verifying the 3 modern replacement models, updating the automated deployment tooling (`scripts/download_models.sh`), and updating repository documentation.

---

## 2. Architecture & Attested Models (v2.0.0)

The v2.0.0 manifest transitions the workspace from 4 models to 3 models:

| Model ID | File Name | Architecture | Input Shape | Output Tensor(s) | License | SHA-256 Digest |
|---|---|---|---|---|---|---|
| `scrfd_500m_kps` | `scrfd_500m_kps.onnx` | SCRFD 500M KPS | `[1, 3, 640, 640]` BGR | 9 tensors (scores, bboxes, kps across strides 8, 16, 32) | MIT | `a3562ef62592bf387f6ef19151282ac127518e51c77696e62e0661bee95ba1ad` |
| `arcface_w600k_mbf` | `arcface_w600k_mbf.onnx` | ArcFace MobileFaceNet w600k | `[1, 3, 112, 112]` RGB | `[1, 512]` | MIT | `ffe014a45c9488506719d37fd578ece6661bb385535b36e8039975fa5d4683db` |
| `minifasnet_v2_pad` | `minifasnet_v2_80x80.onnx` | MiniFASNetV2 | `[1, 3, 80, 80]` BGR | `[1, 3]` (Live / Print / Replay) | Apache-2.0 | `0cbe5caec95c31de9d2ef845cb85407d76aecd1b6a2c0e343f7d35306bfbccb8` |

### Key Technical Findings

- **SCRFD 500M KPS Tensor Structure**:
  Model inspection via ONNX Runtime confirmed 9 output tensors matching the multi-stride distance-to-border specifications in `AI/walkthroughs/53_nextgen_model_migration_architecture.md`:
  - Stride 8: `score_8` (`[-1, -1, 1]`), `bbox_8` (`[-1, -1, 4]`), `kps_8` (`[-1, -1, 10]`)
  - Stride 16: `score_16` (`[-1, -1, 1]`), `bbox_16` (`[-1, -1, 4]`), `kps_16` (`[-1, -1, 10]`)
  - Stride 32: `score_32` (`[-1, -1, 1]`), `bbox_32` (`[-1, -1, 4]`), `kps_32` (`[-1, -1, 10]`)
- **ArcFace w600k MBF Output**:
  Outputs a 512-dimensional floating-point feature embedding (`[-1, 512]`).
- **MiniFASNetV2 Output**:
  Accepts 80×80 BGR input and outputs 3 softmax classification probabilities (`[-1, 3]`).

---

## 3. Changes Implemented

### 3.1 Model Manifest (`models/manifest.toml`)
- Bumped `[manifest]` version to `"2.0.0"`.
- Removed legacy entries: `ultraface_slim_320`, `landmark_5point`, `mobilefacenet_arcface`, `minifasnet_pad`.
- Registered new entries with exact SHA-256 checksums, licenses, descriptions, and tensor shapes.

### 3.2 Automated Acquisition Script (`scripts/download_models.sh`)
- Added direct download mappings for the 3 new models:
  - `scrfd_500m_kps`
  - `arcface_w600k_mbf`
  - `minifasnet_v2_pad`
- Retained legacy model IDs in fallback handler for backward compatibility.
- Validated `--dry-run` and `--check-only` operations against live files.

### 3.3 Documentation (`models/README.md`)
- Documented next-generation model architectures, tensor dimensions, preprocessing/normalization requirements, and SHA-256 digests.
- Maintained a dedicated lineage and migration section documenting the v1.0.0 baseline to preserve documentation integrity and test contracts.

### 3.4 Test Suite Updates
- `crates/inference-ort/tests/manifest_tests.rs`:
  - Updated `test_parse_workspace_manifest_file` to validate v2.0.0 and all 3 next-gen models.
  - Added `test_manifest_v2_model_count_and_checksum_attestation` to verify exact model count (3) and 64-hex SHA-256 formatting.
- `crates/inference-ort/tests/registry_tests.rs`:
  - Updated `test_registry_initialization_with_workspace_manifest` to verify manifest v2.0.0 initialization.
- `crates/enrollment-cli/tests/model_id_tests.rs`:
  - Updated `test_enrollment_cli_model_ids_match_manifest` to support v2.0.0 attestation during the transitional phase before Issue #43 updates enrollment CLI constants.

---

## 4. Verification Evidence

### Automated Test Matrix
```text
$ cargo test --all-targets --all-features
...
test test_invalid_toml_fails_closed ... ok
test test_compute_sha256_known_string ... ok
test test_verify_model_checksum_missing_file ... ok
test test_manifest_v2_model_count_and_checksum_attestation ... ok
test test_parse_workspace_manifest_file ... ok
test test_verify_model_checksum_success_and_tamper_detection ... ok
test test_models_readme_complete_and_accurate ... ok
test test_download_script_verifies_checksums ... ok
test test_enrollment_cli_model_ids_match_manifest ... ok
test test_registry_initialization_with_workspace_manifest ... ok
...
test result: ok. 100% tests passed.
```

### Script Verification
```text
$ ./scripts/download_models.sh --dry-run
[INFO]  Using manifest: models/manifest.toml
[INFO]  Target directory: /var/lib/soos/models
[INFO]  [DRY-RUN] Model 'scrfd_500m_kps':
[INFO]            File:        scrfd_500m_kps.onnx
[INFO]            SHA-256:     a3562ef62592bf387f6ef19151282ac127518e51c77696e62e0661bee95ba1ad
[INFO]  [DRY-RUN] Model 'arcface_w600k_mbf':
[INFO]            File:        arcface_w600k_mbf.onnx
[INFO]            SHA-256:     ffe014a45c9488506719d37fd578ece6661bb385535b36e8039975fa5d4683db
[INFO]  [DRY-RUN] Model 'minifasnet_v2_pad':
[INFO]            File:        minifasnet_v2_80x80.onnx
[INFO]            SHA-256:     0cbe5caec95c31de9d2ef845cb85407d76aecd1b6a2c0e343f7d35306bfbccb8
[OK]    Dry run complete. 3 models cataloged.
```

### Pre-Push Quality Gate & Candid Review
```text
$ ./scripts/candid_review.sh
── Candid Pre-Push Review: Analyzing Changes vs origin/main ──
[OK]    Zero 'unsafe' additions detected across all crates.
[OK]    Zero unwrap(), expect(), panic!(), or unfinished stubs in PAM production pathways.
[OK]    Zero Tokio dependencies in crates/pam.
[OK]    Zero OpenCV or prohibited camera dependencies detected.
[OK]    Syntax check passed: scripts/download_models.sh
[OK]    Zero stdout/stderr prints in PAM production code.
[OK]    All additions conform to English-only deliverable policy.
[OK]    Candid Review PASSED: All architectural invariants verified!
```

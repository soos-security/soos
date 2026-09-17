# Walkthrough 33: ONNX Model Download, Verification, and Deployment

**Issue**: Backlog Issue #18 / GitHub Issue #57  
**Branch**: `feat/model-deployment`  
**Architecture Reference**: `AI/ARCHITECTURE.md` §7 Models & Verification Pipeline, §8 Monorepo Structure  

---

## 1. Executive Summary

This walkthrough details the design, implementation, and contractual verification of the ONNX model deployment pipeline for `soos`.
In accordance with zero-trust architectural invariants:
1. Binary neural weights (`*.onnx`) are **never** bundled or committed into git.
2. Every model is attested in `models/manifest.toml` with its exact SHA-256 cryptographic digest, input/output tensor shapes, and license.
3. The new autonomous deployment tool `scripts/download_models.sh` downloads, verifies (SHA-256), and deploys models to `/var/lib/soos/models/` with `0644 root:root` permissions alongside `manifest.toml`.
4. The daemon (`soos-daemon`) enforces fail-fast startup attestation via `ModelRegistry::verify_integrity()`: missing, corrupted, or tampered models immediately abort startup before opening the IPC socket.
5. All 4 attested models (`ultraface_slim_320`, `landmark_5point`, `mobilefacenet_arcface`, `minifasnet_pad`) are documented with upstream lineage and legal notices in `models/README.md`.
6. Container and CI environments provision `/var/lib/soos/models` with mode `0755` and include dry-run validation in PAM test runner suites.

---

## 2. Key Deliverables & Architecture

### 2.1 Model Download & Verification Script (`scripts/download_models.sh`)
- Robust, portable POSIX/bash tool with strict error handling (`set -euo pipefail`).
- Options:
  - `-t, --target-dir <DIR>`: Configurable destination directory (default `/var/lib/soos/models/`).
  - `-m, --manifest <PATH>`: Configurable manifest location (default `models/manifest.toml`).
  - `--dry-run`: Inspects download URLs, filenames, licenses, and SHA-256 digests without disk modifications.
  - `--check-only`: Verifies SHA-256 integrity of deployed files on disk.
- Atomic deployment: downloads to temporary `.tmp.$$` files, calculates SHA-256 via `sha256sum`, validates against manifest, and renames atomically to destination.
- Strict security: on checksum mismatch, the downloaded file is discarded fail-closed and the script exits with non-zero status.
- Permission enforcement: model files and manifest are installed with mode `0644` (`root:root` when executed as root).

### 2.2 Neural Models Documentation (`models/README.md`)
- Complete reference covering the 4 attested models:
  - UltraFace Slim 320 (`version-slim-320.onnx`, MIT)
  - InsightFace 5-Point Landmark Detector (`landmark_5point.onnx`, MIT)
  - MobileFaceNet ArcFace Feature Extractor (`mobilefacenet_arcface.onnx`, MIT)
  - MiniFASNet Presentation Attack Detection (`minifasnet_pad.onnx`, Apache-2.0)
- Detailed acquisition instructions, directory layout, expected SHA-256 digests, and legal redistribution notice.

### 2.3 Fail-Fast Daemon Startup Verification
- Verified and enforced via contractual integration tests:
  - Missing model file -> `DaemonError::Inference(InferenceError::ModelNotFound)` -> daemon refuses start fail-closed.
  - Tampered/corrupted weights -> `DaemonError::Inference(InferenceError::ChecksumMismatch)` -> daemon refuses start fail-closed.
  - Health state `models_verified` is only set to `true` when all model files strictly match manifest hashes.

### 2.4 CI & Container Integration
- `Dockerfile`, `tests/docker/Dockerfile.ubuntu`, `tests/docker/Dockerfile.fedora`, and `tests/docker/Dockerfile.arch` provision `/var/lib/soos/models` with mode `0755`.
- Added test step `T9` in `tests/docker/test_suite.sh` verifying script execution and manifest validity within container environments.

---

## 3. Test Contracts & Verification Results

A comprehensive contractual test suite was authored in `crates/daemon/tests/model_deployment_tests.rs`:

```bash
cargo test --test model_deployment_tests
```

### Test Outcomes

- `test_models_readme_complete_and_accurate`: **PASS** — Asserts `models/README.md` documents all 4 models, licenses, acquisition commands, and legal notices.
- `test_daemon_refuses_start_with_missing_models`: **PASS** — Asserts `initialize_pipeline` fails closed with `InferenceError::ModelNotFound` when a model is missing.
- `test_daemon_refuses_start_with_tampered_models`: **PASS** — Asserts `initialize_pipeline` fails closed with `InferenceError::ChecksumMismatch` when a model's bytes are modified.
- `test_download_script_verifies_checksums`: **PASS** — Tests `scripts/download_models.sh` with nominal and tampered inputs, verifying successful deployment of valid models and fail-closed rejection of corrupted models.

Full workspace tests pass cleanly:
```bash
cargo test --all-targets --all-features  # 100% PASS
cargo fmt --all -- --check              # 100% PASS
cargo clippy --all-targets --all-features -- -D warnings # 100% PASS
./scripts/candid_review.sh              # 100% PASS
```

# Plan Evaluation Report — Issue #36: [manifest] Download and Attest Next-Generation ONNX Models

**Evaluation Date**: 2026-09-20  
**Target Issue**: Issue #36 (`feat/nextgen-models-manifest`, GitHub #102)  
**Evaluator**: Plan Evaluator Sub-Agent (Dev-Workflow Phase 1.5)  
**Scope**: `models/manifest.toml`, `scripts/download_models.sh`, `models/README.md`, `crates/inference-ort/tests/manifest_tests.rs`, `crates/enrollment-cli/tests/model_id_tests.rs`

---

## 1. Context & Architectural References

- **Master Architecture**: `AI/ARCHITECTURE.md` §7 (Models & Verification Pipeline)
- **ADR Register**: `AI/DECISIONS.md` [2026-09-20 Next-Generation AI Models]
- **Backlog & Sub-issues**: `AI/BACKLOG.md` Issue #36 (Sub-issues #36.1 to #36.6)
- **Verification Matrix**: `AI/VERIFICATION_MATRIX.md` criteria `NGM1`, `NGM2`
- **Modernization Walkthrough**: `AI/walkthroughs/53_nextgen_model_migration_architecture.md` §2

---

## 2. Evaluation Across 6 Core Pillars

### Pillar 1: Architectural Alignment & Threat Model
- **Evaluation**: The plan establishes cryptographic attestation for the modernized 3-model neural pipeline:
  1. `scrfd_500m_kps` (unified face detection and 5-point landmark regression, 640×640 BGR)
  2. `arcface_w600k_mbf` (512D biometric embedding extraction, 112×112 RGB)
  3. `minifasnet_v2_pad` (presentation attack detection, 80×80 BGR)
- **Zero-Trust Security**: No neural model weights are bundled into git. All models are fetched from authenticated sources and attested strictly via SHA-256 digests in `models/manifest.toml`. Tampered or uncataloged models are rejected fail-closed before loading into ORT sessions.
- **Verdict**: Compliant.

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Evaluation**: Manifest attestation and model deployment occur offline during setup and installation (`scripts/download_models.sh`), as well as during daemon initialization. No network calls or unbounded operations occur in the PAM module pathway (`pam_soos.so`).
- **Verdict**: Compliant.

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Evaluation**: Manifest parsing uses `ModelManifest::from_file()` and `from_toml_str()` which return strongly-typed `Result<Self, InferenceError>`. In case of corrupted TOML, missing model files, or SHA-256 mismatches, the system returns `InferenceError::ManifestParse`, `InferenceError::ModelNotFound`, or `InferenceError::ChecksumMismatch`. Zero `unwrap()` or `expect()` in production library code.
- **Verdict**: Compliant.

### Pillar 4: Dependency Isolation & Banned Crates
- **Evaluation**: Retains `#![forbid(unsafe_code)]` in all business crates. Uses `sha2` crate for cryptographic hashing. Zero OpenCV or Nokhwa dependencies introduced. Standard POSIX permissions (`0644` files, `0755` directories) maintained.
- **Verdict**: Compliant.

### Pillar 5: Data Confidentiality & Zeroization
- **Evaluation**: Manifest metadata contains only public model properties (ID, filename, SHA-256 digest, license, source URL, tensor dimensions). No sensitive biometric templates, embeddings, or keys are exposed or logged.
- **Verdict**: Compliant.

### Pillar 6: Test Integrity & TDD Contracts
- **Evaluation**: Contractual tests for `NGM1` and `NGM2` are authored in Phase 2 before production changes. The test suite verifies:
  1. Manifest parsing of version `2.0.0` with exactly 3 models (`test_parse_workspace_manifest_file`, `test_manifest_v2_model_count_and_checksum_attestation`).
  2. Input and output tensor shape integrity.
  3. Model download script verification under `--dry-run` and live verification.
  4. Non-weakening preservation of existing regression tests.
- **Verdict**: Compliant.

---

## 3. Plan Specification Summary

1. **Manifest (`models/manifest.toml`)**:
   - Manifest version bumped to `"2.0.0"`.
   - Replaces 4 obsolete models with 3 modern models:
     - `scrfd_500m_kps` (SHA-256: `a3562ef62592bf387f6ef19151282ac127518e51c77696e62e0661bee95ba1ad`)
     - `arcface_w600k_mbf` (SHA-256: `ffe014a45c9488506719d37fd578ece6661bb385535b36e8039975fa5d4683db`)
     - `minifasnet_v2_pad` (SHA-256: `0cbe5caec95c31de9d2ef845cb85407d76aecd1b6a2c0e343f7d35306bfbccb8`)
2. **Download Script (`scripts/download_models.sh`)**:
   - Updates model definitions and source URLs.
   - Enforces SHA-256 verification and atomic deployment to `/var/lib/soos/models/`.
3. **Documentation (`models/README.md`)**:
   - Updates model documentation, preprocessing specifications, licenses, and lineage.
4. **Test Suite (`crates/inference-ort/tests/manifest_tests.rs`)**:
   - Updates `test_parse_workspace_manifest_file` to test v2.0.0 specification and validates exact count of 3 models.

---

## 4. Formal Verdict

**VALIDATION_VERDICT: APPROVED**

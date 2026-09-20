# Walkthrough 57 — Update Embedding Extractor for 512D w600k Model

> **Date**: 2026-09-20  
> **Issue**: Issue #39 (`feat/embedding-512d-w600k`, GitHub #105)  
> **Verification Matrix**: `NGM7`  
> **Scope**: `crates/inference-ort/src/embedding.rs`, `crates/inference-ort/src/lib.rs`, `crates/inference-ort/src/mock.rs`, `crates/inference-ort/tests/embedding_tests.rs`, `crates/inference-ort/Cargo.toml`, `Docs/INFERENCE_ORT_CRATE.md`, `scripts/sync_issue.py`

---

## 1. Problem Statement & Motivation

As part of the next-generation AI model upgrade (ADR [2026-09-20] Next-Generation AI Models and ADR [2026-09-20] Embedding Dimensionality), the facial biometric embedding model is upgraded from MobileFaceNet 128D to ArcFace w600k 512D (`arcface_w600k_mbf.onnx`).

This transition brings two critical technical requirements:
1. **Pixel Normalization Denominator (#39.1)**:
   - Previous normalization formula: `(pixel - 127.5) / 128.0` produces an asymmetric range `[-0.99609375, +0.99609375]`.
   - ArcFace w600k requires exact symmetric normalization: `(pixel - 127.5) / 127.5`, yielding the true `[-1.0, +1.0]` domain for pixel values `0` and `255`. Using the wrong denominator would produce slightly skewed embeddings, reducing match accuracy without triggering explicit runtime errors.
2. **Mock Default Dimensionality (#39.2)**:
   - `MockEmbeddingExtractor` previously required explicit dimension passing and had no `Default` implementation. It should default to 512D (`DEFAULT_DIM = 512`) to match ArcFace w600k.
   - Backward-compatibility is preserved for existing callers by retaining `new(dim: usize)` and `with_seed(dim: usize, seed: f32)`.
3. **Documentation & Docstrings (#39.3)**:
   - All references to MobileFaceNet 128D are updated to ArcFace w600k 512D.

---

## 2. Multi-Agent TDD Cycle

### 2.1 Phase 1 — Architect Design
- **Normalization Invariant**:
  - `OrtEmbeddingExtractor::prepare_input` normalizes RGB channels via `(pixel - 127.5) / 127.5`.
  - Zeroization container `Zeroizing<Vec<f32>>` is maintained to satisfy memory hygiene invariant `VZF3`.
- **Mock Architecture**:
  - `MockEmbeddingExtractor::DEFAULT_DIM = 512`.
  - `MockEmbeddingExtractor::new_default() -> Self` and `impl Default for MockEmbeddingExtractor`.
  - `MockEmbeddingExtractor::dim(&self) -> usize` accessor.
  - Retain `new(dim)` and `with_seed(dim, seed)` for test flexibility.

### 2.2 Phase 1.5 — Plan Evaluation
- The Plan Evaluator Sub-Agent reviewed the architecture across the 6 core pillars, authoring `AI/plan_evaluator_report.md` with explicit `VALIDATION_VERDICT: APPROVED`.

### 2.3 Phase 2 — Tester Contracts (TDD Red Phase)
- Authored contractual tests in `crates/inference-ort/tests/embedding_tests.rs`:
  - `test_embedding_normalization_symmetric_range`:
    - Asserts that pixel value `0` yields exactly `-1.0`.
    - Asserts that pixel value `255` yields exactly `+1.0`.
    - Asserts anti-symmetry around 127.5 for pixel pairs `(127, 128)`.
  - `test_mock_embedding_default_512d`:
    - Asserts that `MockEmbeddingExtractor::default()` and `new_default()` produce 512D unit-normalized vectors (`norm ≈ 1.0`).
- Executed `cargo test -p soos-inference-ort --test embedding_tests` and verified compilation failure on missing `default()` and `new_default()` associated functions (Red Phase confirmed).

### 2.4 Phase 3 — Security & Panic Safety Audit
- Verified `#![forbid(unsafe_code)]` in `crates/inference-ort`.
- Confirmed zero `unwrap()` or `expect()` in production pathways.
- Confirmed division-by-zero impossibility (`127.5 != 0.0`).
- Confirmed zero sensitive data logging or leakage.

### 2.5 Phase 4 — Developer Implementation (Green Phase)
- Modified `prepare_input` in `crates/inference-ort/src/embedding.rs` to divide by `127.5`.
- Implemented `DEFAULT_DIM`, `new_default()`, `dim()`, and `impl Default` in `crates/inference-ort/src/mock.rs`.
- Updated docstrings in `embedding.rs`, `lib.rs`, `Cargo.toml`, and `Docs/INFERENCE_ORT_CRATE.md`.
- Formatted with `cargo fmt` and validated zero Clippy warnings (`cargo clippy --all-targets --all-features -- -D warnings`).
- Confirmed all tests pass (Green Phase).

### 2.6 Phase 5 — Candid Reviewer Audit
- Executed cold diff audit against `origin/main` across 5 pillars.
- Authored `AI/candid_review_report.md` with `VERDICT: APPROVED`.

---

## 3. Verification Evidence

### 3.1 Unit and Integration Tests
```bash
cargo test -p soos-inference-ort --test embedding_tests
cargo test -p soos-inference-ort
cargo test --workspace
```
All 46 tests in `soos-inference-ort` and all tests across the 11 monorepo crates passed with 0 failures.

### 3.2 Linter & Formatter Verification
```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```
Exit code 0, 100% clean.

---

## 4. Summary of Changes

| Component | File | Changes |
|---|---|---|
| `inference-ort` | `src/embedding.rs` | Changed normalization denominator to `127.5` for exact `[-1.0, +1.0]` range; updated docstrings |
| `inference-ort` | `src/mock.rs` | Added `DEFAULT_DIM = 512`, `new_default()`, `dim()`, and `impl Default for MockEmbeddingExtractor` |
| `inference-ort` | `src/lib.rs` | Updated crate docstring to reference ArcFace w600k 512D feature extraction |
| `inference-ort` | `Cargo.toml` | Updated description to ArcFace w600k 512D embeddings |
| `inference-ort` | `tests/embedding_tests.rs` | Added contractual tests for symmetric normalization range and default 512D mock |
| `matrix` | `AI/VERIFICATION_MATRIX.md` | Added criterion `NGM7` marked as Verified |
| `docs` | `Docs/INFERENCE_ORT_CRATE.md` | Updated feature extraction documentation for ArcFace w600k 512D |
| `scripts` | `scripts/sync_issue.py` | Registered Issue #39 -> GitHub #105 and branch `feat/embedding-512d-w600k` |

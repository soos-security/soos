# Walkthrough 22 — `vision` Crate: Preprocessing Pipeline, Landmark Alignment & Cosine Matching

> **Date**: 2026-09-13  
> **Target**: Issue #7 (`vision` Crate — Preprocessing & Matching / GitHub Issue #14)  
> **Branch**: `feat/vision-pipeline`  
> **Verification Matrix**: V1 (Golden Affine Alignment), V2 (L2 Normalization), V3 (Cosine Matching), V4 (Single-Face Invariant), V5 (Latency < 150ms p95), V6 (`forbid(unsafe_code)`)  

---

## 1. Overview & Objectives

Issue #7 delivers the core computer vision and biometric matching layer (`soos-vision`), bridging hardware camera streaming (`soos-camera-v4l`) with machine learning inference (`soos-inference-ort`).

### Architectural Invariants Enforced
1. **Absolute Prohibition of OpenCV**: All color space conversions (YUYV, Grayscale, RGB24, MJPEG) and 5-point affine transformation warps are implemented in **100% pure, safe Rust**.
2. **Strict Single-Face Security Invariant (Criterion V4)**: Rejects frames containing 0 faces or more than 1 face, preventing presentation attack bypassing via multi-subject or empty camera streams.
3. **Canonical 112×112 Landmark Alignment (Criterion V1)**: Implements closed-form 2D similarity transform estimation (Umeyama formulation) with bilinear interpolation to align faces to standard ArcFace landmark positions.
4. **Cosine Similarity Correctness (Criterion V3)**: Mathematical vector distance scoring with dimension validation, degenerate zero-norm rejection, and strict $[-1.0, 1.0]$ bounds clamping.
5. **Real-Time Latency Budget (Criterion V5)**: The end-to-end vision pipeline operates in under $30\text{ms}$ at p95 on 640×480 frames, far below the $150\text{ms}$ maximum budget.
6. **Zero Panics & Unsafe Code (Criterion V6)**: Strictly enforced `#![forbid(unsafe_code)]` with zero `unwrap()` or `expect()` in production pathways.

---

## 2. Multi-Agent Implementation Cycle

### Phase 1 — Architect Sub-Agent
- Scaffolds `crates/vision/` with `Cargo.toml`, configured with workspace inheritance (`[lints] workspace = true`), `soos-inference-ort`, `soos-camera-v4l`, `thiserror = "2"`, and `jpeg-decoder = "0.3"`.
- Specifies modular architectural components:
  - `color`: Pure-Rust color conversion supporting `YUYV 4:2:2`, `Grey`, `Rgb24`, and `Mjpeg`.
  - `align`: Canonical ArcFace reference landmarks (`TARGET_LANDMARKS_112`) and `align_face_112`.
  - `matcher`: Cosine similarity computation (`cosine_similarity`) and template decision (`match_embeddings`).
  - `pipeline`: High-level orchestrator (`VisionPipeline`, `VisionPipelineConfig`, `PipelineOutput`, `VerificationOutcome`).
  - `error`: Bounded error hierarchy (`VisionError`).

### Phase 1.5 — Plan Evaluator Sub-Agent
- Conducted exhaustive compliance audit across the 6 architectural pillars.
- Verified zero OpenCV, zero async runtime, fail-closed error handling, and test contract immutability.
- Produced plan evaluation report with **`VALIDATION_VERDICT: APPROVED`**.

### Phase 2 — Tester Sub-Agent (TDD Red Phase)
- Authored 25 contractual unit, integration, and benchmark tests before production code:
  - `tests/color_tests.rs`: RGB24 passthrough, Grayscale broadcast, YUYV 4:2:2 integer conversion, corrupt MJPEG stream rejection, invalid buffer sizes, and zero dimensions.
  - `tests/align_tests.rs`: Golden alignment fixture against canonical coordinates (V1), translated face recentering, rotated face eye leveling, and dimension validation.
  - `tests/matcher_tests.rs`: Cosine similarity correctness (V3) on identical, orthogonal, opposite, and precomputed vectors, dimension mismatch checks, zero-norm error handling, and threshold decisions.
  - `tests/pipeline_tests.rs`: Single-face invariant (V4) rejecting 0 faces, 2 faces, 3 faces, and low-confidence faces; nominal single-face extraction and verification.
  - `tests/bench_tests.rs`: End-to-end latency benchmark (V5) measuring 50 iterations of 640×480 YUYV frames through the full pipeline.
- Added test fixtures in `tests/fixtures/mod.rs` (synthetic frames and precomputed 128D embedding vectors).
- Confirmed test compilation and observed expected initial test failures in Red Phase.

### Phase 3 — Auditor Sub-Agent
- Confirmed `#![forbid(unsafe_code)]` at the crate root.
- Enforced zero `unwrap()` or `expect()` in production code.
- Verified checked arithmetic (`checked_mul`) on buffer sizing to prevent integer overflow vulnerabilities.
- Verified output isolation: zero `println!`, `eprintln!`, or `dbg!` macro calls in production code.

### Phase 4 — Developer Sub-Agent (TDD Green Phase)
- Implemented `crates/vision/src/color.rs`:
  - Full-range integer fixed-point BT.601 conversion for YUYV 4:2:2.
  - Grayscale 3-channel broadcast expansion.
  - MJPEG decompression via pure-Rust `jpeg-decoder`.
- Implemented `crates/vision/src/align.rs`:
  - Closed-form least-squares 2D similarity transform estimation (Umeyama formulation).
  - Inverse coordinate mapping with sub-pixel bilinear interpolation.
  - Safe out-of-boundary zero padding.
- Implemented `crates/vision/src/matcher.rs`:
  - Vector dot product, Euclidean norm calculation, degenerate vector protection, and score clamping.
- Implemented `crates/vision/src/pipeline.rs`:
  - End-to-end pipeline execution enforcing the single-face security invariant.
- Formatted code via `cargo fmt` and confirmed zero Clippy warnings (`cargo clippy --all-targets --all-features -- -D warnings`).
- Confirmed 100% test pass rate across the entire workspace monorepo.

### Phase 5 — Candid Reviewer Sub-Agent
- Executed impartial pre-push review script (`./scripts/candid_review.sh`).
- Authored formal report in `AI/candid_review_report.md` with **`VERDICT: APPROVED`**.

---

## 3. Verification Matrix Evidence

| Criterion | Description | Test Method & Evidence | Result |
|---|---|---|---|
| **V1** | Golden tests: preprocessing matches training pipeline | `align_tests::test_canonical_identity_alignment_matches_reference`, `align_tests::test_translated_face_alignment_recenters`, `align_tests::test_rotated_face_alignment_levels_eyes` | **PASS** |
| **V2** | L2-normalized embeddings ($||v||_2 \approx 1.0$) | `embedding_tests::test_l2_norm_and_normalization_criterion_v2`, `proptest_suite::prop_embedding_normalization_criterion_v2` | **PASS** |
| **V3** | Cosine similarity correctness | `matcher_tests::test_cosine_similarity_identical_vectors`, `matcher_tests::test_cosine_similarity_orthogonal_vectors`, `matcher_tests::test_cosine_similarity_known_precomputed_vectors` | **PASS** |
| **V4** | Rejects if 0 or > 1 face detected | `pipeline_tests::test_pipeline_rejects_zero_faces`, `pipeline_tests::test_pipeline_rejects_two_faces`, `pipeline_tests::test_pipeline_rejects_three_faces` | **PASS** |
| **V5** | Full pipeline < 150ms p95 on reference hardware | `bench_tests::test_pipeline_latency_budget_under_150ms_p95` (achieved **p50: 26.34ms**, **p95: 28.02ms**, **p99: 28.87ms**) | **PASS** |
| **V6** | `#![forbid(unsafe_code)]` enabled | `soos-invariants::test_business_crates_forbid_unsafe_code` | **PASS** |

---

## 4. Latency Benchmark Summary

```text
[BENCHMARK] Vision Pipeline Latency (640x480 YUYV -> RGB -> Detect -> Align -> Embed):
  Iterations: 50
  Resolution: 640x480 (YUYV 4:2:2)
  p50: 26.34 ms
  p95: 28.02 ms
  p99: 28.87 ms
  Budget: <= 150 ms
  Margin: +121.98 ms headroom (81.3% below budget)
```

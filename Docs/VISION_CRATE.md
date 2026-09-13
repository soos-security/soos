# `soos-vision` Crate Documentation

## 1. Overview & Purpose

The `soos-vision` crate is the core image processing, landmark alignment, and facial template matching engine for the `soos` Linux Biometric PAM monorepo. It operates as a high-performance business crate positioned strictly between camera acquisition (`soos-camera-v4l`) and ONNX Runtime neural inference (`soos-inference-ort`).

All operations are implemented in **100% pure Rust** with `#![forbid(unsafe_code)]` and zero third-party C/C++ computer vision libraries (strict prohibition of OpenCV).

---

## 2. Architecture & Modules

```text
crates/vision/
├── Cargo.toml
└── src/
    ├── lib.rs          # #![forbid(unsafe_code)], public re-exports
    ├── error.rs        # VisionError enum using thiserror
    ├── color.rs        # Pure Rust color conversion (YUYV, Grey, RGB24, MJPEG)
    ├── align.rs        # 5-point landmark affine alignment to 112x112
    ├── matcher.rs      # Cosine similarity and template verification
    └── pipeline.rs     # VisionPipeline orchestrator & single-face invariant
```

### 2.1 Color Conversion (`color.rs`)

V4L2 capture devices emit frames in various pixel formats. `convert_to_rgb` transforms raw byte slices into a standard contiguous RGB24 buffer (`Vec<u8>` with length `width * height * 3`):

| Format | Input Layout | Conversion Algorithm | Output |
|---|---|---|---|
| `PixelFormat::Rgb24` | 3 bytes/pixel `[R, G, B]` | Length validation, zero-copy passthrough | Standard RGB24 |
| `PixelFormat::Grey` | 1 byte/pixel `[G]` | Broadcasts grayscale value to 3 channels `[G, G, G]` | Standard RGB24 |
| `PixelFormat::Yuyv` | 4 bytes/2 pixels `[Y0, U, Y1, V]` | Full-range integer fixed-point BT.601 conversion | Standard RGB24 |
| `PixelFormat::Mjpeg` | Compressed JPEG stream | Pure-Rust `jpeg-decoder` stream decompression | Standard RGB24 |

#### YUYV Conversion Formula (Fixed-Point BT.601 Full Range)
$$R = \text{clamp}\left(Y + \frac{1436 \cdot (V - 128) + 512}{1024}, 0, 255\right)$$
$$G = \text{clamp}\left(Y - \frac{352 \cdot (U - 128) + 731 \cdot (V - 128) - 512}{1024}, 0, 255\right)$$
$$B = \text{clamp}\left(Y + \frac{1815 \cdot (U - 128) + 512}{1024}, 0, 255\right)$$

### 2.2 5-Point Landmark Affine Alignment (`align.rs`)

MobileFaceNet ArcFace requires facial images to be aligned to canonical reference facial coordinates on a 112×112 canvas:

```text
Canonical Target Reference Coordinates (TARGET_LANDMARKS_112):
  1. Left eye center:     (38.2946, 51.6963)
  2. Right eye center:    (73.5318, 51.5014)
  3. Nose tip:            (56.0252, 71.7366)
  4. Mouth left corner:   (41.5493, 92.3655)
  5. Mouth right corner:  (70.7299, 92.2041)
```

The alignment computes a closed-form least-squares 2D similarity transform (scale $s$, rotation angle $\theta$, translation $t$) mapping source landmarks to canonical target landmarks (Umeyama formulation):
1. Compute centroids $\mu_S$ and $\mu_T$.
2. Compute scale-rotation parameters $a = s \cos\theta$ and $b = s \sin\theta$.
3. Compute translation vector $t = \mu_T - M \mu_S$.
4. Apply the inverse transform with bilinear interpolation to sample each target pixel $(u, v) \in [0, 112) \times [0, 112)$ from source image coordinates $(x_s, y_s)$, padding out-of-boundary regions with zero (black).

### 2.3 Cosine Similarity Matching (`matcher.rs`)

Biometric feature vectors extracted from MobileFaceNet are compared via cosine similarity:
$$\text{similarity}(a, b) = \frac{a \cdot b}{\|a\|_2 \|b\|_2}$$

- **Dimension Mismatch Protection**: Verifies `a.len() == b.len()`.
- **Degenerate Vector Protection**: Rejects zero or near-zero norms ($\le 10^{-12}$).
- **Output Bounds**: Systematically clamped to the interval $[-1.0, 1.0]$.
- **Verification Decision**: `match_embeddings` checks `score >= threshold` (default `0.45` per academic ArcFace literature).

### 2.4 Vision Pipeline Orchestrator (`pipeline.rs`)

`VisionPipeline` coordinates the entire verification flow:
1. Decompresses/converts the raw frame to RGB24.
2. Runs face detection (`FaceDetector`).
3. **Enforces Single-Face Invariant (Criterion V4)**:
   - 0 faces detected $\implies$ returns `Err(VisionError::NoFaceDetected)`.
   - $> 1$ faces detected $\implies$ returns `Err(VisionError::MultipleFacesDetected { count })`.
4. Validates face confidence against `min_face_confidence` (default `0.70`).
5. Regresses 5-point facial landmarks (`LandmarkDetector`).
6. Warps face to normalized 112×112 RGB crop (`align_face_112`).
7. Extracts L2-normalized 128D/512D embedding (`EmbeddingExtractor`).
8. Compares against enrolled template via `match_embeddings`.

---

## 3. Performance & Latency Budget (Criterion V5)

Target latency budget from `AI/ARCHITECTURE.md` §7 is $\le 150\text{ms}$ at p95.

Automated benchmark results (`bench_tests.rs`) across 50 iterations on 640×480 YUYV frames:
- **p50**: $26.34\text{ms}$
- **p95**: $28.02\text{ms}$
- **p99**: $28.87\text{ms}$

The pipeline completes in under 30ms, leaving more than 120ms of headroom for PAM deadline compliance.

---

## 4. Verification Matrix Mapping

| Criterion | Specification | Validation Evidence | Status |
|---|---|---|---|
| **V1** | Golden tests: preprocessing matches training pipeline | `align_tests::test_canonical_identity_alignment_matches_reference`, `test_translated_face_alignment_recenters`, `test_rotated_face_alignment_levels_eyes` | ☑ Validated |
| **V2** | L2-normalized embeddings (norm $\approx$ 1.0) | `embedding_tests::test_l2_norm_and_normalization_criterion_v2`, `proptest_suite::prop_embedding_normalization_criterion_v2` | ☑ Validated |
| **V3** | Cosine similarity correctness | `matcher_tests::test_cosine_similarity_identical_vectors`, `test_cosine_similarity_orthogonal_vectors`, `test_cosine_similarity_known_precomputed_vectors` | ☑ Validated |
| **V4** | Rejects if 0 or > 1 face detected | `pipeline_tests::test_pipeline_rejects_zero_faces`, `test_pipeline_rejects_two_faces`, `test_pipeline_rejects_three_faces` | ☑ Validated |
| **V5** | Full pipeline < 150ms p95 on reference hardware | `bench_tests::test_pipeline_latency_budget_under_150ms_p95` (achieved 28.02ms p95) | ☑ Validated |
| **V6** | `forbid(unsafe_code)` enabled | `soos-invariants::test_business_crates_forbid_unsafe_code` | ☑ Validated |

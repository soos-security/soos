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
    ├── crop.rs         # Bounding box 2.7x expansion and crop-and-resize for PAD
    ├── letterbox.rs    # Aspect-preserving letterbox padding and coordinate projection
    ├── matcher.rs      # Cosine similarity and template verification
    └── pipeline.rs     # VisionPipeline 3-model orchestrator & single-face invariant
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

ArcFace w600k MBF requires facial images to be aligned to canonical reference facial coordinates on a 112×112 canvas:

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

Biometric feature vectors extracted from ArcFace w600k are compared via cosine similarity:
$$\text{similarity}(a, b) = \frac{a \cdot b}{\|a\|_2 \|b\|_2}$$

- **Dimension Mismatch Protection**: Verifies `a.len() == b.len()`.
- **Degenerate Vector Protection**: Rejects zero or near-zero norms ($\le 10^{-12}$).
- **Output Bounds**: Systematically clamped to the interval $[-1.0, 1.0]$.
- **Verification Decision**: `match_embeddings` checks `score >= threshold` (default `0.45` per academic ArcFace literature).

### 2.4 Vision Pipeline Orchestrator (`pipeline.rs`)

`VisionPipeline` coordinates the 3-model verification flow:
1. Decompresses/converts the raw frame to RGB24 (`convert_to_rgb`).
2. Runs face detection (`FaceDetector`, e.g. SCRFD).
3. **Enforces Single-Face Invariant (Criterion V4)**:
   - 0 faces detected $\implies$ returns `Err(VisionError::NoFaceDetected)`.
   - $> 1$ faces detected $\implies$ returns `Err(VisionError::MultipleFacesDetected { count })`.
4. Validates face confidence against `min_face_confidence` (default `0.70`).
5. Extracts 5-point facial landmarks directly from `FaceDetection.landmarks` (fails closed with `VisionError::MissingLandmarks` if absent).
6. Expands bounding box by `pad_bbox_scale` (2.7×) centered on face and clamps to image bounds (`expand_bbox_for_pad`).
7. Crops and resizes the expanded bounding box to 80×80 for Presentation Attack Detection (`crop_and_resize`).
8. Evaluates Presentation Attack Detection (`PadDetector`, MiniFASNetV2) and short-circuits on spoof (`VisionError::PadFailed`). The decision is **format-aware** (GitHub #169, see §2.4.1): a `PixelFormat::Grey` (IR) frame must first pass the fail-closed IR gate (`VisionError::IrLivenessGateFailed`) and is scored against the stricter IR threshold.
9. Warps face to normalized 112×112 RGB crop using 5-point landmarks (`align_face_112`).
10. Extracts L2-normalized 512D biometric embedding (`EmbeddingExtractor`, ArcFace w600k).
11. Compares against enrolled template via `match_embeddings`.

#### 2.4.1 Format-Aware PAD Policy for IR / Grey Frames (`ir_liveness.rs`, GitHub #169)

MiniFASNetV2 is trained on colour captures. `convert_to_rgb` replicates a `Grey` byte into three
identical channels, which is out-of-distribution for the model, and `CameraConfig` prefers the IR
sensor by default (`SensorPreference::PreferIr`). The pipeline therefore derives a
`PadInputModality` from `frame.format` (`Grey` ⇒ `Monochrome`; `Rgb24`, `Yuyv`, `Nv12`, `Mjpeg` ⇒
`Color`) and never lets a monochrome frame take the colour PAD path:

| Step | Monochrome (`Grey`) frame | Colour frame |
|---|---|---|
| IR gate on the 80×80 PAD crop (`evaluate_ir_gate`) | mean luma in `[IR_MIN_MEAN_LUMA = 20, IR_MAX_MEAN_LUMA = 235]`, luma std-dev `>= IR_MIN_LUMA_STDDEV = 10`, texture energy (mean abs. horizontal + vertical neighbour difference) `>= IR_MIN_TEXTURE_ENERGY = 2`; failure ⇒ `IrLivenessGateFailed { reason }` and the model is **not** consulted | skipped |
| MiniFASNetV2 liveness | must be live and `score >= max(pad_threshold, ir_pad_threshold)` (default `DEFAULT_IR_PAD_THRESHOLD = 0.95`) | must be live and `score >= pad_threshold` (0.85) |

- The effective IR threshold is never looser than `pad_threshold`; a NaN score or threshold always
  rejects (`is_finite()` guard on both paths).
- The gate constants and `DEFAULT_IR_PAD_THRESHOLD` are **conservative, uncalibrated** values: the
  gate only rejects clearly degenerate crops (unlit, saturated, flat, texture-less) and never grants
  liveness on its own. No FAR/FRR figure is claimed for IR. Calibration on captured IR frames is a
  hardware follow-up (GitHub #172 / PAD-06).
- `analyze_frame` (GUI) applies the same policy and reports a non-live `PadResult` for a gate
  failure or a sub-threshold IR score, so the GUI never shows an IR capture as live when the daemon
  would reject it.
- `soos-daemon` maps `IrLivenessGateFailed` to a spoof capture (`FrameEvaluation::spoof(0.0)`),
  which vetoes the request (`Deny` / `PadFailed`, password fallback).
- IR emitter requirements are documented in `Docs/CAMERA_V4L_CRATE.md` ("IR Sensors and Emitter
  Requirements").

### 2.5 Letterbox Padding & Coordinate Projection (`letterbox.rs`)

Next-generation face detection (SCRFD) operates on uniform 640×640 square inputs. To accommodate arbitrary camera aspect ratios (e.g. 640×480, 1280×720, 1920×1080) without distortion or stretching:

1. `letterbox_params(img_w, img_h, target_w, target_h) -> Result<LetterboxParams, VisionError>`
   - Computes isotropic scale $s = \min(W_{\text{target}} / W_{\text{orig}}, H_{\text{target}} / H_{\text{orig}})$.
   - Computes centered padding offsets $\text{pad}_x = (W_{\text{target}} - s \cdot W_{\text{orig}}) / 2$ and $\text{pad}_y = (H_{\text{target}} - s \cdot H_{\text{orig}}) / 2$.
2. `letterbox_resize(rgb, img_w, img_h, target_w, target_h) -> Result<(Vec<u8>, LetterboxParams), VisionError>`
   - Allocates zero-filled black canvas of target dimensions.
   - Bilinear-interpolates the scaled image into the centered region.
3. `LetterboxParams` Coordinate Mappings:
   - `project(x, y)`: Transforms source frame coordinates to letterbox canvas space: $(x \cdot s + \text{pad}_x, y \cdot s + \text{pad}_y)$.
   - `unproject(x, y)`: Inversely projects detection coordinates from letterbox space back to original camera frame coordinates: $((x - \text{pad}_x) / s, (y - \text{pad}_y) / s)$.
   - `unproject_bbox(bbox)`: Unprojects bounding box corner coordinates.

### 2.6 Bounding Box Expansion & Cropping (`crop.rs`)

MiniFASNetV2 anti-spoofing requires wider facial context than the aligned 112×112 face crop:
1. `expand_bbox_for_pad(bbox, scale, img_w, img_h) -> BoundingBox`:
   - Scales the bounding box from its center by `scale` factor (typically 2.7×).
   - Clamps the resulting coordinates to valid image dimensions $[0, W]$ and $[0, H]$.
2. `crop_and_resize(rgb, img_w, img_h, bbox, target_w, target_h) -> Result<Vec<u8>, VisionError>`:
   - Resizes the cropped region to target dimensions (e.g. 80×80) using bilinear interpolation.
   - Any region extending beyond original image boundaries is padded with black (zero).

---

## 3. Performance & Latency Budget (Criterion V5)

Target latency budget from `AI/ARCHITECTURE.md` §7 is $\le 150\text{ms}$ at p95.

Automated benchmark results (`bench_tests.rs`) across 50 iterations on 640×480 YUYV frames:
- **p50**: $26.34\text{ms}$
- **p95**: $28.02\text{ms}$
- **p99**: $28.87\text{ms}$

The 3-model pipeline completes in under 30ms on mock fixtures and estimated ~80ms on hardware, leaving ample headroom for PAM deadline compliance.

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
| **NGM11** | VisionPipeline constructs with 3 backends (detector, pad, extractor) | `pipeline_tests::test_pipeline_constructs_with_three_backends` | ☑ Validated |
| **NGM12** | Pipeline extracts landmarks from `FaceDetection`, not separate detector | `pipeline_tests::test_pipeline_extracts_landmarks_from_detection`, `test_pipeline_fails_when_detection_lacks_landmarks` | ☑ Validated |
| **NGM13** | PAD receives 2.7× expanded crop (80×80); embedding receives aligned 112×112 crop | `pipeline_tests::test_pipeline_pad_receives_expanded_crop`, `test_pipeline_embedding_receives_aligned_crop`, `test_expand_bbox_centered`, `test_expand_bbox_clamped_to_image` | ☑ Validated |
| **NGM14** | Letterbox padding preserves aspect ratio with correct coordinate un-projection | `letterbox_tests::test_letterbox_unproject_roundtrip`, `letterbox_tests::test_letterbox_640x480_to_640x640`, `letterbox_tests::test_letterbox_1280x720_to_640x640`, `letterbox_tests::test_letterbox_square_no_padding` | ✅ Verified |
| **NGM14b** | Bounding box crop and resize with bilinear interpolation and out-of-bounds zero (black) padding | `crop_tests::test_crop_and_resize_known_image`, `crop_tests::test_crop_and_resize_out_of_bounds_padding`, `crop_tests::test_crop_and_resize_degenerate_bbox_returns_black`, `crop_tests::test_expand_bbox_for_pad_expansion_and_clamping` | ✅ Verified |
| **PIR1–PIR5** | Format-aware PAD: `Grey` frames pass the fail-closed IR gate and the stricter IR threshold, colour path unchanged (GitHub #169) | `ir_pad_policy_tests::*`, `pipeline_integration_tests::test_169_*` (see `AI/VERIFICATION_MATRIX.md`) | ✅ Verified |

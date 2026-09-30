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
    ├── crop.rs         # Upstream-parity PAD context window, crop-and-resize
    ├── pad_fusion.rs   # Multi-scale PAD fusion (mean live probability)
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
| `PixelFormat::Mjpeg` | Compressed JPEG stream | Bounded, header-checked pure-Rust `jpeg-decoder` decompression (§2.1.1) | Standard RGB24 |

#### 2.1.1 Bounded MJPEG Decoding (GitHub #190, review finding VIS-02)

The SOF header of a device-supplied MJPEG frame is attacker-controlled (faulty firmware, BadUSB),
so `convert_to_rgb` never allocates from it before checking it:

1. The compressed buffer must not exceed `MAX_MJPEG_COMPRESSED_BYTES` (16 MiB), else
   `VisionError::MjpegInputTooLarge { max, actual }`.
2. The negotiated frame must not exceed `MAX_MJPEG_DIMENSION` (4096 px per side, covers 4K UHD),
   else `VisionError::InvalidDimensions`.
3. Only the headers are parsed (`Decoder::read_info`); the SOF width and height must equal the
   negotiated frame dimensions, else `VisionError::MjpegFrameMismatch { expected_width,
   expected_height, actual_width, actual_height }`. A transposed frame with the same byte count
   and a 65535×65535 header are both rejected here, before any pixel is decoded.
4. Only 8-bit YCbCr/RGB and 8-bit greyscale JPEGs are accepted (UVC MJPEG is YCbCr); CMYK and
   16-bit frames fail closed with `VisionError::ColorConversionFailed`.
5. `Decoder::set_max_decoding_buffer_size` caps the output at the exact expected size and the
   decoded length is re-checked (`InvalidBufferSize`) before the RGB24 buffer is returned.

Every failure is a typed error, never a panic; error strings never contain pixel data. Contract:
`tests/mjpeg_bounds_tests.rs` (synthetic 16×8 fixtures plus three proptest properties on the
decoder entry point: arbitrary bytes, mutated real frames with arbitrary SOF dimensions, and
header geometry that must match).

#### YUYV Conversion Formula (Fixed-Point BT.601 Full Range)
$$R = \text{clamp}\left(Y + \frac{1436 \cdot (V - 128) + 512}{1024}, 0, 255\right)$$
$$G = \text{clamp}\left(Y - \frac{352 \cdot (U - 128) + 731 \cdot (V - 128) - 512}{1024}, 0, 255\right)$$
$$B = \text{clamp}\left(Y + \frac{1815 \cdot (U - 128) + 512}{1024}, 0, 255\right)$$

### 2.2 5-Point Landmark Affine Alignment (`align.rs`)

The ArcFace embedding model (an ArcFace ResNet34 attested as `arcface_w600k_mbf`, see `models/README.md`) requires facial images to be aligned to canonical reference facial coordinates on a 112×112 canvas:

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

Biometric feature vectors extracted by the ArcFace embedding model are compared via cosine similarity:
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
6. Computes the PAD context window at `pad_bbox_scale` (2.7×) with the upstream Silent-Face-Anti-Spoofing geometry (`pad_crop_window`, GitHub #213, §2.6).
7. Resizes that window to 80×80 with the `cv2.resize` `INTER_LINEAR` half-pixel convention (`crop_pad_context`). Each additional multi-scale PAD member (`with_additional_pad_model`, GitHub #212, §2.4.2) gets its own window at its own scale.
8. Evaluates Presentation Attack Detection (`PadDetector`, MiniFASNetV2; several members are fused by `fuse_pad_results`) and short-circuits on spoof (`VisionError::PadFailed`). The decision is **format-aware** (GitHub #169, see §2.4.1): a `PixelFormat::Grey` frame, or any frame from an `Infrared` sensor, must first pass the fail-closed IR gate (`VisionError::IrLivenessGateFailed`) and is scored against the stricter IR threshold.
9. Warps face to normalized 112×112 RGB crop using 5-point landmarks (`align_face_112`).
10. Extracts L2-normalized 512D biometric embedding (`EmbeddingExtractor`, ArcFace ResNet34, NHWC input).
11. Compares against enrolled template via `match_embeddings`.

#### 2.4.1 Format-Aware PAD Policy for IR / Grey Frames (`ir_liveness.rs`, GitHub #169)

MiniFASNetV2 is trained on colour captures. `convert_to_rgb` replicates a `Grey` byte into three
identical channels, which is out-of-distribution for the model, and `CameraConfig` prefers the IR
sensor by default (`SensorPreference::PreferIr`). The pipeline therefore derives a
`PadInputModality` with `PadInputModality::for_frame` from `frame.format` **and**
`frame.sensor_type`: any frame from an `Infrared` sensor is `Monochrome` whatever its pixel format
(an IR node streaming YUYV or MJPEG never takes the colour path); otherwise `Grey` ⇒ `Monochrome`
and `Rgb24`, `Yuyv`, `Nv12`, `Mjpeg` ⇒ `Color`. A monochrome frame never takes the colour PAD path:

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

#### 2.4.2 Multi-Scale PAD Fusion (`pad_fusion.rs`, GitHub #212)

Upstream Silent-Face-Anti-Spoofing scores a face with two models on two crops (2.7× MiniFASNetV2 and
4.0× MiniFASNetV1SE) and averages their softmax vectors. `VisionPipeline::with_additional_pad_model(scale, detector)`
adds such a member (at most `MAX_PAD_ENSEMBLE_MODELS = 4` models including the primary; the scale must
be finite and positive, otherwise `VisionError::InvalidPadEnsemble`). `pad_scales()` lists the scales,
primary first.

- Every member runs on its own `crop_pad_context` window; any crop or inference error fails the frame.
- `fuse_pad_results`: one member is returned unchanged (single-model behaviour, the default); several
  members give `score = mean(member scores)` (the mean softmax live probability) and
  `is_live = score.is_finite() && score >= threshold`. A NaN/inf member or NaN threshold rejects.
- The threshold is the modality-aware one (`pad_threshold`, or the stricter IR threshold). For
  monochrome frames the IR gate runs on the primary crop before any model is consulted.
- `analyze_frame` (GUI) reports the fused result.
- **Current deployment**: the attested model set holds only MiniFASNetV2, so `soos-daemon`,
  `soos-enroll` and `soos-gui` still build a single-member pipeline. Wiring the 4.0× MiniFASNetV1SE
  needs its ONNX export attested in `models/manifest.toml` (SHA-256, I/O shapes) and a threshold
  re-measurement; it is a follow-up of GitHub #212 (ADR 2026-09-30 "Upstream-Parity PAD Crop
  Geometry and Multi-Scale Fusion").

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

MiniFASNetV2 anti-spoofing requires wider facial context than the aligned 112×112 face crop.
The PAD model input is built with the **upstream reference geometry** (GitHub #213):

1. `pad_crop_window(bbox, scale, img_w, img_h) -> Option<PadCropWindow>`: transcription of
   upstream `CropImage._get_new_box` + `CropImage.crop`. With `(x, y, w, h) = (x1, y1, x2 - x1, y2 - y1)`,
   the scale is capped by `(H - 1) / h` and `(W - 1) / w`, the scaled box is centred on the face,
   shifted inward against `0` and the **last pixel index** `W - 1` / `H - 1`, corners are truncated
   like Python `int()` and the slice is inclusive. The requested `f32` scale is snapped to 6 decimals
   so `2.7` behaves like the Python float `2.7`. Zero frames, non-finite or empty boxes and a
   non-finite or non-positive scale give `None`.
2. `crop_pad_context(rgb, img_w, img_h, bbox, scale, target_w, target_h)`: resizes that window with
   the `cv2.resize` `INTER_LINEAR` convention: `src = (dst + 0.5) * (window / target) - 0.5`, clamped
   to the window (replicated border), bilinear, round to nearest. A degenerate box gives an
   all-black crop (scored as a spoof). Golden fixtures produced by
   `crates/vision/tests/fixtures/pad_crop/generate_golden.py` (NumPy transcription of the upstream
   crop; OpenCV is not used) are matched within ±1 LSB.

Continuous-coordinate helpers (display overlays and generic crops):

1. `expand_bbox_for_pad(bbox, scale, img_w, img_h) -> BoundingBox` (GUI overlay only):
   - Scales the bounding box from its center by `scale` factor (typically 2.7×); the effective scale is first bounded so that the expanded box never exceeds the image canvas (`min(scale, W / w, H / h)`).
   - When the expanded box crosses an image border it is **shifted inward** (translated, not clipped), matching Minivision `CropImage._get_new_box`, so the PAD crop keeps its context scale and aspect ratio instead of being distorted by independent per-edge clamping. A final clamp to $[0, W]$ × $[0, H]$ only guards floating-point residue.
2. `crop_and_resize(rgb, img_w, img_h, bbox, target_w, target_h) -> Result<Vec<u8>, VisionError>`:
   - Resizes the cropped region to target dimensions (e.g. 80×80) using bilinear interpolation with
     half-pixel centre sampling `x1 + (u + 0.5) * sx - 0.5` (GitHub #213).
   - Samples within half a pixel of the image replicate the border; regions further out are padded
     with black (zero).

---

## 3. Performance & Latency Budget (Criterion V5)

Target latency budget from `AI/ARCHITECTURE.md` §7 is $\le 150\text{ms}$ at p95.

Automated benchmark results (`bench_tests.rs`) across 50 iterations on 640×480 YUYV frames:
- **p50**: $26.34\text{ms}$
- **p95**: $28.02\text{ms}$
- **p99**: $28.87\text{ms}$

These figures use mock backends and measure only the Rust preprocessing path. With the attested
embedding model, one ArcFace ResNet34 embedding alone measures p50 127.5 ms / p95 170.9 ms on one
ORT intra-op thread (`soos-inference-ort` `embedding_real_model_tests`, GitHub #191), so the
150 ms per-capture target is **not** met on real models. The enforced bounds are the daemon's
`DECISION_BUDGET_MS = 900` and its inference admission estimate; see ADR 2026-09-30 (Face
Embedding Model Identity & Retention) for the decision to keep this model and the scheduled
evaluation of a lighter one.

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
| **MJB1–MJB6** | Bounded, header-checked MJPEG decoding (GitHub #190) | `mjpeg_bounds_tests::*` (see `AI/VERIFICATION_MATRIX.md`) | ✅ Verified |
| **PIR1–PIR5** | Format-aware PAD: `Grey` frames pass the fail-closed IR gate and the stricter IR threshold, colour path unchanged (GitHub #169) | `ir_pad_policy_tests::*`, `pipeline_integration_tests::test_169_*` (see `AI/VERIFICATION_MATRIX.md`) | ✅ Verified |

# Walkthrough 117 — Upstream-Parity PAD Crop Geometry and Multi-Scale PAD Fusion

- **Date**: 2026-09-30
- **Issues**: GitHub #213 (review finding PAD-08, crop geometry), GitHub #212 (review finding
  PAD-07, 4.0-scale fusion; partial, see section 6)
- **Branch**: `fix/p2-pad-multiscale-crop-geometry`
- **Matrix criteria**: PMC6–PMC11 (new, component `pad-crop-geometry-and-fusion`; the prefix
  continues the existing PMC1–PMC5 numbering without reusing an ID)
- **ADR**: 2026-09-30 "Upstream-Parity PAD Crop Geometry and Multi-Scale Fusion"

---

## 1. Context

The MiniFASNet PAD models were trained on crops produced by upstream Silent-Face-Anti-Spoofing
`CropImage._get_new_box` + `CropImage.crop` followed by `cv2.resize(INTER_LINEAR)`. The review
found three deviations in soos (PAD-08):

1. `expand_bbox_for_pad` bounded the box by `img_w` / `img_h` and capped the scale by
   `img_w / box_w`; upstream uses the last pixel index `src_w - 1` / `src_h - 1` and an inclusive
   slice after `int()` truncation.
2. `crop_and_resize` sampled `x1 + u * sx` (top-left aligned); `cv2.resize` samples
   `(u + 0.5) * sx - 0.5`.
3. `OrtPadDetector::prepare_input` fell back to top-left nearest-neighbour for a crop that is not
   80x80.

PAD-07: upstream averages the softmax of a 2.7-scale MiniFASNetV2 and a 4.0-scale
MiniFASNetV1SE; soos ran one model on one crop.

## 2. Design

- `soos_vision::crop::pad_crop_window` is a line-by-line transcription of `_get_new_box` in `f64`
  (`PadCropWindow { x, y, width, height }`, always inside the frame, `None` for invalid inputs).
  The `f32` scale is snapped to 6 decimals: widening `2.7f32` gives `2.7000000477`, which moved the
  interior window of a 100 px face from column 15 to 14 during development.
- `crop_pad_context` resizes the window with the half-pixel `INTER_LINEAR` rule, clamped to the
  window (replicated border). The pipeline uses it for every PAD crop (daemon, enroll and GUI
  analysis paths all go through `VisionPipeline`).
- `expand_bbox_for_pad` keeps its `[0, W] x [0, H]` continuous contract, pinned by existing tests
  (`crop_tests::test_expand_bbox_for_pad_expansion_and_clamping`,
  `pipeline_tests::test_expand_bbox_clamped_to_image`); it is now only the GUI overlay helper.
- `crop_and_resize` and `prepare_input` switch to half-pixel bilinear sampling. For an exact
  80x80 crop and for the 2x-downscaled 160x160 checkerboard of the real-model golden test the
  output is bit-identical, so the golden logits did not move.
- Fusion (`soos_vision::pad_fusion::fuse_pad_results`): one member is returned unchanged; several
  members give the mean live probability, thresholded fail-closed. The member scores are the
  softmax live probabilities, so this equals the live component of upstream's averaged vector.
  Because soos thresholds are at least 0.5, the decision is never looser than upstream's argmax.
- `VisionPipeline::with_additional_pad_model(scale, detector)` adds a member (bounded by
  `MAX_PAD_ENSEMBLE_MODELS = 4`, finite positive scale, else `VisionError::InvalidPadEnsemble`).
  For monochrome frames the IR gate runs on the primary crop before any model.

## 3. Golden Reference

OpenCV is prohibited and not installed, so `crates/vision/tests/fixtures/pad_crop/generate_golden.py`
transcribes `_get_new_box` / `crop` verbatim and implements the documented `INTER_LINEAR` rule in
NumPy (float64, round half up). It writes three 80x80 RGB fixtures from a formula-defined,
non-biometric 160x120 frame: an interior 2.7x crop, a scale-capped right/bottom-edge 4.0x crop and
an upscaled 2.7x crop. The Rust crop matches all three within ±1 LSB (tolerance for OpenCV's
fixed-point rounding). A second test checks the analytic output on a linear ramp, where bilinear
interpolation is exact.

## 4. Red → Green Evidence

Before the implementation (tests written first on `e2f602c`):

| Test | Result before |
|---|---|
| `crop_center_sampling_tests::test_crop_and_resize_downscale_samples_pixel_centres` | FAILED: `[0, 20, 40, 60]` instead of `[5, 25, 45, 65]` |
| `crop_center_sampling_tests::test_crop_and_resize_upscale_replicates_border_half_pixel` | FAILED: `[0, 20, 40, 60, 80, 100, 120, 0]` (last sample black) |
| `pad_resample_tests::test_pad_prepare_input_downscale_uses_half_pixel_centres` | FAILED (nearest-neighbour value) |
| `pad_resample_tests::test_pad_prepare_input_upscale_clamps_to_border` | FAILED |
| `pad_crop_geometry_tests`, `pad_ensemble_tests`, `pad_crop_real_model_tests` | did not compile (`pad_crop_window`, `crop_pad_context`, `pad_fusion`, `with_additional_pad_model`, `InvalidPadEnsemble` missing) |

After: all of them pass; `pad_resample_tests::test_pad_prepare_input_80x80_is_identity` passed
before and after (identity preserved). Real-model run on this host (`/var/lib/soos/models`):
input dims `[-1, 3, 80, 80]`; synthetic 640x480 frame, face box near the right edge: real
MiniFASNetV2 scores 0.005487 (2.7x crop) and 0.005389 (4.0x crop); the fused pipeline rejects with
their mean. `pad_real_model_tests` golden logits unchanged.

## 5. Gate

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast` (1102 passed, 0 failed)
and `./scripts/candid_review.sh` pass. No pre-existing test was modified.

## 6. Not Done / Follow-ups

- **4.0x MiniFASNetV1SE in production (#212, PMC11 pending)**: the model is not in the attested
  set and no model was downloaded. Needed: attest the ONNX export in `models/manifest.toml`
  (SHA-256, I/O shapes; coordinated update of
  `manifest_tests::test_manifest_v2_model_count_and_checksum_attestation`), build the second
  `OrtPadDetector` in `soos-daemon`, `soos-enroll` and `soos-gui` and call
  `with_additional_pad_model(4.0, ...)`, then re-measure the fused threshold (and the latency
  budget, about +8-15 ms) on the PAD-06 corpus.
- Re-fitting the 2.7 scale for SCRFD boxes (tighter than upstream RetinaFace boxes) needs the
  PAD-06 corpus.
- Regenerating the golden fixtures with real `cv2.resize` on a machine that has OpenCV (outside
  the repository) would also cover OpenCV's fixed-point rounding directly.
- The GUI PAD overlay still draws `expand_bbox_for_pad` (at most one pixel from the model window).

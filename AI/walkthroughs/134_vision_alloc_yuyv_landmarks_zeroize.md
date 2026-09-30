# Walkthrough 134 — Vision Hot-Path Allocations, YUYV Odd Width, Non-Finite Landmarks and ORT Output Wiping

- **Date**: 2026-09-30
- **Issues**: GitHub #252 (VIS-10), #253 (VIS-11), #254 (VIS-12), #255 (VIS-13)
- **Branch**: `fix/p2-vision-alloc-yuyv-landmarks-zeroize`
- **Matrix criteria**: VAY1–VAY7 (new component `vision-alloc-yuyv-landmarks-zeroize`)
- **ADR**: 2026-09-30 "ORT Output Tensors Wiped In Place; SCRFD Scratch Reuse and Bounded ORT Threads"

---

## 1. Findings

| Issue | Finding | Location (before) |
|---|---|---|
| #252 | 4.9 MB letterbox tensor allocated per `detect`; all nine SCRFD outputs copied (`slice.to_vec()`, ~1 MB per frame); RGB24 frames copied by `convert_to_rgb`; ORT hard-coded to one intra-op thread with no configuration knob | `detector.rs`, `color.rs`, `registry.rs` |
| #253 | YUYV accepted any `width * height * 2` bytes; an odd width dropped the trailing 2 bytes (`chunks_exact(4)`) and returned a short buffer with `Ok` | `color.rs` |
| #254 | NaN landmarks passed the `<=` degeneracy guards of `align_face_112` and produced an all-black crop that was embedded; `decode_stride` had no finiteness check and `f32::clamp` propagates NaN | `align.rs`, `detector.rs` |
| #255 | Only input tensors were zeroized; ORT-owned output buffers (raw embedding, landmark tensors, PAD logits) were dropped unwiped; docs overstated the guarantee | `embedding.rs`, `detector.rs`, `pad.rs`, `Docs/INFERENCE_ORT_CRATE.md` |

## 2. Specification

- `soos_vision::convert_to_rgb_cow(raw, w, h, fmt) -> Result<Cow<'_, [u8]>, VisionError>`:
  `Cow::Borrowed` for validated RGB24, otherwise the `convert_to_rgb` output. `convert_to_rgb`
  is unchanged (GUI preview and `analyze_frame` keep an owned buffer). `process_frame` uses a
  private `RgbFrame` (borrowed, or owned `Zeroizing<Vec<u8>>`).
- YUYV: `width % 2 != 0` returns `InvalidDimensions` before the length check.
- `align_face_112`: `AlignmentFailed` when any landmark coordinate is non-finite or any
  transform coefficient (`sum_xx_yy`, `a`, `b`, `tx`, `ty`, `det`, `c1`, `c2`) is non-finite.
- `decode_stride`: skip a candidate whose raw score, decoded box corners (before clamping) or
  decoded keypoints are non-finite.
- `soos_inference_ort::detector::letterbox_pad_into(rgb, w, h, target, out)`: writes the full
  tensor (padding reset to 0.0) into a caller buffer of exactly `3 * target^2` values
  (`InvalidBufferSize` otherwise). `letterbox_pad` delegates to it.
- `OrtScrfdDetector` gains a private `input_scratch: Mutex<Zeroizing<Vec<f32>>>`; lock order
  scratch then session; the scratch is wiped after each inference.
- `soos_inference_ort::outputs::ZeroizingOutputs<'r>`: owns `SessionOutputs<'r>`, `Deref`s to it,
  `wipe() -> usize` overwrites every `f32` output tensor in place, `Drop` calls `wipe()`.
- `registry`: `DEFAULT_MAX_INTRA_THREADS = 4`, `MAX_INTRA_THREADS = 16`,
  `default_intra_threads()`, `RegistryConfig::allow_spinning` (default `false`, applied through
  `with_intra_op_spinning` / `with_inter_op_spinning`), `RegistryConfig::with_intra_threads`.
- `soos-daemon`: `PipelineConfig::inference_intra_threads` (`[pipeline] inference_intra_threads`,
  `1..=16` or `DaemonError::Config`), `pipeline::registry_config_for(&PipelineConfig)`.

## 3. Red Evidence

New tests were written first. On the unchanged code, the new API names did not resolve:

```
error[E0432]: unresolved import `soos_vision::convert_to_rgb_cow`
error[E0432]: unresolved import `soos_inference_ort::outputs`
error[E0432]: unresolved import `soos_inference_ort::detector::letterbox_pad_into`
error[E0432]: unresolved imports `soos_inference_ort::registry::default_intra_threads`, ...
error[E0609]: no field `allow_spinning` on type `RegistryConfig`
```

The invariant module failed on the unchanged sources (3 of 3 `ort_output_zeroize_contract`
tests FAILED). After adding only the two new helper functions (`convert_to_rgb_cow`,
`letterbox_pad_into`), the behavioural tests of the existing code failed:

```
frame_robustness_tests: test_yuyv_odd_width_rejected FAILED
                        test_yuyv_odd_width_with_even_pixel_count_rejected FAILED
                        test_align_rejects_nan_landmarks FAILED
                        test_align_rejects_infinite_landmarks FAILED
                        test_align_rejects_landmarks_whose_transform_overflows FAILED
                        test_pipeline_rejects_nan_landmarks_before_embedding FAILED
scrfd_robustness_tests: test_decode_stride_skips_non_finite FAILED
                        test_decode_stride_skips_overflowing_coordinates FAILED
```

## 4. Implementation Notes

- `BoundingBox::new` uses `f32::min`/`max`, which silently drop NaN, and `clamp` saturates
  infinity; the box check therefore runs on the unprojected corners before the box is built.
- The SCRFD decoder iterates `outputs.keys()` and borrows each tensor with `outputs.get(name)`
  (no panicking index), so the borrowed slices live as long as the guard; the guard is dropped
  (and wipes) before the session lock is released.
- A poisoned scratch lock is recovered (`PoisonError::into_inner`): the scratch is fully
  overwritten on each use, so it carries no state that would justify failing every later call.
  The session lock keeps its fail-closed "poisoned" error.
- The embedding extractor now builds one `TensorRef` from an NHWC or NCHW shape and runs once,
  so every file has exactly one `.run(` per `ZeroizingOutputs::new(` (checked by the invariant).
- The `OrtFaceDetector` (legacy UltraFace) path is wrapped in the same guard.

## 5. Measurements (development host, debug test profile, real models in `/var/lib/soos/models`)

| Model | Threads | p50 | p95 |
|---|---|---|---|
| SCRFD 500M KPS (640x480 synthetic frame) | 4 (`default_intra_threads()`) | 31.9 ms | 34.5 ms |
| ArcFace ResNet34 embedding | 4 (`default_intra_threads()`) | 27.1 ms | 28.4 ms |

The pre-existing `embedding_real_model_tests::test_real_embedding_latency_report` still prints
the label "1 intra-op thread" although the registry now uses the default thread count; that test
line is left unchanged (test integrity), see follow-ups.

## 6. Audit

- No `unwrap`/`expect`/indexing panics added in production paths; the only `Index` use is
  avoided in favour of `get`.
- Allocations are bounded: the scratch has a fixed size derived from `input_size`, checked
  multiplications guard the size computation.
- No frame, embedding or tensor value is logged; the latency report prints timings only.
- `#![forbid(unsafe_code)]` is unchanged in `soos-vision` and `soos-inference-ort`; the in-place
  wipe uses the safe `try_extract_tensor_mut` API.
- No PAM, socket or Tokio change.

## 7. Follow-ups

- ORT-internal activation buffers cannot be wiped through the public `ort` API (documented
  limitation in the ADR and `Docs/INFERENCE_ORT_CRATE.md`).
- Optional crop sanity check (reject an aligned crop with no in-bounds sample) was not added:
  it could reject mock-based fixtures whose canonical landmarks fall outside small synthetic
  frames, and it needs its own survey of those tests first.
- Latency on low-power laptops (the concern of VIS-10) still needs a measurement on such
  hardware; `[pipeline] inference_intra_threads` lets an operator tune it.
- The historical "1 intra-op thread" label in the embedding latency report.

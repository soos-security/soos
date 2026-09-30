# Walkthrough 133 — Single-Source Vision Thresholds and One Letterbox Implementation

- **Date**: 2026-09-30
- **Issues**: GitHub #215 (PAD-10), #248 (VIS-06), #250 (VIS-08), #251 (VIS-09)
- **Branch**: `fix/p2-vision-thresholds-docs`
- **Matrix criteria**: VTD1–VTD8 (new, verified), VTD9 (new, pending hardware)
- **ADR**: 2026-09-30 "Single-Source Vision Thresholds and Letterbox"

---

## 1. Context

The full project review of 2026-09-29 reported four related findings in the vision and PAD code:

| Finding | Problem |
|---|---|
| VIS-09 (#251) | Detector and PAD thresholds were written as literals in each binary: daemon NMS 0.45, GUI conf 0.60 / NMS 0.40 / PAD 0.80, enrollment NMS 0.40 / PAD 0.80 |
| PAD-10 (#215) | Three PAD thresholds: 0.80 in the detector, 0.85 in the pipeline, and a hardcoded 0.80 for the GUI LIVE label. A score of 0.82 showed LIVE in the GUI, but the daemon rejected it |
| VIS-06 (#248) | Two letterbox implementations. The tested one (bilinear, in `soos-vision`) was not used by the daemon. The production one used nearest neighbour and placed the image at a truncated offset, but un-projected with the fractional one (0.5 px shift on odd padding) |
| VIS-08 (#250) | Docs said match threshold 0.45 and ordinal SCRFD output parsing, and the vision module tree left out `pose.rs` |

## 2. Red evidence (new tests on unmodified production code)

| Test | Result before the fix |
|---|---|
| `letterbox_bilinear_tests::test_letterbox_odd_padding_unproject_exact` | FAILED: `pad_y` was 80.5 (not an integer) |
| `letterbox_bilinear_tests::test_letterbox_integer_offsets_for_all_odd_heights` | FAILED: fractional offset |
| `letterbox_bilinear_tests::test_letterbox_pad_is_bilinear_on_downscale` | FAILED: nearest neighbour gave 0 where bilinear gives 100 |
| `letterbox_parity_tests::test_letterbox_pad_matches_vision_resize_on_gradient` | FAILED at 640×479, pixel (1,80): the two implementations differ |
| `threshold_constants_tests::*` (5 tests) | did not compile: no `DEFAULT_*` constants, no `nms_iou_threshold`, no `pad_passes` |
| `vision_threshold_parity_tests::*` (2 tests) | did not compile: no `soos_vision::DEFAULT_PAD_THRESHOLD`, no `nms_iou_threshold` |
| `vision_threshold_contract::test_binaries_build_detectors_from_pipeline_config` | FAILED: literals in `gui/src/main.rs`, `enrollment-cli/src/service.rs` and `daemon/src/pipeline.rs` |
| `vision_threshold_contract::test_gui_never_hardcodes_pad_display_threshold` | FAILED: `app.rs` had `p.score >= 0.80` twice |
| `vision_threshold_contract::test_vision_and_inference_docs_list_every_module` | FAILED: missing `ir_liveness.rs` and `pose.rs`, and no inference-ort module tree |
| `vision_threshold_contract::test_vision_docs_state_code_thresholds_and_parsing` | FAILED: stale 0.45 default, and the ADR still said ordinal grouping |

## 3. Change

### 3.1 Thresholds (#251, #215)

- `crates/vision/src/pipeline.rs` has four new constants, re-exported from the crate root:
  `DEFAULT_MIN_FACE_CONFIDENCE = 0.70`, `DEFAULT_NMS_IOU_THRESHOLD = 0.45`,
  `DEFAULT_MATCH_THRESHOLD = 0.70` and `DEFAULT_PAD_THRESHOLD = 0.85`. `VisionPipelineConfig` has a
  new field, `nms_iou_threshold`, and its `Default` is built from these constants.
- `VisionPipelineConfig::pad_passes(&PadResult, PadInputModality)` is the single liveness
  decision. The pipeline's `evaluate_pad_crop` now calls it.
- `soos-daemon`, `soos-enroll` and `soos-gui` build `OrtScrfdDetector` from
  `min_face_confidence` / `nms_iou_threshold` and the PAD detector from `pad_threshold`. All three
  read `VisionPipelineConfig`, so one value reaches both the detector and the pipeline.
- GUI: the worker computes `pad_passes` once for each frame, using the frame's modality. It stores
  the result in the new `LatestFrameData::pad_live` field. The LIVE/SPOOF label, the label colour,
  the box colour and the guided-enrollment gating all read that field.
- No threshold was lowered. The GUI values went up (confidence 0.60 → 0.70, NMS 0.40 → 0.45,
  PAD 0.80 → 0.85). The enrollment values went up too (NMS 0.40 → 0.45, PAD 0.80 → 0.85). A lower NMS
  IoU merges more overlapping boxes, which can hide a second face from the single-face invariant.
  That makes 0.45 the stricter value.
- The policy `pad_threshold` is still used: `PadAggregator::classify` applies it again for each
  request (VTD8). The `AuthContext::pad_score` from the review recommendation was not added (see
  the ADR).

### 3.2 Letterbox (#248)

- There is a new module, `crates/inference-ort/src/letterbox.rs`. It provides
  `letterbox_geometry` (scale, scaled size, integer offsets `floor((target - scaled) / 2)`) and
  `letterbox_bilinear`, a bounded bilinear resampler that passes each pixel to a callback.
  The module lives in `soos-inference-ort` because `soos-vision` depends on it.
- `detector::letterbox_pad` packs the callback output into the NCHW BGR tensor. It returns the
  integer offsets, and the same offsets are used for un-projection.
- `soos_vision::letterbox::{letterbox_params, letterbox_resize}` now delegate to that module.
  Their signatures and error variants are unchanged, and every `letterbox_tests` case still passes.

### 3.3 Documentation (#250)

- `Docs/VISION_CRATE.md`:
  - The module tree now lists `ir_liveness.rs` and `pose.rs`.
  - It has a new §2.4.0 threshold table.
  - The match threshold is now given as `DEFAULT_MATCH_THRESHOLD = 0.70`, not 0.45.
  - It describes the integer letterbox offsets and states that the latency figures come from mocks only.
- `Docs/INFERENCE_ORT_CRATE.md`:
  - New module layout.
  - It describes the shared letterbox and the matching of outputs by shape.
  - It says where the detector and PAD thresholds come from.
- `AI/DECISIONS.md`: the 2026-09-20 SCRFD entry now carries a supersession marker, and there is a
  new ADR. The "class 0 = live" wording listed in the finding had already been removed and is
  enforced by `pad_contract::test_minifasnet_class_contract_prose_matches_code_constant`.

## 4. Security audit

- No `unwrap`/`expect`, no `unsafe`. The new letterbox code reads pixels with `get(..)`, and every
  destination index is clamped to the canvas. The output buffers are sized from checked or bounded
  dimensions.
- The tensor stays `Zeroizing`. No new intermediate RGB buffer is allocated on the detector path.
- Nothing is logged. There are no PAM, IPC or socket changes.

## 5. Follow-ups

- VTD9: measure SCRFD recall with the bilinear letterbox on real 1280×720 / 1920×1080 captures.
  The resampler samples at corner-aligned `dst / scale`, not at half-pixel centres like
  `cv2.INTER_LINEAR`.
- Optional: the detector could return raw probabilities only, which would remove its `is_live`
  threshold. It is harmless today because the value always equals the pipeline's.

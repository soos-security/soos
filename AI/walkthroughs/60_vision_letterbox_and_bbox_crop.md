# Walkthrough 60: Vision Letterbox Padding and Bounding Box Cropping Utilities

## Context & Objectives

Issue #42 (GitHub #108) implements and standardizes two core image preprocessing utilities within the pure-Rust `soos-vision` crate:
1. **Letterbox Padding (`letterbox_resize` & `LetterboxParams`)**: Aspect-ratio-preserving resize into target dimensions (such as 640×640 for the SCRFD face detector) with symmetric zero-filled (black) border padding and exact bidirectional coordinate projection/un-projection.
2. **Bounding Box Cropping (`crop_and_resize`)**: Extraction of context-expanded bounding boxes (2.7× for MiniFASNetV2 Presentation Attack Detection) resized to target dimensions (80×80) using bilinear interpolation and black padding for out-of-bounds sampling.

---

## 1. Architecture & Type Specifications

Implemented in `crates/vision/src/letterbox.rs` under `#![forbid(unsafe_code)]`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LetterboxParams {
    pub scale: f32,
    pub pad_x: f32,
    pub pad_y: f32,
}

impl LetterboxParams {
    pub fn new(scale: f32, pad_x: f32, pad_y: f32) -> Self;
    pub fn unproject(&self, x: f32, y: f32) -> (f32, f32);
    pub fn unproject_bbox(&self, bbox: &BoundingBox) -> BoundingBox;
    pub fn project(&self, x: f32, y: f32) -> (f32, f32);
}

pub fn letterbox_params(
    img_w: u32,
    img_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<LetterboxParams, VisionError>;

pub fn letterbox_resize(
    rgb: &[u8],
    img_w: u32,
    img_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<(Vec<u8>, LetterboxParams), VisionError>;
```

### Safety & Invariants
- **Checked Arithmetic**: Pre-computation of expected buffer sizes (`img_w * img_h * 3`) and output sizes (`target_w * target_h * 3`) uses `checked_mul` to prevent integer overflow.
- **Fail-Closed Validation**: Zero dimensions (`width == 0` or `height == 0`) and buffer length mismatches return `VisionError::InvalidDimensions` and `VisionError::InvalidBufferSize`.
- **Degenerate Scale Protection**: Scale values $\le 0.0$ fail closed in `unproject()`, systematically returning `(0.0, 0.0)`.

---

## 2. Tester Contracts & TDD Cycle

### Contractual Suites
- `crates/vision/tests/letterbox_tests.rs`:
  - `test_letterbox_640x480_to_640x640`: Validates scale 1.0, horizontal pad 0.0, vertical pad 80.0, top/bottom black borders.
  - `test_letterbox_1280x720_to_640x640`: Validates scale 0.5, horizontal pad 0.0, vertical pad 140.0, top/bottom black borders.
  - `test_letterbox_square_no_padding`: Validates 500×500 to 640×640 with scale 1.28 and zero padding.
  - `test_letterbox_tall_portrait_padding`: Validates 480×640 to 640×640 with scale 1.0 and left/right 80px borders.
  - `test_letterbox_zero_dimensions_fail_closed`: Validates rejection of 0-dimension arguments.
  - `test_letterbox_buffer_length_mismatch_fails_closed`: Validates buffer length verification.
  - `test_letterbox_unproject_bbox`: Validates inverse projection of detection bounding boxes.
  - `test_letterbox_unproject_roundtrip` (NGM14 proptest): Property test verifying that `unproject(project(x, y)) == (x, y)` across arbitrary valid dimensions and point coordinates.
- `crates/vision/tests/crop_tests.rs`:
  - `test_crop_and_resize_known_image`: Validates exact RGB channel sampling on synthetic quadrant test pattern.
  - `test_crop_and_resize_out_of_bounds_padding`: Validates black padding for coordinates outside `[0, W]` or `[0, H]`.
  - `test_crop_and_resize_zero_dimensions_rejected`: Validates rejection of degenerate canvas dimensions.
  - `test_crop_and_resize_degenerate_bbox_returns_black`: Validates safe black buffer return for inverted/zero-area bounding boxes.
  - `test_expand_bbox_for_pad_expansion_and_clamping`: Validates centered 2.7× context expansion and edge clamping.

---

## 3. Implementation Details

- **Module Scaffolding**: Added `crates/vision/src/letterbox.rs` and re-exported `letterbox_params`, `letterbox_resize`, and `LetterboxParams` in `crates/vision/src/lib.rs`.
- **Pure-Rust Bilinear Interpolation**: Maps canvas pixel indices to floating-point source coordinates and applies 4-corner bilinear weighting $(1-dx)(1-dy)$, $dx(1-dy)$, $(1-dx)dy$, and $dx \cdot dy$.
- **Issue Synchronization**: Registered topic branch `feat/vision-letterbox-and-bbox-crop` to Backlog Issue #42 / GitHub Issue #108 in `scripts/sync_issue.py`.

---

## 4. Candid Reviewer Assessment

The independent Candid Reviewer audited the diff against `origin/main` across all 5 architectural pillars (`AI/candid_review_report.md`):
- **Logic & Architecture**: PASS
- **PAM Concurrency & Real-Time Deadlines**: PASS (< 2ms execution, zero async runtimes, zero stdout/stderr pollution)
- **Panic Safety & Fail-Closed Behavior**: PASS (zero `unwrap`/`expect`, fail-closed error handling)
- **Test Integrity & Anti-Weakening**: PASS (zero weakened tests, comprehensive proptest coverage)
- **Memory & Secret Bounds**: PASS (bounded allocations, no credential exposure)
- **Verdict**: `VERDICT: APPROVED`

---

## 5. Verification Results

```text
running 9 tests (letterbox_tests.rs)
test test_letterbox_degenerate_scale_unproject_safe ... ok
test test_letterbox_buffer_length_mismatch_fails_closed ... ok
test test_letterbox_unproject_bbox ... ok
test test_letterbox_zero_dimensions_fail_closed ... ok
test test_letterbox_unproject_roundtrip ... ok
test test_letterbox_1280x720_to_640x640 ... ok
test test_letterbox_tall_portrait_padding ... ok
test test_letterbox_640x480_to_640x640 ... ok
test test_letterbox_square_no_padding ... ok
test result: ok. 9 passed; 0 failed; 0 ignored

running 6 tests (crop_tests.rs)
test test_crop_and_resize_buffer_size_mismatch_rejected ... ok
test test_crop_and_resize_degenerate_bbox_returns_black ... ok
test test_crop_and_resize_known_image ... ok
test test_crop_and_resize_out_of_bounds_padding ... ok
test test_crop_and_resize_zero_dimensions_rejected ... ok
test test_expand_bbox_for_pad_expansion_and_clamping ... ok
test result: ok. 6 passed; 0 failed; 0 ignored

Workspace-wide test suite: 100% passed (0 failed).
Clippy: 0 warnings across all targets and features.
Code Formatting: Compliant with rustfmt.
```

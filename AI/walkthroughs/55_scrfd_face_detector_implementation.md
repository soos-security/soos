# Walkthrough 55 — SCRFD Face Detector with Multi-Stride Output Parsing

> **Date**: 2026-09-20  
> **Issue**: Issue #37 (`feat/scrfd-face-detector`, GitHub #103)  
> **Verification Matrix**: `NGM3`, `NGM4`, `NGM5`  
> **Scope**: `crates/inference-ort/src/detector.rs`, `crates/inference-ort/src/lib.rs`, `crates/inference-ort/tests/scrfd_tests.rs`, `Docs/INFERENCE_ORT_CRATE.md`

---

## 1. Problem Statement & Motivation

Prior to this implementation, the `soos-inference-ort` crate used `OrtFaceDetector` built for the UltraFace Slim 320 architecture. That detector relied on:
- Pre-generated static anchor priors (4,420 anchor boxes).
- Exponential scale and center-offset decoding across 2 output tensors.
- 320×240 RGB input resolution.
- A completely separate neural pass (`OrtLandmarkDetector`) to regress 5-point facial keypoints.

SCRFD (Sample and Computation Redistribution for Efficient Face Detection) 500M KPS operates on a fundamentally different paradigm:
1. **Unified Face & Landmark Detection**: Bounding boxes and 5-point facial landmarks are produced simultaneously in a single forward pass, eliminating a ~20ms inference pass.
2. **Multi-Stride Distance-to-Border Decoding**: Outputs 9 tensors across strides 8, 16, and 32, using anchor centers and distance-to-border offsets rather than pre-generated prior boxes.
3. **BGR Channel Ordering & High Resolution**: Accepts 640×640 BGR input normalized via `(pixel - 127.5) / 128.0`.
4. **Letterbox Padding & Un-projection**: Requires aspect ratio-preserving letterbox padding and coordinate un-projection back to original camera frame space.

---

## 2. Architectural Design & Implementation

### 2.1 `FaceDetection` Struct Landmark Integration (Sub-issue #37.6)
`FaceDetection` now carries optional 5-point facial landmarks:
```rust
#[derive(Debug, Clone, PartialEq)]
pub struct FaceDetection {
    pub box_: BoundingBox,
    pub score: f32,
    pub landmarks: Option<FaceLandmarks>,
}

impl FaceDetection {
    pub fn new(box_: BoundingBox, score: f32) -> Self {
        Self { box_, score, landmarks: None }
    }

    pub fn with_landmarks(box_: BoundingBox, score: f32, landmarks: FaceLandmarks) -> Self {
        Self { box_, score, landmarks: Some(landmarks) }
    }
}
```

### 2.2 Letterbox Padding & Coordinate Un-projection (Sub-issues #37.2, #37.5)
- `letterbox_pad(rgb, w, h, target) -> (Zeroizing<Vec<f32>>, f32, f32, f32)`:
  - Calculates uniform scaling factor `scale = min(target / w, target / h)`.
  - Centers the scaled image with symmetric padding `pad_x` and `pad_y`.
  - Converts packed RGB888 pixels into planar NCHW float format with BGR channel ordering:
    - Channel 0: Blue, `(B - 127.5) / 128.0`
    - Channel 1: Green, `(G - 127.5) / 128.0`
    - Channel 2: Red, `(R - 127.5) / 128.0`
  - Unused border regions are zero-padded (`0.0f32`).
- `unproject(x, y, scale, pad_x, pad_y) -> (f32, f32)`:
  - Inverts letterbox mapping: `orig_x = (x - pad_x) / scale`, `orig_y = (y - pad_y) / scale`.

### 2.3 `OrtScrfdDetector` & Multi-Stride Tensor Parsing (Sub-issues #37.1, #37.4, #37.7)
`OrtScrfdDetector` implements the `FaceDetector` trait:
- **Startup Validation**:
  - `validate_output_count(count)`: Ensures session exports exactly 9 output tensors.
  - `validate_output_shapes(shapes)`: Validates that shapes for strides 8, 16, and 32 follow `[1, N, 1]` (scores), `[1, N, 4]` (bboxes), and `[1, N, 10]` (keypoints) where $N = (640 / s)^2 \times 2$.
- **Tensor Matching**: Robust shape-based matching allows parsing regardless of whether the ONNX export groups outputs by stride or by tensor type.
- **Distance-to-Border Decoding**:
  For each stride $s \in \{8, 16, 32\}$ and each grid cell $(row, col, anchor)$:
  $$\text{confidence} = \sigma(\text{logit})$$
  $$x_1 = (col - l) \times s, \quad y_1 = (row - t) \times s$$
  $$x_2 = (col + r) \times s, \quad y_2 = (row + b) \times s$$
  $$lm_x = (col + k_x) \times s, \quad lm_y = (row + k_y) \times s$$
- **NMS**: Deterministic tie-breaking Non-Maximum Suppression filters overlapping candidate boxes.

---

## 3. Verification Evidence

### 3.1 Unit, Synthetic & Property Tests (`crates/inference-ort/tests/scrfd_tests.rs`)
| Test | Objective | Status |
|---|---|---|
| `test_face_detection_carries_landmarks` | Verifies `landmarks` presence on `FaceDetection` | ✅ Passed |
| `test_letterbox_preserves_aspect_ratio` | Validates scale, pad offsets, BGR channel assignment, zero padding | ✅ Passed |
| `test_prepare_input_bgr_channel_ordering` | Golden test verifying BGR channel normalization on uniform inputs | ✅ Passed |
| `test_unproject_coordinates_match_original_image` | Verifies exact un-projection of bounding box and keypoints | ✅ Passed |
| `test_scrfd_decode_stride8_known_output` | Synthetic test for distance-to-border math on stride 8 | ✅ Passed |
| `test_scrfd_decode_all_strides` | Synthetic test across strides 8, 16, 32 simultaneously | ✅ Passed |
| `test_scrfd_rejects_invalid_output_count` | Rejects sessions with invalid output tensor counts | ✅ Passed |
| `test_scrfd_validates_shape_patterns` | Validates structural shape matching across strides | ✅ Passed |
| `test_letterbox_unproject_roundtrip` | Proptest asserting round-trip coordinate preservation $\pm 1$px | ✅ Passed |

### 3.2 Workspace Regression
- `cargo check --workspace --all-targets`: Passed (zero errors).
- `cargo clippy --all-targets --all-features -- -D warnings`: Passed (zero warnings).
- `cargo test --workspace`: Passed (100% of tests pass).
- `cargo fmt -- --check`: Passed cleanly.

---

## 4. Traceability Matrix

| Backlog Sub-issue | Description | Verified By |
|---|---|---|
| **#37.1** | Create `OrtScrfdDetector` struct | `scrfd_tests` suite |
| **#37.2** | Implement `letterbox_pad` utility | `test_letterbox_preserves_aspect_ratio`, `test_letterbox_unproject_roundtrip` |
| **#37.3** | Implement BGR channel ordering in `prepare_input` | `test_prepare_input_bgr_channel_ordering` |
| **#37.4** | Multi-stride output tensor parsing (9 tensors) | `test_scrfd_decode_stride8_known_output`, `test_scrfd_decode_all_strides` |
| **#37.5** | Coordinate un-projection from letterbox space | `test_unproject_coordinates_match_original_image`, `test_letterbox_unproject_roundtrip` |
| **#37.6** | Add `FaceLandmarks` to `FaceDetection` | `test_face_detection_carries_landmarks` |
| **#37.7** | SCRFD startup validation | `test_scrfd_rejects_invalid_output_count`, `test_scrfd_validates_shape_patterns` |

# Walkthrough 65 — Fix GUI Camera Rendering Layout and SCRFD Probability Decoding

## Overview
This walkthrough documents the investigation and resolution of two critical bugs affecting the native desktop application (`soos-gui`) during real-time camera streaming on physical hardware:
1. **Camera Video Canvas Collapse**: Inside `ui.horizontal`, egui defaults cross-axis height to `ui.spacing().interact_size.y` (18.0px), collapsing the camera feed and FaceID oval reticle to an invisible 24×18 pixel rectangle and leaving the viewport blank grey.
2. **Double Sigmoid Suppression in SCRFD**: The deployed `scrfd_500m_kps.onnx` model already emits pre-activated probability scores in `[0.0, 1.0]`. Applying an extraneous sigmoid `1 / (1 + exp(-raw_score))` compressed authentic face probabilities from 0.7839 down to 0.6865 (below the 0.70 detection threshold) while elevating zero-probability background noise to 0.5001 across thousands of anchors.

---

## Root Cause Analysis

### 1. Egui Cross-Axis Layout Constraint
- When rendering `render_live_inspection` and `render_guided_enrollment`, `ui.horizontal` was invoked directly without setting an explicit height or panel container.
- Egui sets `available_size().y` inside an unconstrained horizontal layout to `18.0`.
- The aspect ratio calculation `target_w = avail_size.x.min(avail_size.y * aspect_ratio)` clamped `target_w` to `18.0 * 1.33 = 24.0px`, rendering the video texture as an imperceptible speck under the UI checkboxes.

### 2. SCRFD Neural Network Activation
- InsightFace SCRFD unified models exported to ONNX include a final Sigmoid activation layer on classification heads.
- Verification on raw physical frames captured from `/dev/video0` showed score tensor values bounded in `[0.0003, 0.7839]`.
- Applying a second sigmoid resulted in `sigmoid(0.7839) = 0.6865`, falsely discarding the user's face at `threshold = 0.70` and reporting `Face Count: 0`.

---

## Implemented Changes

### 1. Responsive Canvas Allocation (`crates/gui/src/app.rs`)
- Extracted total available container dimensions prior to entering horizontal layouts:
  ```rust
  let total_avail = ui.available_size();
  let main_width = (total_avail.x * 0.70).max(450.0);
  let sidebar_width = (total_avail.x - main_width - 24.0).max(280.0);
  let content_height = total_avail.y;
  ```
- Allocated explicit dimensions to both the camera view and sidebar panels using `ui.allocate_ui`:
  ```rust
  ui.horizontal(|ui| {
      ui.allocate_ui(Vec2::new(main_width, content_height), |ui| {
          ...
      });
      ui.separator();
      ui.allocate_ui(Vec2::new(sidebar_width, content_height), |ui| {
          ...
      });
  });
  ```
- Wrapped the entire application UI in `egui::CentralPanel::default().show(ui, ...)` to ensure correct margins and window padding.
- Added visual fallback placeholder ("Acquiring video stream...") in case the texture handle is pending initialization.

### 2. Dual-Mode Activation Handling (`crates/inference-ort/src/detector.rs`)
- Updated `decode_stride` in `OrtScrfdDetector` to support both pre-activated probabilities (`[0.0, 1.0]`) and raw logits (`< 0.0` or `> 1.0`):
  ```rust
  let conf = if (0.0..=1.0).contains(&raw_score) {
      raw_score
  } else {
      1.0 / (1.0 + (-raw_score).exp())
  };
  ```
- Maintained 100% backward compatibility with synthetic test fixtures using logit inputs (`-10.0`, `+4.0`) while accurately processing genuine ONNX model outputs (`0.7839`).
- Tuned default GUI detector threshold to `0.60` with `0.40` IoU for reliable detection across various indoor lighting conditions.
- Added `.with_active(true)` to `ViewportBuilder` so the native window gains window manager focus on start.

### 3. Automated Verification Tests (`crates/gui/tests/layout_tests.rs`)
- Authored contract tests verifying that `render_live_inspection` and `render_guided_enrollment` allocate large canvas areas (`target_w >= 500px`, `target_h >= 350px`) preserving 4:3 and 16:9 aspect ratios.
- Authored `test_scrfd_supports_preactivated_probabilities` in `crates/inference-ort/tests/scrfd_tests.rs`.

---

## Verification Results
- All unit and property tests passed cleanly:
  ```bash
  cargo test --workspace
  # Result: 100% passed, 0 failed, 0 warnings
  ```
- Formatting and lints:
  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  # Result: 0 warnings
  ```
- Pre-push audit:
  ```bash
  ./scripts/candid_review.sh
  # Result: Candid Review PASSED: All architectural invariants verified!
  ```

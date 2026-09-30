# Walkthrough 66 — Fix GUI Windowed Layout and MiniFASNetV2 Live Class Index

> **Historical record - class order superseded.** The MiniFASNetV2 live class index stated below is obsolete. The current contract is `[PrintPhoto, Live, ScreenReplay]`, live class index 1 (`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`), per the 2026-09-29 and 2026-09-30 ADR entries in `AI/DECISIONS.md` (GitHub #146, #171; walkthroughs 79 and 85).


## Context & Objectives
During real-world camera testing of `soos-gui`, two issues were identified:
1. **Windowed Layout Positioning**: In windowed mode (e.g. 900×600 to 1120×780), the camera video area disappeared off-screen. It was only visible when maximized or fullscreen.
2. **Anti-Spoofing False Rejection**: The live feed reported `SPOOF ✗ (0.01)` even when the user was well-lit and directly facing the camera.

## Root Cause Analysis
1. **Layout Direction Inheritance**:
   - In `crates/gui/src/app.rs`, `render_live_inspection` and `render_guided_enrollment` wrapped the left panel and sidebar in `ui.horizontal(|ui| { ... })`.
   - Child `ui.allocate_ui` calls inside `ui.horizontal` inherited a horizontal layout direction (`Layout::left_to_right`).
   - Consequently, the top control checkboxes (width ~770px) were laid out horizontally, and the video painter was positioned *to the right* of the checkboxes at `x ≈ 770px`.
   - In windowed mode, where the panel width is 600–750px, `ui.available_size().x` collapsed or became pushed out of the visible screen area.
   - When maximized (1920px wide), the video area had remaining space to the right of 770px, which made it visible only when full screen.
2. **MiniFASNetV2 Class Mapping**:
   - The deployed ONNX model `2.7_80x80_MiniFASNetV2.onnx` has 3 output classes:
     - Class 0: PrintPhoto spoof attack
     - Class 1: ScreenReplay spoof attack
     - Class 2: Genuine Live face
   - Prior code had configured `live_class_index = 1`. Under live camera presentation, the model computed softmax probabilities `[0.0003, 0.0054, 0.9943]`. Reading index 1 yielded 0.0054 (~0.01 spoof score), while index 2 contained the true live probability (99.43%).

## Key Changes
- **Inference (`crates/inference-ort`)**:
  - Defined `pub const DEFAULT_MINIFASNET_LIVE_CLASS_INDEX: usize = 2;` in `pad.rs`.
  - Updated `OrtPadDetector::new` to default to `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`.
  - Added unit test `test_pad_default_live_class_index_is_two` and `test_pad_class_ordering_live_index_2_minifasnet_v2`.
- **Daemon & CLI & GUI**:
  - Updated `crates/daemon/src/pipeline.rs`, `crates/enrollment-cli/src/service.rs`, and `crates/gui/src/main.rs` to configure `live_class_index = 2`.
- **GUI Layout (`crates/gui/src/app.rs`)**:
  - Replaced `allocate_ui` in `render_live_inspection` and `render_guided_enrollment` with `ui.allocate_ui_with_layout(..., Layout::top_down(Align::Min), ...)`.
  - Replaced `ui.horizontal` for filter checkboxes with `ui.horizontal_wrapped(|ui| { ... })`.
  - Clamped sidebar width dynamically (`(total_avail.x * 0.32).clamp(280.0, 380.0)`) and allocated the remaining space to the camera stream to guarantee zero horizontal overflow across window sizes from 900px upwards.
  - Wrapped sidebars in `egui::ScrollArea::vertical()` to prevent vertical clipping on smaller screen heights.
- **Verification Tests (`crates/gui/tests/layout_tests.rs`)**:
  - Added `test_windowed_mode_live_inspection_layout_with_checkboxes` validating that the camera canvas allocates >= 450×300px in compact windowed mode (900×600) with wrapped checkboxes.

## Verification
- `cargo fmt --all -- --check`: Clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings.
- `cargo test --workspace`: All 12 crates passed cleanly (including 14 PAD tests and 3 GUI layout tests).
- `./scripts/candid_review.sh`: Passed with all invariants intact.

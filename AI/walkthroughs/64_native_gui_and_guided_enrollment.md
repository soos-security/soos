# Walkthrough 64 — Native Desktop GUI and Apple FaceID-Style Guided Enrollment

## 1. Executive Summary

This walkthrough details the design, implementation, and verification of **`soos-gui`**, a native Linux desktop application built with `eframe` (egui + glow/OpenGL) for Wayland and X11, alongside an Apple FaceID / Samsung-style multi-pose guided enrollment workflow and real-time model diagnostic visualization.

Prior to this work, diagnostic feedback was limited to static HTML snapshots. This implementation delivers:
1. **Live Camera Feed & Authentic Model Overlays**: Shows real-time 30 FPS camera frames with authentic bounding boxes (SCRFD), 5-point facial landmarks, MiniFASNetV2 anti-spoofing (PAD) liveness confidence, and ArcFace 112×112 aligned face crop insets.
2. **Apple FaceID-Style Guided Enrollment**: Replaces single-snapshot enrollment with an interactive multi-pose session (Frontal, Turn Left, Turn Right, Tilt Up) that verifies centering, distance, stability, and liveness to fuse a composite 512D reference vector for lightning-fast PAM unlocking.
3. **Biometric Profile Management**: Allows listing enrolled users, testing live 1-to-1 face matching with an interactive similarity bar and PAM verdict, and securely shredding stored templates.
4. **V4L2 Camera Negotiation Fix**: Hardens the camera driver to capture actual negotiated sensor dimensions (handling hardware cameras that return 640×360 instead of 640×480).

---

## 2. Architectural Components

### 2.1 Crate Scaffolding (`crates/gui`)
- **Location**: `crates/gui/`
- **Binary**: `soos-gui`
- **Dependencies**: `eframe` (0.35 with glow backend), `egui`, `image`, `arc-swap`, `clap`, and workspace crates (`soos-camera-v4l`, `soos-vision`, `soos-inference-ort`, `soos-biometric-store`, `soos-enrollment-cli`).
- **Safety Invariant**: Enforces `#![forbid(unsafe_code)]` with zero `unwrap()`, `expect()`, or `panic!()` in production code.

### 2.2 Decoupled Worker Architecture (`crates/gui/src/worker.rs`)
- Because ONNX inference (SCRFD detection + MiniFASNetV2 PAD + ArcFace embedding) takes 80–140ms on CPU, running inference directly in the GUI event loop would drop GUI frame rates to 7–10 FPS.
- `soos-gui` uses a dedicated background thread (`spawn_vision_worker`):
  - Fetches latest frames from `CameraManager` lock-free via `ArcSwapOption`.
  - Executes `pipeline.analyze_frame(&frame)` without short-circuiting.
  - Updates atomic feedback slots (`WorkerSharedInput`) and publishes `LatestFrameData`.
  - Triggers `egui_ctx.request_repaint()` to maintain smooth UI animation at 30–60 FPS.

### 2.3 Head Pose Estimation (`crates/vision/src/pose.rs`)
- Computes roll, yaw, and pitch from the 5 canonical facial landmarks:
  - **Roll**: Angle between left eye and right eye.
  - **Yaw**: Ratio of horizontal nose-tip position relative to interocular distance.
  - **Pitch**: Ratio of vertical nose-tip position between the eye line and mouth line.
- Tested and verified against canonical landmarks in `crates/vision/tests/pose_tests.rs`.

### 2.4 Guided Enrollment State Machine (`crates/enrollment-cli/src/guided_enrollment.rs`)
- State machine phases:
  1. `WaitingForFace`: Prompts the user to center their face in the oval reticle.
  2. `Frontal`: Collects stable frontal samples while verifying PAD liveness.
  3. `TurnLeft`: Prompts user to turn head slightly left (yaw <= -12°).
  4. `TurnRight`: Prompts user to turn head slightly right (yaw >= +12°).
  5. `TiltUp`: Prompts user to tilt head slightly upward (pitch >= +10°).
  6. `Completed`: Fuses all multi-angle embeddings into a single composite template:
     $$\mathbf{v}_{\text{composite}} = \frac{\sum_i \mathbf{v}_i}{\|\sum_i \mathbf{v}_i\|_2}$$

---

## 3. UI Tabs & User Experience

```
┌────────────────────────────────────────────────────────────────────────┐
│  soos Biometric Manager — Linux PAM Face Verification                  │
├────────────────────────────────────────────────────────────────────────┤
│ [ Tab 1: Live Diagnostics ] [ Tab 2: Guided Enrollment ] [ Tab 3: Profiles ]
├──────────────────────────────────────┬─────────────────────────────────┤
│                                      │ Diagnostics Panel               │
│                                      │ • Camera: /dev/video0 (640x360) │
│       LIVE CAMERA FEED               │ • FPS: 28.4                     │
│    (30 FPS with overlays)            │ • Inference Latency: 98 ms      │
│                                      │                                 │
│    ┌───────────────┐                 │ SCRFD Detection:                │
│    │  [Live Face]  │  Landmarks (5)  │ • Confidence: 0.94              │
│    │               │  PAD: 0.98 LIVE │ • Bounding Box: [140, 60, ...]  │
│    └───────────────┘                 │                                 │
│                                      │ MiniFASNetV2 Anti-Spoofing:     │
│   Aligned 112x112 Crop Inset         │ • PAD Score: 0.985 (LIVE)       │
│                                      │                                 │
│                                      │ Head Pose:                      │
│                                      │ • Yaw: -1.2° | Pitch: +2.1°     │
└──────────────────────────────────────┴─────────────────────────────────┘
```

1. **Tab 1: Live Inspection**:
   - Real-time video canvas with toggleable overlays: SCRFD Bounding Box, 5 Facial Landmarks, PAD Crop bounds, Aligned Face Crop inset.
   - Comprehensive model latency and frame rate statistics.
   - Live 1-to-1 PAM unlock test bar against enrolled templates.
2. **Tab 2: Guided Enrollment**:
   - Apple FaceID-style oval guide reticle with animated progress arc.
   - Instruction cards for each pose step with visual feedback (centering, stability, distance).
   - "Save & Encrypt Biometric Template" button on completion.
3. **Tab 3: Profiles & Security**:
   - Enrolled user table with UID, creation timestamp, and template attributes.
   - Live PAM match simulator.
   - One-click secure profile shredding (cryptographic zeroization before unlinking).

---

## 4. Verification & Testing

### 4.1 Automated Workspace Tests
All 14 workspace crates passed 100% of unit, property, and invariant tests:
```bash
cargo test --workspace
```
- `crates/vision/tests/pose_tests.rs`: 4/4 passed (frontal, roll, yaw left/right, pitch up/down).
- `crates/enrollment-cli/tests/guided_enrollment_tests.rs`: 1/1 passed (full state machine transition and composite fusion).
- `tests/invariants/src/lib.rs`: 21/21 passed (including `#![forbid(unsafe_code)]` verification for `crates/gui`).

### 4.2 Quality Gate & Compiler Profile
- `cargo fmt -- --check`: Passed cleanly.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Passed cleanly with zero warnings or errors.
- `target/debug/soos-gui --help`: Executed and verified CLI arguments and defaults.

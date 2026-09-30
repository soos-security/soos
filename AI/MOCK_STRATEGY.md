# Mocking & Simulation Strategy (Development Environment)

Access to `/dev/video0` (webcam) is not always available in headless environments (CI pipelines, Docker containers, laptops without webcams). To enable smooth development and automated testing without hardware friction, the project implements a comprehensive mocking architecture.

## 1. Cargo Feature Flag
The camera crate (`camera-v4l`) exposes a conditional compilation feature in its `Cargo.toml`:
`[features] mock-camera = []`

## 2. The "Dummy Camera Driver"
When the `mock-camera` feature is enabled:
- The daemon bypasses kernel `v4l` device binding.
- It instantiates a `MockCameraManager` struct that:
  - Generates static test frames (e.g. 640x480 pixel arrays) simulating live video capture.
  - Slices frames with simulated monotonic timestamps to validate latency constraints (< 150ms).
  - Can simulate device disconnections, frame corruption, or starvation.

## 3. Vision Test Fixtures
No image or tensor files are committed. The shared fixture module `tests/fixtures/mod.rs` (included by
`#[path]` from the `vision`, `daemon` and `enrollment-cli` tests) generates everything in code:
- `synthetic`: solid RGB24, YUYV and Grey frames built from `soos_camera_v4l::Frame`.
- `embeddings`: small pre-computed unit vectors for two subjects, used for cosine matching logic.
- `pad`: synthetic live, printed-photo and screen-replay frames for the PAD wiring tests.
- `onnx`: a deterministic minimal `Identity` ONNX model so ORT session factories run in CI.

Face detection (SCRFD), 112x112 alignment, MiniFASNetV2 PAD and ArcFace 512D embeddings are exercised
through the mock backends above. Real-model evidence lives in
`crates/inference-ort/tests/pad_real_model_tests.rs` and `embedding_real_model_tests.rs`, which skip
cleanly when `/var/lib/soos/models` is absent.
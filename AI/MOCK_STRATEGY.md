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
To validate the AI inference pipeline without a live camera, static image fixtures are placed under `tests/fixtures/`:
- Contains sample facial images (JPEG / serialized tensor arrays) of known and unknown subjects.
- Unit and integration tests in the `vision` crate ingest these fixtures to validate:
  - Face detection (NMS thresholds)
  - 5-point landmark affine transformation (112x112 alignment)
  - MobileFaceNet embedding generation and cosine distance matching
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
Shared, generated (never recorded) fixtures live in the dev-only workspace crate
`soos-test-fixtures` (`tests/fixtures/Cargo.toml`, library root `tests/fixtures/mod.rs`,
GitHub #241). It contains no facial image, no serialized tensor and no embedding:
- `synthetic`: solid RGB24, YUYV and Grey frames of any size;
- `pad`: synthetic live-face, printed-photo and screen-replay presentations for the PAD tests;
- `onnx`: `minimal_identity_model()`, a hand-encoded ONNX `Identity` graph so the ORT wiring
  of the daemon and enrollment CLI runs in CI without a model file.

Every test consumes `soos-test-fixtures` through `[dev-dependencies]`; it is never a normal
dependency. No `#[path]` include of `tests/fixtures/mod.rs` remains: the legacy list
`LEGACY_PATH_INCLUDES` in `tests/invariants/src/fixtures_contract.rs` is empty and any new
include fails that invariant. Face embeddings have the dimension of the shipped model
(`soos_inference_ort::EMBEDDING_DIMENSION`, SFace, `AI/DECISIONS.md`; `MockEmbeddingExtractor`
defaults to it); embedding tests build their vectors locally. Real face captures
(detection, alignment, matching accuracy) are covered only by the physical suite (`tests/physical/`).

Face detection (SCRFD), 112x112 alignment, MiniFASNetV2 PAD and SFace embeddings are exercised
through the mock backends above. Real-model evidence lives in
`crates/inference-ort/tests/pad_real_model_tests.rs` and `embedding_real_model_tests.rs`, which skip
cleanly when `/var/lib/soos/models` is absent.

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

## Presence Auto-Unlock Doubles (GitHub #323)
The presence worker (`soos_daemon::presence::worker::PresenceWorker`) is generic over its
logind access, display probe and account guard, so every contract test runs without D-Bus,
without a real lock screen and without reading `/etc` or `/run`
(`crates/daemon/tests/common/mod.rs`):
- `MockPresenceLogind`: implements `PresenceLogind` with a scripted `seat_sessions` snapshot,
  one-shot `session_state` replies, settable `lid_closed` / `unlock_session` results,
  never-resolving calls (`hang_*`, to prove the 500 ms call bound), per-call counters, the
  recorded `unlock_session` and `session_state` IDs, and hooks that shift the test clock or
  inject faults between the `Allow` and the unlock;
- `TestDisplay` (settable `DisplayState`) instead of `SysfsDisplayProbe`;
- `StaticAccountGuard` / `ScriptedAccountGuard` (per-call answers, blocking delays, call
  counter) instead of `SystemAccountGuard`, which is itself tested on tempdir trees
  (`faillock.conf`, `pam.d/`, byte-built `struct tally` files, `shadow`) with an injected
  wall clock;
- `SpyCamera` over `MockCameraManager` (counts `notify_activity` and captures, can stay not
  ready, re-stamps frames with the test clock) and `CountingExtractor` over the mock vision
  backends (counts inferences, runs a hook that registers PAM demand or panics);
- `PresenceWorker::tick()` with `with_clock_fn` (a `fn` pointer over `CLOCK_MONOTONIC` plus a
  static offset) drives grace, interval and backoff deterministically.
The production `ZbusLogind` is exercised only on hardware (`tests/physical/`); its pure reply
mapping `session_state_from_properties` is tested with hand-built `zvariant::OwnedValue` maps.

## Remote Companion Doubles (GitHub #339)
`soos-remote` is generic over its logind access (`soos_remote::logind::SessionSource`) and
takes its Unix clock through `ServerState::with_unix_clock`, so the whole end-to-end suite
(`crates/remote/tests/server_tests.rs`) runs without D-Bus, without Tailscale and without a
real socket directory:
- `MockSource`: implements `SessionSource` with a settable `own_sessions` answer (snapshotted
  when a call starts, which models a slow read), a settable `lock_session` result,
  `hold_next(n)` / `hold_lock_next(n)` gates that block the next calls until `release()` (to
  prove the 1500 ms snapshot and 2000 ms lock-flow deadlines), per-call counters and the
  recorded `lock_session` ids and uids;
- `TestClock`: the paused tokio clock plus a settable offset, injected as the `checked_unix_ms`
  source (R3-1), with `shift_ms(-3_600_000)` for the backward-clock test;
- `FrozenClock`: one live `spawn_blocking` task that inhibits tokio's auto-advance, so virtual
  time moves only through `tokio::time::advance` and the 5 s head timer, the 15 s keep-alive
  and the 30 min stream lifetime fire exactly when the test says;
- `LogCapture`: a `tracing_subscriber::fmt` writer at `TRACE` that proves no identity, `Host`,
  path, header value or session id is ever logged;
- the HTTP client is raw bytes over `tokio::net::UnixStream` against a listener bound in a
  `tempfile::TempDir`.
The production `ZbusSessionSource` is exercised only on the owner's host; its pure reply
mapping `session_props_from_properties` is tested with hand-built `zvariant::OwnedValue` maps,
and the socket helpers over `TempDir` (symlink, regular file and foreign-uid cases).

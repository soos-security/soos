# Walkthrough 184 — `soos-gui` Frame Pacing and Stable egui Ids

- **Date**: 2026-10-06
- **Issue**: no backlog id (owner request after the brand redesign: low frame rate in the live
  view, egui "changed id between passes" warnings) — **Branch**: `fix/gui-fps-and-egui-ids`
- **Matrix criteria**: GFP1–GFP3 (component `gui-frame-pacing`)

## 1. Context & Objectives

With the real camera (daemon preview), the live view analyzed about 27 frames per second in a
release build and about 6 in a debug build, against a 30 fps camera. A debug build also logged a
burst of 35 "Widget rect changed id between passes" warnings a few seconds after startup.
Objectives: reach the camera rate in release, make `cargo run -p soos-gui` usable, remove the
id warnings, without changing any behavior or security property.

## 2. Architect Design

Measured with temporary per-second logs (removed before commit; frames, embeddings and scores
were never logged):

| Build | FPS | Analysis per frame | Worker tail | Fixed sleep |
|---|---|---|---|---|
| release | ~27 | ~25 ms | <1 ms | 10 ms |
| debug | ~6.5 | ~103 ms | ~8 ms | 10 ms |

- **Vision worker pacing** (`worker.rs`): the loop slept 10 ms after every iteration, also right
  after an analyzed frame, so a frame cost analysis + 10 ms > 33 ms and frames were skipped. New
  `WORKER_IDLE_POLL` (5 ms) and `worker_idle_delay(analyzed_frame)`: no sleep after a frame,
  5 ms otherwise.
- **Preview cadence** (`ipc_camera.rs`): the preview worker slept the full 33 ms after each
  reply. New `PollStep::Cadence` and `frame_cadence_delay(elapsed)` keep 33 ms from request to
  request (30 per second, below the daemon default of 40). Back-offs are unchanged.
- **Debug profile** (`Cargo.toml`): per-package `opt-level = 3` for `soos-vision`,
  `soos-inference-ort`, `jpeg-decoder` and `zeroize` (the volatile wipe of each ~1 MB frame
  buffer was a large share of the debug cost). Overrides for `soos-camera-v4l` and
  `soos-protocol` were measured and brought nothing, so they were not kept.
- **egui ids** (`widgets.rs`): the right column measures its card on one pass; a second egui
  pass in the same frame can read the new height and show or hide a tile above the card. The
  card content scope used the parent's auto-id counter (`UiBuilder::id_salt` still mixes it in
  with egui 0.35), so every widget inside the card changed id. New `card_scope_id` gives
  `card_in_rect` and `card_in_rect_with_footer` an explicit id (`UiBuilder::id`) built from the
  parent's stable id and the card's `id_salt`.

## 3. Plan Evaluation

No separate plan-evaluator report was produced for this change. The design was checked against
the measurements above and the egui 0.35 source (`Ui::new_child`).

## 4. Tester Contract

New files only (red first, existing tests untouched):

- `crates/gui/tests/ipc_preview_cadence_tests.rs` (GFP1): failed to compile before the change
  (`frame_cadence_delay` missing, `PREVIEW_POLL_INTERVAL` private).
- `crates/gui/tests/worker_pacing_tests.rs` (GFP2): failed to compile before the change.
- `crates/gui/tests/card_id_stability_tests.rs` (GFP3): compiled and failed (ids differed) on the
  previous code and with `UiBuilder::id_salt`; passes with the explicit id.

## 5. Auditor Constraints

- No new dependency, no `unsafe`, no `unwrap` / `expect` in production code.
- No logging of frames, embeddings, scores or credentials; the timing logs were temporary.
- The UI thread still never blocks; the worker loop still checks `running` every iteration and
  still withdraws a frozen frame when the source fails.
- The preview request rate stays at most 30 per second (daemon limit 40).
- `[profile.dev]` keeps `overflow-checks = true` (invariant 7); the release profile is unchanged.

## 6. Implementation

- `crates/gui/src/worker.rs`: `WORKER_IDLE_POLL`, `worker_idle_delay`, `analyzed_frame` flag.
- `crates/gui/src/ipc_camera.rs`: public `PREVIEW_POLL_INTERVAL`, `frame_cadence_delay`,
  `PollStep::Cadence`, request start time in the polling loop.
- `crates/gui/src/widgets.rs`: `card_scope_id` used by both card helpers.
- `Cargo.toml`: four `[profile.dev.package.*]` overrides.
- Docs: `Docs/GUI_APPLICATION.md` §1 (frame pacing, debug builds), §4, §5.4;
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` profile table; `AI/VERIFICATION_MATRIX.md` GFP1–GFP3.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) and Layer 2 (`scripts/candid_subagent.sh`, a context-free
reviewer on the raw diff, report `AI/candid_review_report.md`) are run before `save.sh`.

## 8. Verification Results

- Release build, real camera through `soos-daemon`: 30 analyzed frames per second (was ~27).
- Debug build: ~23 (was ~6.5).
- Debug build, three runs of 20–30 s: 0 "changed id between passes" warnings (was 35 on one
  frame).
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: 0 failures.

## 9. Known Limitations / Follow-ups

- Polling a 30 fps source at 30 Hz can still fetch the same frame twice now and then; the
  release build measured 26–30 per second over one-second windows.
- A debug build stays below the camera rate (~23): the remaining cost is unoptimized
  `soos-gui` code, which is kept debuggable on purpose.

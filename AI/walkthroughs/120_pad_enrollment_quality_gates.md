# Walkthrough 120 — Pre-PAD Face Quality Gate and Session-Level Enrollment Liveness

- **Date**: 2026-09-30
- **Issues**: GitHub #217 ([PAD-12] guided enrollment gates each sample on single-frame liveness),
  GitHub #218 ([PAD-13] no face-size or image-quality gate before PAD)
- **Branch**: `fix/p2-pad-enrollment-quality-gates`
- **Matrix criteria**: PQG1–PQG8 (new, verified), PQG9 (pending, hardware calibration)
- **ADR**: 2026-09-30 "Pre-PAD Face Quality Gate And Session-Level Enrollment Liveness"

---

## 1. Context

- **PAD-13**: `VisionPipeline::process_frame` only checked the detection score. A 30 px face was
  expanded 2.7x and upsampled into the 80x80 MiniFASNet input, where the liveness score is
  unreliable in both directions (phone at arm's length, print seen from far).
- **PAD-12**: the GUI computed `is_live` from the current frame only, and
  `GuidedEnrollmentSession::process_sample` simply skipped a spoof frame. The next frame that passed
  PAD was sampled, so a presentation that passed PAD only now and then could still build a template.

## 2. Specification

### Vision (`crates/vision/src/quality.rs`, `pipeline.rs`, `error.rs`)

- `VisionPipelineConfig::min_face_width_px` (default `DEFAULT_MIN_FACE_WIDTH_PX` = 48 px) is
  compared with the smaller bounding-box side. Below it: `VisionError::FaceTooSmall`.
- `VisionPipelineConfig::min_pad_crop_sharpness` (default `DEFAULT_MIN_PAD_CROP_SHARPNESS` = 0.0,
  disabled) is compared with `laplacian_variance` of the 80x80 PAD crop. Below it:
  `VisionError::FaceBlurred`.
- Both checks run before the PAD model and the embedding extractor. Every comparison fails closed
  (NaN size, sharpness or threshold rejects).
- `analyze_frame` reports `VisionAnalysis::quality_rejection` and then skips PAD, alignment and
  embedding.
- The daemon maps both errors to `FrameEvaluation::no_face()`. No new `ReasonClass`, so the
  protocol does not change.

### Guided enrollment (`crates/enrollment-cli/src/guided_enrollment.rs`)

- `LivenessPolicy { min_consecutive_live_frames, max_spoof_events }`, clamped to `1..=32`.
  `strict()` = (3, 3), `single_frame()` = (1, 3).
- `with_liveness_policy(target, policy)`; `new(target)` uses `single_frame()`.
- A spoof frame resets the live streak and discards the current step's samples. The
  `max_spoof_events`-th spoof frame aborts the session: all samples are cleared,
  `SessionAborted` is returned from then on and `compute_composite_embedding` fails.
- `interrupt_liveness_streak()` breaks the streak without counting a spoof.
- New feedback variants: `SessionAborted`, `FaceQualityTooLow` (the second is set by the GUI).

### GUI (`crates/gui/src/worker.rs`, `app.rs`)

- `new_guided_enrollment_session()` creates the session with `LivenessPolicy::strict()`.
- `feed_guided_enrollment()` now contains the per-frame logic that used to be inline in the
  worker loop. A quality rejection gives `FaceQualityTooLow`. A face with no PAD verdict gives
  `PromptHoldStill`. A frame with no face reports nothing. In all three cases the streak is broken
  and no spoof is counted. `is_live` also rejects a NaN score.

## 3. Red Evidence (before the implementation)

- `cargo test -p soos-daemon --test face_quality_gate_tests`:
  `test_218_too_small_face_returns_deny_no_face` failed with `left: Allow, right: Deny`. A 20 px
  face was authenticated.
- `cargo test -p soos-vision --test face_quality_gate_tests` did not compile (15 errors):
  `soos_vision::quality` was unresolved, there was no `min_face_width_px` or
  `min_pad_crop_sharpness` field, no `FaceTooSmall` or `FaceBlurred` variant and no
  `quality_rejection` field.
- `cargo test -p soos-enrollment-cli --test guided_enrollment_liveness_tests` did not compile
  (16 errors): `LivenessPolicy`, `with_liveness_policy`, `is_aborted`, `spoof_events`,
  `interrupt_liveness_streak` and `SessionAborted` were missing.
- `crates/gui/tests/guided_liveness_feed_tests.rs` depends on `feed_guided_enrollment`,
  `new_guided_enrollment_session` and `FaceQualityTooLow`, which did not exist.

## 4. Green Evidence

- `face_quality_gate_tests` (vision): 13 passed. `face_quality_gate_tests` (daemon): 2 passed.
  `guided_enrollment_liveness_tests`: 8 passed. `guided_liveness_feed_tests`: 5 passed.
- The existing `guided_enrollment_tests` and `guided_enrollment_consistency_tests` pass unchanged.
  So do all the existing vision, daemon and enrollment tests: the smallest box that an existing test
  expects to pass is 60 px, and every synthetic frame is flat, which the disabled sharpness floor
  accepts.
- Full gate: `cargo fmt --check`, `cargo clippy --locked --workspace --all-targets
  --all-features -D warnings` and `cargo test --locked --workspace --all-targets --all-features
  --no-fail-fast` are clean. `./scripts/candid_review.sh` passes.

## 5. Audit Notes

- No `unwrap` or `expect` in production code. `laplacian_variance` validates the buffer length
  with checked multiplication before it indexes, and its allocation is proportional to the input
  (the 80x80 crop in the pipeline).
- The new errors carry only geometry and sharpness numbers, never pixels or embeddings.
- `#![forbid(unsafe_code)]` still holds in `soos-vision`, `soos-enrollment-cli` and `soos-gui`.
- The PAM module is not touched. A quality-gated face can only lead to `Deny` or `NoFace`, never
  to `Allow`.

## 6. Limitations and Follow-ups

- The sharpness floor is disabled by default (PQG9). It must be calibrated on real camera captures
  (genuine faces compared with distant or blurred presentations) before a positive default is set.
- `min_face_width_px` and `min_pad_crop_sharpness` cannot yet be set from the daemon
  configuration file. Only code or tests can set them.
- `GuidedEnrollmentSession::new` keeps the single-frame gate because the existing contract
  `test_guided_enrollment_full_workflow` expects a sample on the first live frame. Making `new`
  strict needs the test change that the report proposes.
- The non-guided `soos-enroll enroll` path already aborts on any PAD failure. A too-small or
  blurred frame now also aborts it, with an explicit error.

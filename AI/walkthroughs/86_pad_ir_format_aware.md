# Walkthrough 86 — Format-Aware PAD and Short-Term IR Liveness Policy

- **Date**: 2026-09-30
- **Issue**: Review finding PAD-03 (GitHub #169) — **Branch**: `fix/pad-ir-format-aware`
- **Matrix criteria**: PIR1, PIR2, PIR3, PIR4, PIR5
- **ADR**: `AI/DECISIONS.md` entry 2026-09-30 "Format-Aware PAD and Short-Term IR Liveness Policy"

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, finding PAD-03, severity
MAJOR, verifier CONFIRMED) established a design fact, re-verified in code before the change:

- `crates/camera-v4l/src/config.rs`: `CameraConfig::default()` sets
  `sensor_preference: SensorPreference::PreferIr` (the enrollment CLI defaults to `PreferIr` too).
- `crates/vision/src/color.rs`: `PixelFormat::Grey` is converted by replicating each grey byte into
  R, G and B.
- `crates/vision/src/pipeline.rs`: `frame.format` was only used by `convert_to_rgb`; the PAD crop was
  handed to `PadDetector::evaluate_liveness` with no notion of the source format and compared against
  the RGB `pad_threshold`.
- MiniFASNetV2 is trained on colour captures, so replicated-grey NIR input is out-of-distribution and
  its scores are uncalibrated. `grep -rni emitter crates Docs AI scripts` returned no hits.

The verifier explicitly downgraded the causal claim (that this explains observed false-live and
false-spoof readings) to speculation; the finding is an unmitigated domain-shift risk on the default
sensor, not a demonstrated misbehavior.

Objectives:

1. Make the pipeline format-aware so that a `Grey` frame never silently takes the RGB PAD path.
2. Implement a documented, conservative, fail-closed short-term IR liveness policy.
3. Document IR-emitter requirements.
4. Do not invent calibration numbers and do not claim measured metrics; leave hardware calibration
   as an explicit follow-up (GitHub #172 / PAD-06).

## 2. Architect Design

### Scope & blast radius

- `crates/vision/src/ir_liveness.rs` (new), `crates/vision/src/lib.rs` (re-exports),
  `crates/vision/src/error.rs` (new variant), `crates/vision/src/pipeline.rs`.
- `crates/daemon/src/dispatcher.rs`: one new match arm.
- The `PadDetector` trait, `OrtPadDetector`, the tensor layout (NCHW BGR 1×3×80×80, `x/255`) and the
  live class index are **unchanged** (no conflict with #146 / #172). `PipelineOutput` is unchanged.
  No daemon TOML key is added (the concurrent #170 change owns `pad_threshold` validation).

### Types and constants

| Item | Value / semantics |
|---|---|
| `PadInputModality` | `Color` / `Monochrome`; `from_pixel_format(Grey) = Monochrome`, all other formats `Color` |
| `DEFAULT_IR_PAD_THRESHOLD` | `0.95` — conservative, **uncalibrated** floor, stricter than the RGB default 0.85 |
| `IR_MIN_MEAN_LUMA` / `IR_MAX_MEAN_LUMA` | `20.0` / `235.0` (0–255 luma) |
| `IR_MIN_LUMA_STDDEV` | `10.0` |
| `IR_MIN_TEXTURE_ENERGY` | `2.0` (mean abs. horizontal + mean abs. vertical neighbour difference) |
| `IrGateRejection` | `Underexposed`, `Overexposed`, `LowContrast`, `LowTexture`, `InvalidCrop` |
| `IrCropStatistics` | `mean_luma`, `luma_stddev`, `texture_energy` (aggregates only, no pixels kept) |
| `ir_crop_statistics(crop, w, h)` | `Result<IrCropStatistics, VisionError>`; zero dims / overflow / size mismatch are errors |
| `evaluate_ir_gate(crop, w, h)` | `Result<(), IrGateRejection>`; order exposure → contrast → texture; malformed ⇒ `InvalidCrop` |
| `VisionPipelineConfig::ir_pad_threshold` | new field, default `DEFAULT_IR_PAD_THRESHOLD` |
| `VisionPipelineConfig::effective_pad_threshold(modality)` | `Color ⇒ pad_threshold`, `Monochrome ⇒ max(pad_threshold, ir_pad_threshold)` |
| `VisionError::IrLivenessGateFailed { reason }` | gate rejection, treated as a presentation attack |

### Decision flow for the PAD crop (`VisionPipeline::evaluate_pad_crop`)

1. `modality = PadInputModality::from_pixel_format(frame.format)`.
2. Monochrome only: `evaluate_ir_gate` on the 80×80 crop; a failure returns
   `IrLivenessGateFailed` and the RGB-trained model is **not** consulted.
3. Model result must satisfy `is_live && score.is_finite() && score >= effective_pad_threshold`,
   otherwise `PadFailed { score, threshold }` (threshold = the effective one).

Rationale for the gate (NIR physics from the review): LCD/OLED screens emit almost no 850/940 nm
light, so a replay under IR appears as a dark or flat rectangle; a missing or blocked emitter yields
an unlit crop. The gate bounds are loose sanity limits that only reject clearly degenerate crops;
they never grant liveness on their own.

### Invariants affected

- Fail-closed: every uncertain case (malformed crop, non-finite statistics, NaN score or threshold,
  gate failure) rejects; no error is converted into success.
- Never-looser: the IR threshold can only be equal to or stricter than `pad_threshold`.
- Colour path semantics are unchanged except that a NaN score/threshold now rejects instead of
  passing (`score < NaN` used to be `false`), which is strictly safer.
- `#![forbid(unsafe_code)]` in `soos-vision` untouched; no new dependency.

## 3. Tests First (Red Phase)

New file `crates/vision/tests/ir_pad_policy_tests.rs` (23 tests, PIR1–PIR4) and two daemon
integration tests in `crates/daemon/tests/pipeline_integration_tests.rs` (PIR5). The daemon fixture
gained `TestPipelineFixture::new_with_format(enroll, rate_limit, format)`; the existing `new`
delegates to it with `PixelFormat::Rgb24`, so every existing test keeps its exact behavior.

Red evidence on the base commit (`2e15447`):

- `cargo test -p soos-vision --all-features --test ir_pad_policy_tests` did not compile:
  `no variant named IrLivenessGateFailed found for enum VisionError` (×4),
  `no method named effective_pad_threshold` (×4), `no field ir_pad_threshold` (×2), plus the missing
  `ir_liveness` exports.
- `cargo test -p soos-daemon --all-features --test pipeline_integration_tests test_169`:
  `test_169_grey_frames_use_stricter_ir_pad_threshold ... FAILED` —
  `Grey frames must be scored against the IR threshold: reason=FaceMatch` (`left: Allow`), i.e. a Grey
  capture scored 0.90 by the RGB-trained model authorized through the RGB threshold.
  `test_169_grey_frames_above_ir_threshold_allow` passed (usability guard, green before and after).

## 4. Audit (Phase 3)

1. No `unwrap`/`expect`/`panic` in production code; the only indexing in `ir_liveness.rs` is over a
   buffer whose length is checked against `width * height * 3` with `checked_mul`; the module-level
   `allow` is scoped with a `reason`.
2. Bounded work: the gate runs on the fixed-size PAD crop (80×80 by default); one `Vec<f64>` of
   `w*h` luma values, freed immediately.
3. No sensitive data exposure: `IrLivenessGateFailed` carries only the rejection reason; the daemon
   logs the reason at `debug` level, never pixel values, statistics or embeddings.
4. The daemon maps the new variant to `FrameEvaluation::spoof(0.0)` (request veto, `Deny`/`PadFailed`,
   password fallback). Without the arm it would have hit the generic `Unavailable`/`InternalError`
   arm — also fail-closed, but mis-classified.
5. The PAM module is untouched; no Tokio, no IPC change.

## 5. Implementation (Green Phase)

- `ir_liveness.rs`: modality, constants, statistics, gate (non-finite statistics ⇒ `InvalidCrop`).
- `pipeline.rs`: `process_frame` calls `evaluate_pad_crop(&pad_crop, frame.format)`;
  `analyze_frame` (GUI) calls `analyze_pad_crop`, which returns the raw model result for colour frames
  and a spoof `PadResult` for a Grey gate failure or sub-IR-threshold score.
- `error.rs`: `IrLivenessGateFailed { reason: IrGateRejection }`.
- `dispatcher.rs`: new arm mapping the gate failure to a spoof capture.

## 6. Documentation

- `Docs/VISION_CRATE.md`: step 8 of the orchestrator, new §2.4.1 (format-aware PAD policy), matrix row.
- `Docs/CAMERA_V4L_CRATE.md`: new "IR Sensors and Emitter Requirements" section; fixed prose drift in
  principle 9, which claimed "selects RGB by default" while the code default is `PreferIr`.
- `AI/ARCHITECTURE.md`: PAD model paragraph notes the Grey policy.
- `AI/DECISIONS.md`: ADR 2026-09-30.
- `AI/VERIFICATION_MATRIX.md`: component `pad-ir-format-aware`, rows PIR1–PIR5.

## 7. Quality Gate

```
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test   --locked --workspace --all-targets --all-features
./scripts/candid_review.sh
```

All green (see the branch report). One transient failure of
`soos-camera-v4l::error_recovery_tests` (`camera.is_ready()` timing under heavy parallel machine load,
crate untouched by this change, 50 ms readiness sleep at load average ~60) passed on isolated re-run; the full suite re-run with `--no-fail-fast` exited 0.

## 8. Follow-ups (require hardware or a user decision)

- **IR calibration** (GitHub #172 / PAD-06): capture live faces, prints and screen replays on the
  target IR sensor with the emitter active, then replace `DEFAULT_IR_PAD_THRESHOLD` and, if needed,
  the gate bounds with measured values (new ADR). No FAR/FRR figure is claimed until then.
- **Medium term**: run PAD on the RGB sibling sensor while matching on IR, or ship an IR-trained PAD
  model.
- ~~**Sensor-type flag**~~: resolved in the candid-review rework (section 9): `Frame` now carries
  the sensor type.
- **IR corpus measurement** (candid review finding 6): `test_real_pad_corpus_apcer_bpcer_under_ceilings`
  drives only `OrtPadDetector` at 0.85. Add an IR bona fide / attack corpus class evaluated through
  `VisionPipeline`'s PAD decision (IR gate + 0.95) before any IR APCER/BPCER is reported (#172).
- **Preview wire**: the daemon preview (`PreviewResponse`) carries the pixel format but not the
  sensor type, so `soos-gui` in IPC mode classifies IR frames by format only. Production decisions
  are unaffected (the daemon analyzes its own tagged frames) and IR nodes now negotiate `Grey`;
  carrying the sensor type on the wire needs a protocol version bump.
- **Configuration**: `ir_pad_threshold` is not exposed in the daemon TOML (it can only be stricter
  than `pad_threshold`); exposing it is deferred until the #170 threshold validation lands.

## 9. Candid Review Rework (2026-09-30, finding 2)

The review found that the IR policy keyed on `PixelFormat::Grey` only: with the default
`PreferIr`, an IR node is selected by its card name (for example `USB2.0 FHD UVC WebCam: USB2.0 I`),
but `negotiate_format` preferred YUYV/MJPEG over GREY, so such a node took the RGB PAD path at the
RGB threshold.

- **Architect**: `Frame` gains `sensor_type: SensorType` (default `Unknown` in `Frame::new`, set with
  `Frame::with_sensor_type`; no other constructor exists). `soos_camera_v4l::plan_capture(card,
  formats, config) -> CapturePlan { format, sensor_type }` classifies the opened node with the
  resolver's `classify_sensor`, prefers `Grey` on `Infrared` nodes under auto negotiation (an
  explicit format is still honored), and the capture loop stamps every frame.
  `PadInputModality::for_source(format, sensor)` returns `Monochrome` for any `Infrared` sensor and
  falls back to `from_pixel_format` otherwise; the pipeline uses `for_frame`.
- **Tester (red evidence)**: `crates/vision/tests/ir_sensor_policy_tests.rs` (8 tests) and
  `crates/camera-v4l/tests/ir_capture_plan_tests.rs` (6 tests) were run against signature stubs:
  6/8 and 5/6 failed on assertions, e.g. `test_ir_sensor_streaming_yuyv_flat_frame_is_rejected_by_ir_gate`
  (the flat YUYV IR frame was accepted on the colour path),
  `test_ir_sensor_streaming_yuyv_uses_stricter_ir_threshold` (`process_frame` returned `Ok` at 0.90),
  `test_infrared_sensor_is_monochrome_modality_for_every_pixel_format` (`left: Color`,
  `right: Monochrome`) and `test_ir_node_advertising_yuyv_and_grey_is_planned_as_grey_infrared`.
- **Auditor**: fail-closed only. `Unknown` keeps the previous format-based behavior (mock and IPC
  frames); an `Infrared` tag can only move a frame to the stricter policy. No new allocation, no
  panic path, no frame data logged (the sensor type is logged once at stream start).
- **Developer**: all tests green; the existing PIR1–PIR5 tests are unchanged. Matrix row PIR6.

# Walkthrough 118 — PAD Output Contract and Startup Self-Test

- **Date**: 2026-09-30
- **Issue**: GitHub #214 ([PAD-09] No startup validation of PAD output shape or live_class_index;
  out-of-range index silently yields 100% spoof)
- **Branch**: `fix/p2-pad-startup-validation`
- **Matrix criteria**: PSV1–PSV4 (new)
- **ADR**: 2026-09-30 "PAD Output Contract and Startup Self-Test"

---

## 1. Context

`OrtPadDetector::interpret_probabilities` read `p_live` with
`probs.get(live_class_index).copied().unwrap_or(0.0)`: an index outside the output vector was not
an error, it was a spoof verdict on every frame, with no log line pointing at PAD.
`evaluate_liveness` softmaxed the first output tensor whatever its length. Registry shape
attestation (`ModelMetadata::validate_session_shapes`) already compares the graph metadata with
the manifest `output_shapes`, but symbolic graph dimensions are wildcards and nothing ever
checked the *runtime* output length or the live class index against it.

A second, fail-open consequence surfaced while writing the tests: a single-logit head gives
softmax `[1.0]`, which the single-output branch accepted as live on every frame.

## 2. Specification

`crates/inference-ort/src/pad.rs`:

| Item | Contract |
|---|---|
| `MINIFASNET_CLASS_COUNT = 3` | Required logit count of the MiniFASNetV2 head |
| `validate_pad_output_contract(len, index, expected)` | `PadFailed` unless `len == expected` and `index < len` |
| `pad_class_count_from_manifest(output_shapes)` | `[]` gives 3 (legacy), `[[.., 3]]` gives 3; any other count, rank 0, or several outputs is `PadFailed` |
| `interpret_probabilities` | Out-of-range index is `PadFailed` for every vector length |
| `evaluate_liveness` | Checks the contract on every inference before copying the logits |
| `OrtPadDetector::self_test(expected)` | One inference on a fixed synthetic 80x80 grey fixture; returns `PadSelfTestReport { class_count, live_class_index, liveness_threshold }` |

`crates/daemon/src/pipeline.rs`: `PAD_MODEL_ID`, and `validate_pad_detector(&pad, &manifest)`
(manifest entry required, class count from `output_shapes`, self-test, info log, error log and
fail closed). `initialize_pipeline` calls it with `?` before assembling the `VisionPipeline`.

The fixture verdict is not asserted: the fixture is not a face, and asserting a spoof verdict
would couple daemon startup to model calibration and configured thresholds.

## 3. Tests first (red evidence)

- `tests/fixtures/pad_onnx.rs` (new): hand-encoded ONNX graphs with input `[1, 3, 80, 80]` —
  `ReduceMean(axes=[2,3])` gives `[1, 3]`, `ReduceMean(axes=[1,2,3])` gives `[1]`, `Flatten`
  gives `[1, 19200]`. They load in a real ORT session with no model file on disk.
- `crates/inference-ort/tests/pad_output_contract_tests.rs` (16 tests),
  `crates/daemon/tests/pad_startup_validation_tests.rs` (7 tests),
  `tests/invariants/src/pad_startup_contract.rs` (1 invariant).

Before the fix: the new API did not exist (compile errors for `self_test`,
`validate_pad_output_contract`, `pad_class_count_from_manifest`, `MINIFASNET_CLASS_COUNT`,
`validate_pad_detector`, `PAD_MODEL_ID`); the invariant failed
(`initialize_pipeline must run validate_pad_detector`). The behavioural tests that only use the
pre-existing API, run against the old code, failed 5 of 6:
`test_interpret_probabilities_rejects_out_of_range_index_{three_class,two_class,single_output}`,
`test_evaluate_liveness_rejects_single_logit_head` (old code returned `Ok(live, 1.0)`),
`test_evaluate_liveness_rejects_wrong_output_length` (old code returned a spoof verdict).

## 4. Audit

- No `unwrap`/`expect`/indexing added to production code; errors are typed `PadFailed`.
- Bounded: the logit copy happens only after the length check (3 values).
- Logs carry only model id, class count, class index and threshold: no frame or embedding data.
- `#![forbid(unsafe_code)]` untouched; no PAM code changed; no Tokio or OpenCV introduced.
- Fail closed: a failed self-test aborts `initialize_pipeline`; a per-frame contract violation is
  an inference error (never `Allow`).

## 5. Result (green)

All new tests pass; `pad_tests`, `pad_wiring_tests` and the invariant suite stay green. The
real attested `minifasnet_v2_80x80.onnx` (installed at `/var/lib/soos/models`) passes the
self-test (`test_self_test_passes_on_real_minifasnet_model`).

## 6. Follow-ups

- `soos-enroll` and `soos-gui` rely on the per-inference contract check only; they could also run
  `self_test` at startup.
- Physical confirmation of the daemon startup log line on a host with the real camera.

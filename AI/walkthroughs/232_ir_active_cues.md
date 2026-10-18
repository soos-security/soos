# Walkthrough 232 — Active-Illumination On/Off Differential Liveness Cues

- **Date**: 2026-10-18
- **Issue**: GitHub #431 = PR 2 of epic #353 (split item 2), GitHub-only (no backlog id) — **Branch**: `feat/ir-active-cues`
- **Decision**: ADR 2026-10-18 "Active-Illumination Differential Liveness Cues" (amends ADR 2026-09-30 "Format-Aware PAD and Short-Term IR Liveness Policy")
- **Matrix criteria**: IRA29–IRA37 (IRA36, IRA37 pending on IR hardware, owner decision 2026-10-12)

## 1. Context & Objectives

PR 1 of #353 (#378) gave `soos` the means to switch the IR emitter and to classify how an IR stream is
lit, but nothing used the difference between a lit and an unlit frame. A single IR frame only passes the
loose #169 gate (exposure, contrast, texture) and the RGB-trained MiniFASNet at a stricter threshold, so a
static IR image, a screen or a flat print is not specifically defended against.

This PR adds the measurement side: a pure function in `soos-vision` that compares a frame captured with the
emitter on with one captured with it off and rejects pairs whose response to the emitter does not look like
a nearby, three-dimensional face with corneas. It has no consumer yet: strict IR mode (split item 4) will
pair frames by commanded emitter state and AND this result with the existing checks.

## 2. Architect Design

Spec `AI/architect_spec_ir_active_cues.md`. New module `crates/vision/src/ir_active.rs`, re-exported from
`lib.rs`:

- `IrActiveConfig` (11 thresholds, `Default` = provisional `IR_ACTIVE_*` constants, `validate()` naming the
  first offending field through `IrActiveConfigError`);
- `IrActiveRejection` (10 value-free reasons), `IrActiveStatistics` (aggregates only);
- `IrActiveCuesClear` (private field, `statistics()` only: a pass is never a liveness decision);
- `ir_active_statistics` (validation + statistics, no decision, for calibration) and
  `evaluate_ir_active_cues` (validation, then static pair → response ratio → saturation → reflectance band
  → relief → glint, first failure reported).

Inputs are `soos_frame::Frame` and the existing `soos_inference_ort::{BoundingBox, FaceLandmarks}`: no new
dependency and no new `soos-frame` type.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md` (self-check): **APPROVED** after three minor spec fixes: the statistics entry
point takes the configuration (pixel levels for saturation and glint), the difference is signed so a swapped
pair fails, and equal sequence numbers count as a static pair.

## 4. Tester Contract

`AI/tester_contract_ir_active_cues.md`: 24 tests in `crates/vision/tests/ir_active_cues_tests.rs` (20 unit,
4 proptests) and 4 invariants in `tests/invariants/src/ir_active_contract.rs`. The synthetic fixtures were
added to `tests/fixtures` `ir_strobe` (`active_pair` with seven presentations). Red phase: `E0432` /
`E0433` on `soos_vision::ir_active` and four failing invariants (module and ADR missing). No existing test
was modified, weakened or deleted.

## 5. Auditor Constraints

`AI/auditor_constraints_ir_active_cues.md` A1–A11, **CLEARED**: no panicking path (rows through
`slice::get`), checked dimension arithmetic, casts only on finite clamped values, two bounded
`Zeroizing<Vec<u8>>` crop copies with exact capacity, background summed in place, value-free messages,
veto-only result type, configuration validated first, NaN never compared, no dependency, no `unsafe`, no
authentication-path change.

## 6. Implementation

Files: `crates/vision/src/ir_active.rs` (new), `crates/vision/src/lib.rs` (module and re-exports),
`tests/fixtures/mod.rs` (`ir_strobe` active pairs), `crates/vision/tests/ir_active_cues_tests.rs` (new),
`tests/invariants/src/{ir_active_contract.rs (new), lib.rs}`, `Docs/VISION_CRATE.md` (§2.4.5, module tree,
mapping row), `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/BACKLOG.md`, the spec, contract,
constraints and plan-evaluation files, this walkthrough.

Regions are derived from the face window and the eye distance `d`: background ring (window grown by 50 %
per side, minus the window), face-edge band (15 % of each side), nose square (half-side `0.25 d`), eye-corner
squares (`0.30 d` outward along the inter-eye axis, half-side `0.08 d`) and eye regions (half-side
`max(2, 0.15 d)`).

Statistics of the fixtures with the default configuration (160 × 120 frames):

| Fixture | Face response | Ratio | Saturated | Lit mean | Relief | Glints | Result |
|---|---|---|---|---|---|---|---|
| Shaded head | 95.7 | 3.19 | 0.17 % | 113.5 | 2.97 | 4 / 4 | pass |
| Flat print | 118.0 | 2.03 | 0 % | 138.0 | 1.03 | 0 / 0 | `FlatRelief` |
| Screen | 2.0 | 0.12 | 0 % | 22.0 | 1.00 | 0 / 0 | `NoResponse` |
| No glint | 95.5 | 3.18 | 0 % | 113.3 | 2.97 | 0 / 0 | `NoGlint` |
| Saturated | 190.1 | 6.34 | 78.1 % | 207.9 | 1.58 | 81 / 81 | `Saturated` |
| Unlit | 0.0 | 0.00 | 0 % | 17.8 | −0.04 | 0 / 0 | `NoResponse` |
| Static injection | 0.0 | 0.00 | 0.17 % | 113.5 | 0.00 | 0 / 0 | `StaticPair` (before any statistic) |

These numbers describe synthetic fixtures only; they are not evidence about real faces or attacks.

## 7. Validation

Scoped, with `LIBONNXRUNTIME_NO_PKG_CONFIG=1`, a dedicated `CARGO_TARGET_DIR`, `CARGO_BUILD_JOBS=3` and
`--test-threads=3`:

- `cargo fmt -p soos-vision -p soos-test-fixtures -p soos-invariants -- --check`: clean.
- `cargo clippy -p soos-vision -p soos-test-fixtures -p soos-invariants --all-targets -- -D warnings`: clean.
- `cargo test -p soos-vision -p soos-test-fixtures -p soos-invariants`: all green (24 new vision tests,
  791 invariants including `matrix_citations` and the four new contracts).
- `cargo test -p soos-camera-v4l --features mock-camera --test strobe_classification_tests`: 17 green
  (the shared `ir_strobe` fixtures are unchanged for this consumer).
- `python3 scripts/sync_issue.py --check`: OK.

The full `./save.sh` gate, the independent candid review and CI run before the pull request is merged; this
run stops at a local commit.

## 8. Pending and Limits

- IRA36: calibration of every threshold on captured lit/unlit pairs of a real IR module.
- IRA37: real attack presentations on a real IR module, reported as counts under split item 5.
- A print of an IR capture can pass the relief cue (its reflectance carries the captured shading).
- An adaptive USB device that synthesizes lit and unlit frames is out of scope (#353).
- The open decision on `StrobeStatus::vetoes_allow(Stable(Absent))` stays with split item 4.

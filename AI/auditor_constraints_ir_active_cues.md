# Auditor Constraints — Active-Illumination On/Off Differential Liveness Cues

- **Date**: 2026-10-18
- **Issue**: GitHub #431 = PR 2 of epic #353 — **Branch**: `feat/ir-active-cues`
- **Inputs audited**: `AI/architect_spec_ir_active_cues.md`, `AI/tester_contract_ir_active_cues.md`, `crates/vision/src/{lib,ir_liveness}.rs`

| # | Constraint | How it is met |
|---|---|---|
| A1 | No `unwrap`, `expect`, `panic!`, `unreachable!` or unchecked indexing in `ir_active.rs` | rows come from `slice::get` (`row_slice`), every missing row maps to a rejection; workspace lints deny the panicking forms |
| A2 | Dimension arithmetic is checked | `usize::try_from`, `checked_mul`/`checked_add` for frame size and row offsets; `IR_ACTIVE_MAX_FRAME_PIXELS` bound |
| A3 | Float-to-integer casts only on finite, clamped values | box finite-checked then `floor`/`ceil` clamped to `[0, side]`; landmark centres are inside a window of at most 1024 × 1024 |
| A4 | Bounded allocation | two face-crop copies of at most `IR_ACTIVE_MAX_CROP_PIXELS` bytes each, exact capacity; background summed in place |
| A5 | Biometric pixels wiped | crop copies are `Zeroizing<Vec<u8>>` and never reallocate; no pixel is stored in the results |
| A6 | No sensitive output | no logging or printing; `IrActiveRejection` and `IrActiveConfigError` messages carry no value (tested) |
| A7 | Veto only | `IrActiveCuesClear` has a private field and only `statistics()`; `Ok` returned only after every cue held (tested by property) |
| A8 | Fail closed on configuration | `validate()` runs first in both entry points; NaN and out-of-range fields rejected; lower bounds keep response, relief and glint on |
| A9 | NaN never passes a comparison | non-finite statistics are rejected before thresholds; thresholds compared only after validation |
| A10 | No new dependency, no `soos-frame` change, no `unsafe` | `crates/vision/Cargo.toml` unchanged; `#![forbid(unsafe_code)]` kept; invariant IRA35 |
| A11 | No authentication-path change | `soos-daemon`, `soos-policy`, `pam_soos.so` and IPC untouched; no consumer in this PR |

VERDICT: CLEARED

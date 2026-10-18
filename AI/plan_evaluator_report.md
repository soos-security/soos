# Plan Evaluation Report
- **Date**: 2026-10-18
- **Issue**: GitHub #431 = PR 2 of epic #353 (active-illumination on/off differential liveness cues), GitHub-only (no backlog id)
- **Branch**: `feat/ir-active-cues`
- **Base commit**: `3f76090`
- **Plan evaluated**: `AI/architect_spec_ir_active_cues.md` §0–§7
- **Mode**: self-check by the implementing agent (owner run of 2026-10-18; the independent candid review still runs separately before any push).

## 1. Coverage Matrix

| #353 split item 2 / #431 line | Spec element | Status |
|---|---|---|
| Zero-I/O `crates/vision/src/ir_active.rs`, `#![forbid(unsafe_code)]`, no OpenCV, no new `soos-frame` type | §1, §2, invariant IRA35 | Covered |
| Lit/unlit grey pair with the same geometry, SCRFD box and landmarks | §2 signature, §3 items 2–3 | Covered |
| Bounded, `Zeroizing` difference statistics on the face crop | §1 (crop bound, `Zeroizing` copies, in-place ring sums) | Covered |
| Response ratio (face vs background) | §4 cue 2 | Covered |
| Inverse-square relief (nose/cheek vs edges and eye corners) | §4 cue 5, regions | Covered |
| Corneal glint in the eye regions on lit frames only | §4 cue 6 (lit level and lit − unlit delta) | Covered |
| Skin NIR reflectance band | §4 cue 4 | Covered |
| Every cue only removes liveness; NaN, empty or mismatched input rejects | §2 `IrActiveCuesClear` (no `is_live`), §3, §4 | Covered |
| Validated config struct, provisional and uncalibrated | §2 `IrActiveConfig::validate`, §3 item 1 | Covered |
| Fixtures: shaded head accepted; flat print, screen, no glint, saturated, unlit, identical pair rejected | §5 | Covered |
| Proptest: no input panics or grants liveness | IRA34 | Covered |
| No daemon/policy consumer, no `ir_mode`; open `vetoes_allow` decision untouched | §0 | Covered |
| ADR amending 2026-09-30, `Docs/VISION_CRATE.md`, matrix IRA29+, BACKLOG, walkthrough 232 | §7 | Covered |

## 2. Facts Verified

| Fact | Check | Actual | Match |
|---|---|---|---|
| Box and landmark types | `crates/inference-ort/src/{detector,landmarks}.rs`, re-exported in `lib.rs` | `BoundingBox { x1, y1, x2, y2 }`, `FaceLandmarks { left_eye, right_eye, nose, mouth_left, mouth_right }` | Yes |
| `soos-vision` already depends on `soos-inference-ort`, `soos-frame`, `zeroize`; dev-deps `proptest`, `soos-test-fixtures` | `crates/vision/Cargo.toml` | yes | Yes (no manifest change) |
| `soos-test-fixtures` depends on `soos-frame` only | `tests/fixtures/Cargo.toml` | yes, so fixture geometry is plain `[f32; 4]` / `[[f32; 2]; 5]` | Yes |
| `ir_strobe` exists (#378) and is used by `strobe_classification_tests` | `tests/fixtures/mod.rs` line 685 | existing items unchanged, only additions | Yes |
| Matrix prefix | `grep '^| IRA' AI/VERIFICATION_MATRIX.md` | IRA1–IRA28 used; IRA29+ free | Yes |
| Walkthrough number | `ls AI/walkthroughs` | 232 free | Yes |
| Vision doc module tree invariant | `vision_threshold_contract::test_vision_and_inference_docs_list_every_module` | new `ir_active.rs` must appear in the `├──` tree | Added to §7 |
| Citation invariant | `matrix_citations` resolves `test_*` and `prop_*` names | every Verified row cites real tests | Planned |
| Open decision of split item 4 | ADR 2026-10-13 item (4) | `StrobeStatus` not used by this module | Yes |

## 3. Pillar Analysis

- **Architecture alignment**: pure statistics in a business crate; no camera, IPC, PAM or daemon change.
- **Security**: veto-only result type; fail-closed validation precedes every cue; value-free reasons;
  `Zeroizing` crop copies; no logging.
- **Testability**: every cue has a dedicated fixture or a single-threshold test; the proptest covers
  arbitrary bytes, geometry and configs.
- **Honesty**: §6 records that a print of an IR capture can pass the relief cue and that no metric is
  claimed.

## 4. Findings

- P-1 (minor, fixed in spec): `ir_active_statistics` must take the config, because saturation and glint
  counting need its levels.
- P-2 (minor, fixed in spec): a swapped pair must fail; the difference is signed so the response is
  negative.
- P-3 (minor, fixed in spec): equal sequence numbers are treated as a static pair even if the bytes
  differ (the same capture passed twice after an in-place edit is not a lit/unlit pair).

VALIDATION_VERDICT: APPROVED

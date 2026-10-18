# Plan Evaluation Report
- **Date**: 2026-10-18
- **Issue**: GitHub-only (no backlog id) — `soos-gui` motion system with portal-driven reduced motion (GitHub #438 = PR 3
  of epic #358, matrix UIM9)
- **Branch**: `feat/gui-motion`
- **Base commit**: `3f76090`
- **Plan evaluated**: `AI/architect_spec_gui_motion.md` §1–§9
- **Mode**: self-check by the implementing agent (the independent candid review runs separately before any merge).

## 1. Coverage Matrix

| Contract line (`AI/tester_contract_motion.md`) | Spec element | Status |
|---|---|---|
| B.2.1 tokens, UIM-S2 live, `theme_tests.rs` / `brand_layout_tests.rs` unmodified | §2 | Covered |
| B.2.2 spring at 30/60/144 Hz, overshoot 1–4 %, retarget continuity, no-op retarget, dt edge cases, non-finite target, zero span | §3 `SpringState` | Covered |
| B.2.3 easing endpoints, clamp, monotonic, `NaN` | §3 `cubic_bezier` | Covered |
| B.2.4 resolution order and `--reduce-motion` | §3, §7 | Covered |
| B.2.5 portal off the UI thread, never-answering bus, dead reader, `try_recv`, no retry, no spawn, direct deps only | §5, §8 | Covered |
| B.2.6 reduced: target on first frame, `animation_time` 0, no repaint | §4 | Covered |
| B.2.7 idle budget (120 frames), pending star via `request_repaint_after` ≥ 33 ms, `responsiveness_tests.rs` unmodified | §4 | Covered |
| B.2.8 truthful overlay | §6 G9 | Covered |
| B.2.9 spinners replaced, authorization text visible | §6 G2, G7 | Covered |
| B.2.10 hygiene (no panic paths, no per-frame allocation, clippy, fmt) | §1, §3, §4 | Covered |
| B.2a PAU17 migration and GUI zbus invariant, guideline still names `soos-remote` | §1, §5, §9 | Covered |
| B.3 approval gate | header | Covered (PR body, no auto-merge) |

## 2. Facts Verified

| Fact | Check | Actual | Match |
|---|---|---|---|
| UIM-S2 parser | `ui_motion_spec_contract.rs` `rust_const_value`, `numbers_in` | reads the literal between `=` and `;`; seconds = ms / 1000; `[f32; 4]` arrays parsed as four numbers | Yes (§2 uses plain literals) |
| PAU17 | `presence_unlock_contract.rs:197` | `ALLOWED` of two manifests, `zbus` named exactly once per allowed manifest | Yes (no comment in `crates/gui/Cargo.toml` may name zbus) |
| zbus 6 API | `zbus-6.0.0/src/connection/builder.rs`, `wire/variant.rs`, `wire/owned_variant.rs` | `Builder::session() -> Self`, `method_timeout`, `max_queued`, `call_method`, `OwnedVariant: Deref<Target = Variant>`, `Variant::{U32, Bool, Variant}` | Yes |
| zbus default features | workspace `Cargo.toml:65` | `default-features = false, features = ["tokio"]` (no proxy, no tracing) | Yes |
| egui 0.35 API | `egui-0.35.0` sources | `Ui::set_opacity`/`opacity`, `with_visual_transform`, `cumulative_pass_nr`, `IdTypeMap::get_temp_mut_or_insert_with`, `Style::animation_time`, `Mesh::rotate` | Yes |
| `stable_dt` in headless tests | `input_state/mod.rs:376` | `predicted_dt` when no immediate repaint was requested, else the measured `time` delta | Yes (tests drive `predicted_dt` and `time`) |
| README capture | `app/capture.rs`, `tests/readme_capture/mod.rs` | builds `SoosApp` literally, renders the last of several passes, compares to committed PNGs | Yes (capture stays reduced; widgets at rest unchanged) |
| Spinners | `grep Spinner crates/gui/src` | `header.rs:374`, `camera_status.rs:154` only | Yes |
| Prototype | `Docs/prototypes/ui_motion_prototype.html` | sliding pale tab pill (`translate`, `T_MEDIUM`, standard), labels `T_FAST`, pending star turning a quarter per 1.2 s | Yes |
| Matrix prefix `GMO` unused | `grep '^| GMO' AI/VERIFICATION_MATRIX.md` | 0 hits | Yes |
| Walkthrough 234 free | `ls AI/walkthroughs` | absent (number assigned by the lead) | Yes |

## 3. Pillar Analysis

- **Architecture alignment**: Tokio and zbus stay off the UI thread and out of `pam_soos.so`; the PAU17 widening is
  the one recorded in the motion ADR item (6).
- **Test integrity**: the only edit of an existing test is the B.2a migration (one `ALLOWED` entry, array length);
  every other assertion, message and the test name are unchanged.
- **Truthfulness**: animation follows the confirmed state (star turn on a confirmed `Inactive`, overlay never
  interpolated, camera status text replaced at once).
- **Supply chain**: no new crate in `Cargo.lock`.

## 4. Findings

| # | Severity | Finding | Resolution |
|---|---|---|---|
| P-1 | MINOR | `Docs/UI_MOTION.md` G7 asks for the old status to fade out over `T_SHORT_EXIT` before the new one fades in; keeping stale camera text on screen contradicts principle 6 (security truth first) | Spec §6 G7 replaces the text at once and fades the new one in; `Docs/UI_MOTION.md` G7 updated to match |
| P-2 | MINOR | G4 "stat values cross-fade" would fade a live match score at camera rate (flicker, continuous repaints, §9 budget) | Spec §6 G4 fades only when the caption or the presence of the value changes; documented in `Docs/UI_MOTION.md` G4 |
| P-3 | INFO | Banner text ids could grow the temp store without bound | Banner state uses 64 hashed buckets with a collision check |

VALIDATION_VERDICT: APPROVED

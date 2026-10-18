# Tester Contract — Active-Illumination On/Off Differential Liveness Cues

- **Date**: 2026-10-18
- **Issue**: GitHub #431 = PR 2 of epic #353 — **Branch**: `feat/ir-active-cues`
- **Spec**: `AI/architect_spec_ir_active_cues.md`
- **Matrix**: IRA29–IRA35 (IRA36, IRA37 pending on IR hardware)

## 1. New Test Files

| File | Tests | Rows |
|---|---|---|
| `crates/vision/tests/ir_active_cues_tests.rs` | 20 `#[test]` + 4 proptests | IRA29–IRA34 |
| `tests/invariants/src/ir_active_contract.rs` (registered in `tests/invariants/src/lib.rs`) | 4 | IRA35 |

Fixture additions (test infrastructure, no assertion): `tests/fixtures/mod.rs` `ir_strobe` gains
`ACTIVE_*` constants, `ActivePresentation`, `ActivePair` and `active_pair`. The existing `ir_strobe`
items used by `strobe_classification_tests` are unchanged.

## 2. Contract

- IRA29: `test_ira_active_shaded_head_passes_every_cue`, `test_ira_active_statistics_match_the_evaluation`,
  `test_ira_active_statistics_are_computed_for_attack_fixtures_too`.
- IRA30: `test_ira_active_flat_print_is_rejected_for_flat_relief`, `test_ira_active_screen_is_rejected_for_no_response`,
  `test_ira_active_missing_glint_is_rejected`, `test_ira_active_saturated_pair_is_rejected`,
  `test_ira_active_unlit_pair_is_rejected_for_no_response`, `test_ira_active_identical_pair_is_rejected_as_static_injection`,
  `test_ira_active_only_the_shaded_head_passes`.
- IRA31: `test_ira_active_swapped_pair_is_rejected`, `test_ira_active_each_cue_alone_vetoes_the_shaded_head`.
- IRA32: `test_ira_active_rejects_malformed_frames`, `test_ira_active_rejects_malformed_geometry`,
  `test_ira_active_rejects_a_face_filling_the_frame`, `test_ira_active_rejects_a_face_crop_above_the_bound`.
- IRA33: `test_ira_active_default_config_is_valid_and_provisional`, `test_ira_active_config_validation_names_the_field`,
  `test_ira_active_invalid_config_always_rejects`, `test_ira_active_rejection_messages_carry_no_value`.
- IRA34: `prop_ira_active_arbitrary_input_never_panics`, `prop_ira_active_malformed_input_always_rejects`,
  `prop_ira_active_tightening_never_turns_a_rejection_into_a_pass`, `prop_ira_active_identical_random_frames_are_static`.
- IRA35: `test_ira_active_module_is_zero_io_and_silent`, `test_ira_active_crop_copies_are_zeroizing`,
  `test_ira_active_result_is_veto_only`, `test_ira_active_adr_and_docs_are_recorded`.

## 3. Red Phase Evidence

Before `ir_active.rs` existed, `cargo test -p soos-vision --test ir_active_cues_tests` failed to compile
with exactly `E0432 unresolved import soos_vision::ir_active` and `E0433 cannot find ir_active in
soos_vision`; the four `ir_active_contract` tests failed reading the missing module and the missing ADR.
No existing test was modified, weakened or deleted.

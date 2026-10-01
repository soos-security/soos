# Walkthrough 164 — SFace Switch Follow-ups: GUI Match Reference, Extractor Spec Layout, Store Doc

- **Date**: 2026-10-01
- **Issue**: GitHub #298 (review follow-ups of the SFace switch, PR #297; no backlog issue) —
  **Branch**: `fix/sface-gui-and-spec-followups`
- **Matrix criteria**: SGF1–SGF4 (component `sface-gui-spec-doc-followups`)

## 1. Context & Objectives

The approved candid review of the SFace switch (walkthrough 162) listed optional follow-ups in
GitHub #298. This batch covers three of them (the daemon dispatcher and rate-limit items are
delivered by a separate branch):

1. `crates/gui/src/app.rs`: selecting a profile updated the match reference and the live score in
   two separate critical sections, a selection whose template lookup failed left both the previous
   reference and the previous note in place, and switching between two current profiles kept the
   previous profile's score on screen until the next frame.
2. `crates/inference-ort/src/embedding.rs`: `OrtEmbeddingExtractor::with_spec` accepted a spec of
   any layout although extraction always feeds an NCHW tensor (it only failed closed at run time).
3. `Docs/BIOMETRIC_STORE_CRATE.md`: the CRUD example still built a 512-D `glintr100` template.

## 2. Architect Design

- `soos_gui::worker::select_match_reference(shared, template: Option<&BiometricTemplate>,
  loaded_model_id, loaded_dimension) -> Option<String>`: installs the new reference (only for a
  template accepted by `template_matches_model`) and clears `live_match_score` while still holding
  the `match_reference` lock; returns the re-enrollment note for a foreign template and `None`
  otherwise (a failed lookup therefore clears the note). Poisoned locks are recovered
  (`PoisonError::into_inner`) so the display state is always cleared.
- `soos_gui::worker::update_live_match_score(shared, live: &[f32])`: the worker's existing scoring
  block, extracted unchanged; it writes the score while holding the reference lock. Lock order is
  reference then score on both sides, so no deadlock and no score computed against an old
  reference can become visible after a selection.
- `OrtEmbeddingExtractor::with_spec` now returns `Result<Self, InferenceError>` and rejects a
  non-NCHW `spec.input_layout` with `InferenceError::InvalidInput` naming the model id and NCHW.
  `OrtEmbeddingExtractor::new` keeps its infallible signature through a private `bind`; a
  `const _: () = assert!(...)` guarantees at compile time that `SHIPPED_EMBEDDING_MODEL` is NCHW.
  No caller outside the module used `with_spec`.
- Invariants touched: none of the PAM or daemon invariants (no daemon or PAM change).

## 3. Plan Evaluation

Scope agreed with the orchestrator (no separate plan-evaluator run for this follow-up batch):
`crates/daemon` is out of scope, no existing assertion may change. Verdict: proceed.

## 4. Tester Contract

| Test (path::name) | Matrix ID | Red evidence |
|---|---|---|
| `crates/gui/tests/match_reference_selection_tests.rs::test_selecting_current_template_sets_reference_and_withdraws_previous_score` | SGF1 | `assertion left == right failed, left: None, right: Some([1.0, 0.0, ...])` (stub left the reference unset) |
| `...::test_switching_to_foreign_profile_shows_no_stale_score` | SGF1 | `assertion failed: score(&shared).is_some()` |
| `...::test_concurrent_worker_leaves_no_score_after_foreign_selection` | SGF1 | `assertion failed: select(&shared, Some(&foreign_template())).is_some()` |
| `...::test_failed_template_lookup_clears_note_reference_and_score` | SGF2 | `assertion failed: select(&shared, Some(&foreign_template())).is_some()` |
| `crates/inference-ort/tests/embedding_spec_layout_tests.rs::test_with_spec_rejects_nhwc_spec_for_nchw_session` | SGF3 | `a non-NCHW spec must be rejected at construction` (stub accepted every spec) |
| `...::test_with_spec_rejects_nhwc_spec_for_nhwc_session` | SGF3 | same panic |
| `...::test_with_spec_accepts_nchw_shipped_spec` | SGF3 | passes (non-regression of the accepted path) |
| `tests/invariants/src/sface_followups_contract.rs::test_biometric_store_doc_example_uses_shipped_sface_template` | SGF4 | `Docs/BIOMETRIC_STORE_CRATE.md still names the retired glintr100 model` |

Red was proven against signature stubs (`select_match_reference` returning `None` without touching
state, `with_spec` returning `Ok` for every spec).

### Migrated existing tests

None. No existing test file was edited.

### Flakiness check

`match_reference_selection_tests` (the concurrent test runs 200 selection rounds against a
spinning worker thread) was run 10 times in a row: 10/10 green.

## 5. Auditor Constraints

1. No `unwrap`, `expect` or `panic!` in production code: the GUI helper recovers poisoned locks
   with `PoisonError::into_inner`; `with_spec` returns an error. Met.
2. No embedding or template value is logged; the note contains only the model id and the vector
   length (as before). Met.
3. Fail closed early: a non-NCHW spec cannot build an extractor. Met.
4. No change to `crates/daemon` or `crates/pam`. Met.
5. `#![forbid(unsafe_code)]` kept in `soos-gui` and `soos-inference-ort`. Met.

## 6. Implementation

| File | Change |
|---|---|
| `crates/gui/src/worker.rs` | `select_match_reference`, `update_live_match_score` (worker loop uses it) |
| `crates/gui/src/app.rs` | profile selection calls `select_match_reference` for every lookup result |
| `crates/inference-ort/src/embedding.rs` | fallible `with_spec`, private `bind`, compile-time NCHW assertion on the shipped spec |
| `Docs/BIOMETRIC_STORE_CRATE.md` | CRUD example uses `sface_2021dec`, `2.0.0`, 128 values |
| `Docs/GUI_APPLICATION.md` | section 1d "Live Verification Reference" |
| `Docs/INFERENCE_ORT_CRATE.md` | `with_spec` signature and rejection rule |
| `tests/invariants/src/sface_followups_contract.rs`, `tests/invariants/src/lib.rs` | SGF4 invariant |

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) run locally on this branch; the Layer 2 sub-agent review
is run by the orchestrator before the push.

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=6
SOOS_MODELS_DIR=<scratch SFace models dir> cargo test --locked --all-features \
  -p soos-gui -p soos-inference-ort -p soos-biometric-store -p soos-invariants   # 731 passed, 0 failed
cargo fmt --all -- --check                                                       # clean
cargo clippy --locked --all-targets --all-features \
  -p soos-gui -p soos-inference-ort -p soos-invariants -p soos-biometric-store -- -D warnings  # clean
./scripts/candid_review.sh                                                       # Layer 1 passed
```

## 9. Known Limitations / Follow-ups

- The GUI match reference is still a plain `Vec<f32>` copy of the template (unchanged behavior);
  wrapping it in `Zeroizing` would be a separate hardening item.
- The GitHub #298 dispatcher (template check before camera wake) and rate-limit items are
  delivered by the parallel daemon branch.

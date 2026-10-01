# Candid Review Report

- **Date**: 2026-10-01
- **Target Branch**: `feat/sface-embedding-model`
- **Base (merge-base)**: `11c967e`
- **Reviewed-Diff-Fingerprint**: `262e6aa29023848ca390e82463ef08a47838e144a89a6841282bba3a91e1135e`
- **Fingerprint cross-check**: `./scripts/candid_subagent.sh --prepare` on the clean working tree and the `--rev HEAD` gate computation over `HEAD^{tree}` (commit `0011039`) both yield the fingerprint above.
- **Review round**: second review. The first review (fingerprint `29d417d8…d595656`, HEAD `5963958`) was APPROVED with four MINOR findings; commit `0011039` addresses them. This report covers the full diff `11c967e..0011039`; the first-round audit of the 71 files still applies because `0011039` changes only three documentation/script files (verified with `git diff --stat 5963958 0011039`).
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/162_sface_embedding_model.md`, `Docs/DAEMON.md`, `Docs/ENROLLMENT_CLI.md`, `Docs/INFERENCE_ORT_CRATE.md`, `Docs/MEMORY_PROTECTION_AND_SWAP.md`, `Docs/POLICY_CRATE.md`, `Docs/VISION_CRATE.md`, `crates/biometric-store/src/template.rs`, `crates/daemon/src/dispatcher.rs`, `crates/daemon/src/inference.rs`, `crates/daemon/src/pipeline.rs`, `crates/daemon/tests/model_deployment_tests.rs`, `crates/daemon/tests/pad_nonface_pipeline_real_model_tests.rs`, `crates/daemon/tests/sface_template_binding_tests.rs` (new), `crates/daemon/tests/template_model_binding_tests.rs`, `crates/enrollment-cli/src/args.rs`, `crates/enrollment-cli/src/error.rs`, `crates/enrollment-cli/src/guided_enrollment.rs`, `crates/enrollment-cli/src/main.rs`, `crates/enrollment-cli/src/service.rs`, `crates/enrollment-cli/tests/cli_hygiene_tests.rs`, `crates/enrollment-cli/tests/enroll_model_provenance_tests.rs`, `crates/enrollment-cli/tests/import_enroll_if_absent_tests.rs`, `crates/enrollment-cli/tests/import_json_presize_tests.rs`, `crates/enrollment-cli/tests/import_stdin_overwrite_tests.rs`, `crates/enrollment-cli/tests/import_stdin_tests.rs`, `crates/enrollment-cli/tests/import_tests.rs`, `crates/enrollment-cli/tests/migrate_tests.rs`, `crates/enrollment-cli/tests/model_id_tests.rs`, `crates/enrollment-cli/tests/quality_gate_report_tests.rs`, `crates/enrollment-cli/tests/verify_pad_report_tests.rs`, `crates/enrollment-cli/tests/verify_tests.rs`, `crates/gui/src/app.rs`, `crates/gui/src/lib.rs`, `crates/gui/src/main.rs`, `crates/inference-ort/Cargo.toml`, `crates/inference-ort/src/embedding.rs`, `crates/inference-ort/src/lib.rs`, `crates/inference-ort/src/mock.rs`, `crates/inference-ort/src/registry.rs`, `crates/inference-ort/tests/embedding_io_contract_tests.rs`, `crates/inference-ort/tests/embedding_preprocessing_evaluation_tests.rs`, `crates/inference-ort/tests/embedding_real_model_tests.rs`, `crates/inference-ort/tests/embedding_tests.rs`, `crates/inference-ort/tests/manifest_shape_tests.rs`, `crates/inference-ort/tests/manifest_tests.rs`, `crates/inference-ort/tests/registry_tests.rs`, `crates/policy/src/threshold.rs`, `crates/policy/tests/decision_tests.rs`, `crates/policy/tests/pad_consensus_tests.rs`, `crates/policy/tests/threshold_tests.rs`, `crates/vision/src/pipeline.rs`, `crates/vision/tests/embedding_lfw_evaluation_tests.rs`, `crates/vision/tests/pipeline_tests.rs`, `crates/vision/tests/threshold_constants_tests.rs`, `models/README.md`, `models/manifest.toml`, `models/retired_models.toml` (new), `scripts/download_models.sh`, `tests/docker/systemd_unit_acceptance_test.sh`, `tests/fixtures/mod.rs`, `tests/invariants/src/embedding_model_docs_contract.rs` (new), `tests/invariants/src/lib.rs`, `tests/invariants/src/vision_threshold_contract.rs`, `tests/physical/screensaver_test.md` (71 files)

## 1. Executive Summary

The diff implements the owner decision of 2026-10-01 (GitHub #278): OpenCV Zoo SFace 2021dec
(`sface_2021dec`, 128-D, NCHW, RGB raw 0..255) replaces the ArcFace ResNet34
(`arcface_w600k_mbf`, 512-D, NHWC, BGR `/127.5`). The default `match_threshold` moves from 0.70
to 0.50 (floor 0.40 unchanged). Templates are bound by model id **and** vector length through
one pure function (`soos_inference_ort::template_matches_model`), and the legacy `mobilefacenet`
alias is removed. `import` only accepts SFace/128, which also fixes the literal ArcFace default
used by the GUI pkexec import. `verify` and the GUI refuse foreign templates before capture or
matching, and `migrate`/`list` print re-enrollment notices. The ArcFace attestation moves to
`models/retired_models.toml`, and only the SFace ORT session logs at `Error`.

Round 2 delta (`0011039`): the four MINOR findings of round 1 are fixed exactly as requested,
with no code change outside `scripts/download_models.sh` (one warning string). No new finding.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Round 2: `0011039` touches no test file (diff stat: walkthrough 162, `Docs/ENROLLMENT_CLI.md`,
`scripts/download_models.sh`). The mechanical listing of step 3 on the new patch is identical
to round 1:

- Escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`) added: **none**.
- Inline `mod tests` added/removed: **none**.
- Removed/changed assertions: all map onto the owner-approved items 1–56 of walkthrough 162 §4,
  changed exactly as described. #2 and #26–#28 are stricter, and #56 is setup only
  (`257` → `IMPORT_EMBEDDING_DIM / 2 + 1`). The vector-length setup of the oversized/owner/symlink
  tests in `import_stdin_tests` falls under #44. The undocumented candidate-slot change of the
  ignored LFW harness (#37) is now recorded in an execution note (round-1 MINOR-4). Items #24 and
  #52–#55 are "left as they are" and are absent from the patch.

| Area | Approved items | Verdict |
|---|---|---|
| `embedding_io_contract_tests`, `embedding_tests` | #1–#8 (+ SFC2) | exact, strength kept or increased |
| `embedding_real_model_tests`, `embedding_preprocessing_evaluation_tests` | #9–#18 (+ SFC5, SFC6, SFC18) | exact (#17 deleted and replaced by SFC6) |
| `manifest_shape_tests`, `manifest_tests`, `registry_tests` | #19–#23 (+ SFC1) | exact |
| `template_model_binding_tests`, `model_deployment_tests`, `pad_nonface_pipeline_real_model_tests` | #25–#31 | exact, stricter |
| policy and vision threshold tests, LFW harness | #32–#37 | exact |
| enrollment-cli tests | #38–#50, #56 (+ SFC13–SFC15) | exact / setup only |
| `systemd_unit_acceptance_test.sh`, `tests/fixtures/mod.rs` | #51, #52 (c) | exact / doc only |
| invariants | SFC12, SFC16, SFC17 | additive |

Reviewer re-runs:
- Round 1: targeted `soos-enrollment-cli`, `soos-daemon` and `soos-inference-ort` targets were green.
- Round 2: `cargo test -p soos-invariants embedding_model_docs_contract` (SFC16/SFC17, which
  executes `download_models.sh` and checks the reworded "not attested" notice) gave 3 passed / 0 failed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Round 1 scenarios, all **PASS**:
  - **Daemon 8d:** id + dimension binding runs before inference, and the spy extractor proves zero inferences.
  - **`verify`:** refuses before capture.
  - **GUI:** installs no reference for a foreign template.
  - **`import`:** CBOR and JSON are both checked against `MODEL_ID_EMBEDDING`.
  - **`migrate`:** never rebinds.
  - **`list`:** warns on stderr, so the JSON is unchanged.
  - **Single source:** `SHIPPED_EMBEDDING_MODEL`.
  - **Grep of `crates/*/src`:** no remaining 512/0.70/ArcFace/alias assumption.
  - **RGB raw NCHW:** equals OpenCV `blobFromImage(bgr, swapRB=true)` because the soos aligned crop is RGB24. There is no double swap, and SFC6 uses a channel-asymmetric gradient.
- Round 2: `Docs/ENROLLMENT_CLI.md:45` now states the implemented rule. A template is current only
  for `sface_2021dec` with 128 values. Any other id (including `mobilefacenet`/`1.0.0` and
  `arcface_w600k_mbf`) or any other length gets `Unavailable`/`ModelUnavailable` before inference,
  which means a password fallback, and the user must re-enroll. This is consistent with
  `pipeline::classify_template` and with lines 43 and 48. The walkthrough status, gate row and F6
  now agree with §4 item 56 and commit `5963958`. **PASS**.

### PAM Concurrency & Deadlines

- `crates/pam` is untouched, and `Unavailable` already maps to `PAM_IGNORE`. The 8d refusal comes
  before inference, so no new blocking path appears. Round 2 changes no code path. **PASS**.

### Panic Safety & Fail-Closed

- **No panics:** the new production code has no `unwrap`/`expect`/`panic!`/indexing.
- **Wrong session:** an NHWC session fails extraction with `TensorError`, and a wrong output length gives `DimensionMismatch`.
- **Partial upgrade:** in either direction the daemon refuses to start, so PAM returns `PAM_IGNORE`.
- **ORT log level:** `Error` applies to the SFace session only (SFC18), and errors stay visible.
- **Rate limit:** semantics are unchanged; a refusal never reaches `Allow`.
- **Round 2:** the warning-string change is output only; the loop still never deletes and still uses an exact name comparison.
- **PASS**.

### Test Integrity & Anti-Weakening

- See section 2. There is no test change in round 2. **PASS**.

### Memory, Bounds & Secrets

- `Zeroizing` and `ZeroizingOutputs` are unchanged. The bounded import is unchanged. New messages
  carry only UIDs, model ids and dimensions. Round 2 adds no logging of data. **PASS**.

### Supply Chain & Automation

- No dependency change. The manifest pins the HF revision `3d70824…`, the SHA-256 `0ba9fbfa…4c34e79`
  and `size_bytes = 38696353` over HTTPS. `retired_models.toml` is never read at runtime (SFC17).
- Round 2: the `download_models.sh:551` notice no longer claims an untracked file is unused. It
  tells the operator to keep a file enabled as an optional model in the deployed manifest and to
  remove a retired one manually. This resolves the misleading `--check-only` case without changing
  behaviour. **PASS**.

### English-Only Policy

- Round-2 text (doc sentence, walkthrough notes, warning string, commit message) is English.
  Commit `0011039` follows Conventional Commits (`docs(embedding): …`, `Refs #278`). **PASS**.

## 4. Detailed Findings & Action Items

Round-1 findings, all resolved in `0011039`:

- ~~**[MINOR]** `Docs/ENROLLMENT_CLI.md:45` stale legacy-alias sentence~~ — fixed (current rule
  stated: `sface_2021dec` + 128 only, everything else `Unavailable`/`ModelUnavailable`, re-enroll).
- ~~**[MINOR]** walkthrough 162 status / gate row / F6 showed item 56 as pending~~ — fixed (approved
  and applied in `5963958`; 2166 passed / 0 failed).
- ~~**[MINOR]** `scripts/download_models.sh:551` "and is unused" claim~~ — fixed (keep the file if
  enabled as an optional model in the deployed manifest, remove a retired one manually).
- ~~**[MINOR]** LFW harness candidate-slot change undocumented under §4 #37~~ — fixed (execution
  note added).

New findings in round 2: **none**.

Suggestions carried to a follow-up issue by the coordinator (not this PR):
- In `dispatcher.rs`, run steps 8c/8d before the camera wake.
- Fix the GUI stale-score race (`app.rs:813-827`).
- Make `with_spec` reject a non-NCHW spec (`embedding.rs:235/346`).
- Update the 512-D example in `Docs/BIOMETRIC_STORE_CRATE.md:225`.

Note (informational): the walkthrough status line already reads "Phase 5 (candid review)
APPROVED". This report confirms it for fingerprint `262e6aa2…1135e`.

## 5. Gate Result

`./scripts/candid_subagent.sh` (Layer 1 deterministic invariants + Layer 2 fingerprint/verdict)
on the working tree with this report: **PASSED** (Layer 1 all invariants verified; Layer 2
APPROVED report bound to fingerprint 262e6aa29023…).

## 6. Final Verdict

No CRITICAL or MAJOR finding. Every round-1 MINOR finding is resolved, and nothing outside the
three declared files changed.

**VERDICT: APPROVED**

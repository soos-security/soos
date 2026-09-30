# Walkthrough 104 — Verification Matrix Citations Resolve to Real Evidence

- **Date**: 2026-09-30
- **Issue**: Review finding TCI-04 (GitHub #187) — **Branch**: `fix/verification-matrix-citations`
- **Matrix criteria**: MXC1–MXC3 (new, ✅ Verified); 15 existing rows corrected in place
  (table in §5), GARP2 downgraded to `⬜ Pending`, GEPU3 set to `⏹ Superseded`
- **Related**: walkthrough 85 (PAD rows, PTF4/PTF5), walkthrough 98
  (`test_matrix_gdm_references_resolve_to_real_tests`)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Tests, CI, Tooling
& Documentation) found that `AI/VERIFICATION_MATRIX.md`, the contractual acceptance record,
marked criteria `✅ Verified` with tests that do not exist (`test_minifasnet_live_class_index_is_1`,
`test_default_timeout_is_1000ms`, `test_gdm_pam_line_includes_timeout_ms_2500`,
`test_daemon_pipeline_decision_budget_calibrated_for_warmup`), cited a non-existent
`tests/physical_hardware_tests.rs`, cited production source files and methods as tests (PRX2,
GEPU3), and contradicted the code (D7 "150ms decision budget" vs `DECISION_BUDGET_MS = 900`;
VZF3 listing the removed `OrtLandmarkDetector`; PAD1/NGM9 live class index 0 vs 1).

Earlier batches had already fixed part of it with narrow, row-specific invariants: the PAD rows and
the class-index prose (walkthrough 85: PTF4, PTF5; PAD1, NGM9, ASG1, `AI/DECISIONS.md`,
`Docs/INFERENCE_ORT_CRATE.md` now all say index 1 per ADR 2026-09-29/30), the GDM rows
(walkthrough 98: LSF1, ASG5) and PK5 (`tests/docker/test_packages.sh` now runs in the CI job
`package-deploy`; PK6/PK7 are `⬜ Pending`).

Objective: one repository-wide invariant for every claimed row, then fix every row it reports —
point it to the real test that proves the criterion, or downgrade it when no evidence exists.
Never invent evidence; keep row IDs; do not delete or reorder rows.

## 2. Architect Design

New module `tests/invariants/src/matrix_citations.rs` (declared `#[cfg(test)] mod
matrix_citations;` in `tests/invariants/src/lib.rs`; zero dependencies, `#![forbid(unsafe_code)]`
inherited):

- **Row selection**: table lines split on unescaped `|` (so `O_CREAT \| O_EXCL` stays one cell);
  a row is *claimed* when its last cell starts with `✅ Verified` or `☑ Validated` (a
  `⬜ Pending (spec ✅ Verified ...)` row is not); checked global invariants (`- [x]`) are claimed
  too. Cell 2 is the criterion; the remaining cells are evidence.
- **Annotations**: text inside `*( ... )*` is history ("renamed from", "never existed") and is
  removed before parsing.
- **Tokens**: every backticked span, split on whitespace and on commas outside `{...}`; brace lists
  are expanded (`tests/docker/Dockerfile.{ubuntu,fedora,arch}`); surrounding `()[];,` stripped.
- **Classification**:
  - path: contains `/` and its first segment is a top-level repository entry (runtime paths such
    as `/var/lib/soos` or `state/...` are ignored); `:line` / `#anchor` suffixes stripped;
    `*`, `?`, `**` globs supported;
  - bare file name: `[A-Za-z0-9_-]+\.(rs|sh|py)` (e.g. `debug_vision_tests.rs`, `test_suite.sh`);
  - test: last `::` segment starts with `test_` (optionally ending in `*`), or is `*` with a
    qualifier (`pipeline_integration_tests::*`). `test_name` is a grammar placeholder (PTF5).
- **Resolution** against a single repository snapshot (skips `.git`, `target`, `.claude`):
  - each qualifier narrows the candidate files: `soos-<crate>` / `soos_<crate>` → that package's
    directory (read from every `Cargo.toml`), `tests`/`crate`/`super`/`self` ignored, otherwise the
    file stem, the parent directory or a declared `mod name {`/`mod name;`; an unknown qualifier
    fails;
  - a test function is a Rust `fn` whose attribute block contains a test attribute (`#[test]`,
    `#[tokio::test]`, ...; `#[cfg(test)]` alone does not count), any `fn` in a `proptest!` file, a
    shell `name()` / `function name`, or a Python `def`; `test_*` helpers without an attribute
    (e.g. `fn test_dispatcher_config`) are not tests;
  - a `.rs` file cited in the evidence columns must contain a test unless it is test-support code
    under a `tests/` directory (fixtures); a production source is never test evidence.
- **Content invariants** for the contradictions: superseded criterion ⇒ non-claimed status;
  "decision budget" values equal `DECISION_BUDGET_MS` parsed from `crates/daemon/src/pipeline.rs`;
  `OrtLandmarkDetector` only mentioned with "removed".
- A floor of 900 checked citations (and 300 claimed rows) prevents a vacuous pass if the matrix
  format drifts away from the parser.

## 3. Tester Contract (written first)

| Test | Purpose |
|---|---|
| `test_matrix_claimed_rows_cite_only_existing_evidence` | Every citation of every claimed row resolves (MXC1) |
| `test_matrix_citation_parser_recognizes_every_citation_form` | Parser self-test: escaped pipes, `and siblings`, globs, braces, bare names, placeholders, annotations, pending rows, runtime paths, decision-budget extraction, module declarations (MXC2) |
| `test_matrix_citation_parser_detects_test_functions_only` | Test detection: `#[test]`, `#[tokio::test]` with doc comment, nested `#[test]` + `#[should_panic]`, helper without attribute rejected; shell functions (MXC2) |
| `test_matrix_superseded_rows_do_not_claim_verification` | GEPU3-style self-contradiction (MXC3) |
| `test_matrix_decision_budget_matches_code_constant` | D7 150 ms vs 900 ms (MXC3) |
| `test_matrix_claimed_rows_do_not_list_removed_landmark_detector` | VZF3 vs NGM6 (MXC3) |

Existing tests are untouched (`test_pad_matrix_rows_cite_existing_tests`,
`test_matrix_gdm_references_resolve_to_real_tests`, `test_minifasnet_class_contract_prose_matches_code_constant`).

### Red evidence (before any row edit)

```
test matrix_citations::test_matrix_superseded_rows_do_not_claim_verification ... FAILED
  superseded rows still claim verification: ["L380 GEPU3"]
test matrix_citations::test_matrix_claimed_rows_do_not_list_removed_landmark_detector ... FAILED
  rows list the removed OrtLandmarkDetector: ["L231 VZF3"]
test matrix_citations::test_matrix_decision_budget_matches_code_constant ... FAILED
  DECISION_BUDGET_MS = 900 but claimed rows state: ["L78 D7: 150 ms"]
test matrix_citations::test_matrix_claimed_rows_cite_only_existing_evidence ... FAILED
  21 of 1059 verification-matrix citations do not resolve
  (PA5, PAD5, BIO1, CAM1 ×2, CAM2, PRX1 ×3, PRX2 ×2, ASG3 ×2, ASG4 ×2, LSF3 ×2, GARP1 ×2, GARP2)
```

The first prototype also reported parser false positives, which drove the robustness rules above:
the PTF5 grammar placeholder `module::test_name`, the DVM1 brace list, the DVM7 bare file name
`test_suite.sh`, the DIB6 module glob over `#[tokio::test]` functions, and the PLC1 fixture
`tests/fixtures/mod.rs` (test-support code, not a production source).

## 4. Auditor Constraints

1. Test-only code: the module is `#[cfg(test)]`, reads files only, writes nothing, and uses no
   network or process execution.
2. No weakening: no existing test, assertion or matrix row is deleted; row IDs and order are
   unchanged; each correction was checked by reading the cited test body (not only its name).
3. Evidence honesty: where no test proves a criterion, the row is downgraded (GARP2) or the
   unevidenced sub-claim is marked as drift (ASG4 "wake timeout 800ms"); no new evidence invented.
4. Performance: the repository is loaded once per test and test functions / module names are
   indexed per file (first draft took 172 s, final 2–3 s in a debug build).

## 5. Corrected and Downgraded Rows

| Row | Before | After | Reason |
|---|---|---|---|
| D7 | "150ms decision budget"; cites only `test_12_2_...` | "900 ms decision budget (`DECISION_BUDGET_MS`)"; adds `config_tests::test_decision_budget_calibrated_to_pam_deadline` | Prose drift: `crates/daemon/src/pipeline.rs` defines 900 ms |
| PA5 | `test_pam_crate_has_no_unwraps_or_expects` | `test_pam_crate_has_no_unwraps_or_expects_in_production_code` | Test was renamed; old name does not exist |
| PAD5 | "renamed from `test_pad_far_frr_benchmark`" inline | same text inside a `*( ... )*` annotation | Historical name, not evidence |
| VZF3 | lists `OrtLandmarkDetector` | list is `OrtFaceDetector`, `OrtEmbeddingExtractor`, `OrtPadDetector` | Detector removed (NGM6); `zeroize_tests` covers the three remaining ones |
| GEPU3 | criterion "**Superseded by GRE1, GRE2, GRE7 and GRE8**", status `✅ Verified`, evidence `SoosApp::*` methods | status `⏹ Superseded (GRE1, GRE2, GRE7, GRE8)` | Self-contradiction; the cited methods are neither tests nor present |
| BIO1 | `embedding_tests::test_prepare_input_bgr_channel_ordering` | `embedding_tests::test_arcface_input_bgr_ordering` | The old name is the SCRFD test (`scrfd_tests`); the ArcFace BGR test asserts channel 2 = R |
| CAM1 | `config_tests::test_config_sensor_preference_and_idle_timeout`, `pipeline_init_tests::test_pipeline_init_device_selection_prefers_ir` | `config_tests::test_pipeline_default_sensor_preference_is_prefer_ir`, `config_tests::test_pipeline_config_idle_timeout_zero_from_toml`, `dual_sensor_tests::test_dual_sensor_override_prefers_ir` | Cited tests never existed; replacements assert the daemon default, the idle timeout key and IR device selection |
| CAM2 | `pam_config_tests::test_default_timeout_is_1000ms` | `soos-pam::config_tests::test_default_config_on_null_argv` | Never existed; the PAM config test asserts `timeout_ms == 1000` |
| PRX1 | `preview_tests::test_preview_request_and_response_roundtrip`, `preview_tests::test_preview_frame_large_payload_rejected`, `dispatcher_tests::test_dispatcher_handles_preview_frame_request` | `preview_tests::test_preview_frame_request_and_response_roundtrip`, `preview_tests::test_preview_codec_rejects_oversized_payload`, `dispatcher_tests::test_dispatcher_preview_frame_roundtrip` | Names drifted; the replacements assert the round-trip, the `MAX_PREVIEW_MESSAGE_SIZE` rejection and the dispatcher path |
| PRX2 | `crates/gui/src/ipc_camera.rs`, `crates/gui/src/main.rs`, `cargo test -p soos-gui` | `ipc_camera_tests::test_probe_preview_accepts_authorized_reply`, `..._stops_on_unauthorized_response`, `..._reports_io_error_when_socket_absent`; arbitration via GRE7 | Production sources are not tests; `ipc_camera_tests` drives `IpcCameraManager` over a temporary unprivileged socket |
| ASG3 | `dual_sensor_tests::test_sensor_preference_defaults_to_prefer_ir`, `..._v4l2_31_char_truncated_name_classified_as_ir` | `dual_sensor_tests::test_default_sensor_preference_is_prefer_ir`, `dual_sensor_tests::test_sensor_classification_truncated_ir_card_name` | Names drifted; replacements assert `PreferIr` and the `USB2.0 FHD UVC WebCam: USB2.0 I` case |
| ASG4 | `config_tests::test_daemon_pipeline_decision_budget_calibrated_for_warmup`, `config_tests::test_daemon_camera_config_defaults_prefer_ir`; criterion "wake timeout 800ms" | `config_tests::test_decision_budget_calibrated_to_pam_deadline`, `config_tests::test_pipeline_default_sensor_preference_is_prefer_ir`; 800 ms marked as drift | Cited tests never existed; no 800 ms constant exists (the wake wait is bounded by the request deadline and `connection_timeout`, DIB4) |
| LSF3 | `config_tests::test_send_pam_info_null_safe`, `config_tests::test_send_pam_info_low_address_guard` | `config_tests::test_authenticate_with_none_handle_returns_ignore_cleanly`, `pam_bindings_tests::test_pam_hooks_authenticate_offline_daemon_returns_ignore`; texts via `pam_handle_tests` (PDM3) | Never existed; the replacements drive `send_pam_info` with no handle and with a `0x1000` dummy handle (the guard prevents a dereference) |
| GARP1 | `device_resolution_tests::test_resolve_camera_device_auto_resolution`, `..._default_resolution` | `..._auto_resolution_with_config`, `..._ignores_literal_default_in_cli_arg` | Names drifted; replacements assert `auto` / `default` resolve under `/dev` |
| GARP2 | `soos_camera_v4l::v4l_impl`, `tests/physical_hardware_tests.rs`; `✅ Verified` | `⬜ Pending (no test evidence; manual check on real hardware required)` | The file never existed and the module is production code; no test exercises the `meta.bytesused` slice |

Items of the issue already resolved on `main` before this branch and left as they are: PAD1, NGM9,
ASG1 (live class index 1, enforced by PTF4/PTF5), `AI/DECISIONS.md` line 20 (marked superseded),
`Docs/INFERENCE_ORT_CRATE.md`, LSF1 (walkthrough 98), PK5–PK7 (`test_packages.sh` in CI for
Ubuntu, Fedora/Arch pending).

## 6. Traceability

- `AI/VERIFICATION_MATRIX.md`: rows above edited in place; new section
  `verification-matrix-citations` (MXC1–MXC3) inserted after the PTF table (not appended, to avoid
  conflicts with parallel branches).
- `.agents/skills/dev-workflow/references/project-facts.md` §6 and
  `.agents/skills/traceability-agent/SKILL.md`: citation rule, `*( ... )*` annotations,
  `⬜ Pending (<reason>)` / `⏹ Superseded (<rows>)` markers.
- No `AI/BACKLOG.md` or `scripts/sync_issue.py` change: review-issue branches are not registered.

## 7. Validation

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features --no-fail-fast
./scripts/candid_review.sh
```

Results are recorded in the final report of the branch; the new tests are green after the row
corrections (10/10 `matrix_` tests of `soos-invariants`).

# Tester Contract — GitHub #367 (first PR of #360): macOS Port Foundation

- **Date**: 2026-10-13
- **Branch**: `chore/macos-port-foundation`
- **Spec**: `AI/architect_spec_macos_port_foundation.md` (Round 3)
- **Red run**: `cargo test --locked -p soos-invariants --all-features portable_crates_contract` → 3 passed, 10 failed
  (the 3 green tests check the guard itself and are expected green at Red, spec §9).

| Test (path::name) | Acceptance / matrix | Red evidence |
|---|---|---|
| `tests/invariants/src/portable_crates_contract.rs::test_portable_crates_have_no_linux_only_dependencies` | A7 / MAC3 | `cargo tree failed for soos-frame (fail-closed): error: cannot specify features for packages outside of workspace` (and `soos-vision reaches ["soos-camera-v4l", "v4l", "v4l2-sys-mit"]` on today's graph) |
| `…::test_mac_forbidden_hits_is_exact` | A7 / MAC4 | green at Red (pure guard logic, X-1 root exclusion) |
| `…::test_mac_guard_query_sees_linux_only_crates` | A7 / MAC4 | green at Red (witness per forbidden group) |
| `…::test_mac_guard_detects_a_forbidden_crate_in_a_broken_manifest` | A7 / MAC4 | green at Red (hermetic stub `v4l`) |
| `…::test_mac_frame_crate_is_pure_and_forbids_unsafe` | A2 / MAC2 | `read crates/frame/Cargo.toml: No such file or directory` |
| `…::test_mac_camera_v4l_reexports_the_frame_types` | A3 / MAC2 | `soos-camera-v4l depends on soos-frame` assertion |
| `…::test_mac_vision_and_fixtures_do_not_depend_on_camera_v4l` | A3 / MAC2 | `crates/vision/Cargo.toml must not depend on soos-camera-v4l` |
| `…::test_mac_business_lists_include_frame` | A5 / MAC2 | `candid_review.sh BUSINESS_CRATES lists frame` |
| `…::test_mac_macos_job_is_pinned_secret_free_and_non_required` | A9 / MAC5 | `ci.yml must define the portable-macos job` |
| `…::test_mac_pr_loop_treats_only_the_macos_job_as_advisory` | A9 / MAC5 | `read scripts/pr_checks_gate.sh: No such file or directory` |
| `…::test_mac_pr_checks_gate_decisions` | A9 / MAC5 (R2-1, R2-3) | `case adv_fail_rest_pass`: exit 127 ≠ 0 (helper absent) |
| `…::test_mac_adr_records_the_separate_repository_decision` | A1 / MAC1 | `AI/DECISIONS.md must record "macOS Port in a Separate Repository"` |
| `…::test_mac_macos_port_doc_covers_the_bootstrap_spec` | A8, A10 / MAC6 | `read Docs/MACOS_PORT.md: No such file or directory` |
| `crates/frame/tests/frame_contract_tests.rs::{test_frame_pixel_format_fourcc_strings_are_unchanged, test_frame_expected_buffer_sizes_are_unchanged_and_saturating, test_frame_new_defaults_to_unknown_sensor_and_keeps_metadata, test_frame_age_saturates_and_staleness_is_strict, test_frame_zeroize_empties_data_and_keeps_capacity, test_frame_types_keep_their_derives}` | A2 / MAC2 | compile error: package `soos-frame` does not exist (missing item = exactly the specified API) |
| `crates/camera-v4l/tests/frame_drop_wipe_tests.rs::{test_frame_buffer_wiped_on_drop, test_drop_probe_detects_an_unwiped_buffer, test_camera_frame_types_are_the_soos_frame_types}` | A3, A4 / MAC2 | compile error: unresolved crate `soos_frame` |

### Migrated existing tests (import paths only, assertions untouched)

| ID | Test | Old | New | Mandate |
|---|---|---|---|---|
| AM-1 | `tests/invariants/src/lib.rs::test_business_crates_forbid_unsafe_code` | 12 crates | + `"frame"` (strengthened) | #360 AC "soos-frame is added to the business-crate list" |
| AM-2 | `crates/vision/tests/{ir_sensor_policy_tests, pipeline_tests, color_tests, bench_tests, frame_robustness_tests, ir_pad_policy_tests, face_quality_gate_tests, gcv_review_tests, zeroize_tests, pad_crop_real_model_tests, embedding_lfw_evaluation_tests, mjpeg_bounds_tests, pad_ensemble_tests}.rs` | `use soos_camera_v4l::…;` | `use soos_frame::…;` (same items) | #360 AC "soos-vision no longer depends on soos-camera-v4l" and "import-path updates" |
| AM-3 | `tests/fixtures/tests/fixture_generators_tests.rs` | `use soos_camera_v4l::PixelFormat;` | `use soos_frame::PixelFormat;` | spec D-8 |

`git diff -U0` over the AM-2/AM-3 files shows exactly 14 `-use soos_camera_v4l::` / `+use soos_frame::` pairs and
nothing else.

### Flakiness check

`test_mac_pr_checks_gate_decisions` and `test_mac_guard_detects_a_forbidden_crate_in_a_broken_manifest` use
per-process temporary directories; the drop-wipe tests serialise on a mutex and key the probe on a buffer address. Run
10× after Green (see the developer report).

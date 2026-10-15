# Tester Contract — GitHub #378 (PR 1 of #353): UVC XU IR emitter control and strobe classification

- **Date**: 2026-10-13
- **Branch**: `feat/ir-antispoofing-mock`
- **Spec**: `AI/architect_spec_ir_emitter_control.md` (Revisions 2 and 3); plan evaluation round 3 APPROVED.

## Contract

| Test file (all names `test_ira_*` / `prop_ira_*`) | Rows | Red evidence |
|---|---|---|
| `crates/camera-v4l/tests/ir_emitter_config_tests.rs` (20 tests incl. 1 proptest) | IRA1–IRA4 | `error[E0432]: unresolved import soos_camera_v4l::ir_emitter_config` (the specified module) |
| `crates/camera-v4l/tests/uvc_xu_tests.rs` (11, `mock-camera`) | IRA5–IRA8 | `unresolved import soos_camera_v4l::uvc_xu` / `ir_mock` |
| `crates/camera-v4l/src/uvc_xu.rs` unit tests (2) | IRA5 | written with the module (layout of a private kernel mirror) |
| `crates/camera-v4l/tests/ir_emitter_tests.rs` (10, `mock-camera`) | IRA9–IRA11 | `unresolved import soos_camera_v4l::ir_emitter` |
| `crates/camera-v4l/tests/strobe_classification_tests.rs` (17 incl. 2 proptests) | IRA12–IRA15 | `unresolved import soos_camera_v4l::strobe`, `soos_test_fixtures::ir_strobe` |
| `crates/camera-v4l/tests/ir_mock_camera_tests.rs` (4, `mock-camera`) | IRA11 | `unresolved import soos_camera_v4l::ir_mock` |
| `crates/daemon/tests/ir_emitter_daemon_config_tests.rs` (4) | IRA16 | `no field ir_emitter on PipelineConfig` |
| `crates/admin-cli/tests/ir_emitter_command_tests.rs` (16) | IRA17–IRA21 | `unresolved import soos_admin_cli::ir_emitter` |
| `tests/invariants/src/ir_emitter_contract.rs` (5) | IRA22–IRA24 | missing files `crates/camera-v4l/src/uvc_xu.rs`, ADR title absent |

Red was observed as compile errors on exactly the specified API (no stub needed); the camera-v4l run
reported 11 `E0432` unresolved imports over the five new test targets.

Fixture: `soos-test-fixtures::ir_strobe` (synthetic Grey `Infrared` sequences: steady, native
alternating, controllable, unresponsive, absent, class change; non-biometric).

## Migrated existing tests

None. No existing test was modified. One fixture datum of a *new* test was corrected during Phase 4:
the RGB node of `ir_emitter_command_tests` reported the IR frame-size signature (640×360) and was
therefore classified Infrared by the shared scorer; it now reports 1280×720 (assertions untouched).

## Candid-review amendments (Revision 4, tests first)

New tests, written before the fix and observed Red (`E0432` unresolved `ignored_camera_keys`,
`XuBudget`, `ensure_same_node`; `E0425` `MAX_IGNORED_CAMERA_KEYS`):
`ir_emitter_config_tests::test_ira_other_camera_keys_are_ignored_and_reported`,
`ir_emitter_daemon_config_tests::test_ira_daemon_loads_with_unknown_camera_key`,
`uvc_xu_tests::test_ira_budget_refuses_requests_beyond_the_bound`,
`ir_emitter_command_tests::test_ira_configure_device_test_stays_within_the_enforced_bound`,
`ir_emitter_command_tests::test_ira_opened_descriptor_must_be_the_classified_node`.

Changed tests (all new in this PR, none pre-existing; changed because the review changed the
contract, not to make code pass):

| Test | Old expectation | New expectation | Reason |
|---|---|---|---|
| `ir_emitter_config_tests::test_ira_unknown_keys_are_refused` | `strobe = true` in `[camera]` refused | only an unknown key inside `[camera.ir_emitter]` is refused (assertion kept) | R21 |
| `ir_emitter_daemon_config_tests::test_ira_daemon_refuses_invalid_section` | case `[camera] exposure = 3` refused | case removed; covered by `test_ira_daemon_loads_with_unknown_camera_key` (loads with a warning) | R21 |
| `MAX_DEVICE_TEST_QUERIES` used by `test_ira_device_test_restores_*` | 10 (documented only) | 11, the real enforced maximum (asserted equal in the new bound test) | R22 |

## Flakiness check

`ir_mock_camera_tests` (the only timing-dependent file) uses `step_frame()` on a frozen mock and a
bounded `wait_until`; run 10 times in a row, all green.

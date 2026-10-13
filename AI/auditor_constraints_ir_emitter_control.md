# Audit Constraints — GitHub #378 (PR 1 of #353): UVC XU IR emitter control and strobe classification

- **Date**: 2026-10-13
- **Branch**: `feat/ir-antispoofing-mock`
- **Inputs**: `AI/architect_spec_ir_emitter_control.md` (with Revision 2), `AI/plan_evaluator_report.md`,
  `AI/tester_contract_ir_emitter_control.md`, current code of `crates/camera-v4l`, `crates/daemon/src/config.rs`,
  `crates/admin-cli`.

## Checklist results (greps run on the base before the change)

| # | Item | Result on base |
|---|---|---|
| 1 | Panic paths (`unwrap`/`expect`/`panic!`/indexing) in `crates/camera-v4l/src`, `crates/admin-cli/src`, `crates/daemon/src/config.rs` production code | none outside `#[cfg(test)]` (workspace lints deny them) |
| 2 | `unsafe` in `crates/camera-v4l/src` | existing sites in `v4l_guard.rs`, `v4l_impl.rs`, `mock.rs`, all with `// SAFETY:`; `#![deny(clippy::undocumented_unsafe_blocks)]` in `lib.rs`; `soos-admin-cli` forbids unsafe |
| 3 | Output isolation | not applicable to `crates/pam` (untouched); `soos-admin` prints only from `main.rs` |
| 9 | Supply chain | no new crate: `proptest 1.11.0` and `soos-test-fixtures` are already in `Cargo.lock` (dev edges only); `soos-camera-v4l` `mock-camera` feature for `soos-admin-cli` tests only |
| 10 | CI / workflow changes | none |

## Constraints

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| A1 | Exactly one new `unsafe` block, the `UVCIOC_CTRL_QUERY` ioctl, preceded by `// SAFETY:` stating: the fd is owned by `self.file` and open for the whole call; the struct is `#[repr(C)]` and matches `struct uvc_xu_control_query` (layout asserted by a unit test); `data` points to the caller's exclusively borrowed slice of exactly `size` bytes (1..=64, `GET_LEN` 2, `GET_INFO` 1, checked before the call); the kernel copies at most `size` bytes and keeps no pointer after return | `uvc_xu.rs::UvcXuDevice::query` | invariant `ir_emitter_contract::test_ira_unsafe_is_confined_to_the_xu_ioctl`, clippy `undocumented_unsafe_blocks`, unit test of the layout and request code |
| A2 | No `as` narrowing without proof: `size` is built with `u16::try_from(len)` after the length check; errno mapping via `raw_os_error()` | `uvc_xu.rs` | clippy `cast_possible_truncation` (workspace lint), review |
| A3 | `UvcXuDevice::open` refuses non-V4L2 character devices **before** `open(2)` and re-checks the opened descriptor (`fstat`, char device, major 81) before any ioctl; opened with `O_CLOEXEC \| O_NONBLOCK \| O_NOCTTY` | `uvc_xu.rs::UvcXuDevice::open` | `uvc_xu_tests::test_ira_device_open_refuses_non_v4l2_nodes` |
| A4 | Every request has a bounded buffer (≤ 64 bytes); every scan is bounded (`MAX_PROBE_QUERIES`, `MAX_PROBE_CONTROL_ERRORS`); the device test issues ≤ `MAX_DEVICE_TEST_QUERIES` requests | `uvc_xu.rs::probe_xu_controls`, `admin-cli/src/ir_emitter.rs::test_emitter_on_device` | `test_ira_probe_is_bounded_by_query_budget`, `test_ira_probe_skips_transient_errors_with_a_bound`, `test_ira_device_test_restores_original_with_signals_blocked` |
| A5 | The probe never writes: it wraps the control in `XuReadOnly`; `XuReadOnly` returns `WriteRefused` without forwarding | `uvc_xu.rs`, `admin-cli/src/ir_emitter.rs::probe_ir_emitter` | `test_ira_read_only_wrapper_never_forwards_a_write`, `test_ira_probe_is_read_only_and_reports_controls`, invariant `test_ira_admin_probe_is_read_only` |
| A6 | The device is written only under `--test-on-device`; the original value is restored on every exit path (error, panic through `Drop`), with termination signals blocked through an RAII guard created before the emitter; the restore failure is reported loudly | `admin-cli/src/ir_emitter.rs`, `ir_emitter.rs::UvcXuEmitter` (`Drop`) | `test_ira_failed_set_still_restores`, `test_ira_device_test_restores_after_failure`, `test_ira_configure_dry_run_writes_nothing` |
| A7 | `mode = "auto"` is written only after a successful device test in the same run | `configure_ir_emitter` | `test_ira_auto_requires_a_successful_device_test` |
| A8 | `[camera.ir_emitter]` parsing is fail-closed and never echoes a value; unknown key names are sanitized (`sanitize_display_text`) and truncated to 64 chars | `ir_emitter_config.rs::parse_camera_table`, `daemon/src/config.rs` | `ir_emitter_config_tests::*`, `ir_emitter_daemon_config_tests::*` |
| A9 | Config writer: no-follow bounded read of the original, unique `create_new` temporary, mode ≤ 0644, owner copied before rename, pre-rename identity re-check, temporary removed on failure, directory fsync; result validated through `parse_ir_emitter_text` before writing; no `toml::` in `soos-admin-cli` | `admin-cli/src/ir_emitter.rs::write_ir_emitter_section` | `test_ira_configure_refuses_symlinked_config`, `test_ira_configure_refusals_leave_file_unchanged`, `test_ira_configure_refuses_text_that_would_not_parse`, `test_ira_configure_creates_missing_file`, invariant `test_ira_admin_probe_is_read_only` |
| A10 | No frame, payload, original value or difference data in any `tracing` call or error message of the new code (payload lengths and VID:PID are allowed) | new modules | invariant `test_ira_no_payload_in_logs`, review |
| A11 | Strobe classifier rejects NaN/±inf/out-of-range luma, non-monotonic sequences, too few/many samples; arithmetic in `f64`/`u64` sums with `checked`/`saturating` ops; no indexing panics | `strobe.rs` | `strobe_classification_tests::*` incl. proptests |
| A12 | sysfs reads bounded to `MAX_USB_ID_FILE_BYTES`, node name restricted to `video<N>`, canonical parent directory | `ir_emitter_config.rs::usb_device_id_of_node` | `test_ira_usb_id_is_read_from_sysfs_bounded` |
| A13 | Mocks compiled only with `mock-camera`; no IR type added to `soos-frame` | `lib.rs` | invariant `test_ira_mocks_are_feature_gated_and_frame_is_untouched`, `portable_crates_contract` |
| A14 | Nothing on the authentication path changes; the daemon never opens the XU control; `pam_soos.so` untouched | `daemon/src/config.rs` | diff review, `ir_emitter_daemon_config_tests::*` (warning that it is not driven) |

## Candid-review additions (Revision 4)

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| A15 | Unknown `[camera]` keys never stop the daemon; warned by name only | `ir_emitter_config::ignored_camera_keys`, `daemon/src/config.rs` | `test_ira_other_camera_keys_are_ignored_and_reported`, `test_ira_daemon_loads_with_unknown_camera_key` |
| A16 | `MAX_DEVICE_TEST_QUERIES` (11) enforced in code with `XuBudget` | `admin-cli/src/ir_emitter.rs` | `test_ira_budget_refuses_requests_beyond_the_bound`, `test_ira_configure_device_test_stays_within_the_enforced_bound` |
| A17 | The opened descriptor is the classified node (same `st_rdev`) before any request | `ensure_same_node`, `main.rs` | `test_ira_opened_descriptor_must_be_the_classified_node` |

## Pre-existing violations found (not introduced by this change)

None relevant. Note: `[camera]` keys were silently ignored by `soos-daemon` before (no
`deny_unknown_fields` at the top level); this change makes `[camera]` strict (documented upgrade note).

## Clearance: CLEARED

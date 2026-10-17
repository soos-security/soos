# Tester Contract — GitHub #405: GNOME Shell Unlock-Dialog Extension `soos-lockscreen@soos`

- **Date**: 2026-10-17 — **Branch**: `feat/lockscreen-gnome`
- **Spec**: `AI/architect_spec_lockscreen_gnome.md` (plan self-check APPROVED, `AI/plan_evaluator_report.md`).
- All tests are new; no existing test or assertion was changed. The only edit to an existing test file is one
  `mod lockscreen_gnome_contract;` declaration (with its doc comment) in `tests/invariants/src/lib.rs`.
- Red was proven against stub files (`metadata.json` = `{}`, `extension.js` and `stylesheet.css` holding a
  one-line comment) so that every failure is an assertion on the specified content, not a missing file.

| Test file | Tests | Matrix | Red evidence |
|---|---|---|---|
| `tests/invariants/src/lockscreen_gnome_contract.rs` | `test_lkg1_metadata_uuid_session_modes_and_versions` | LKG1 | failed (no uuid) |
| same | `test_lkg2_esm_shape_and_imports`, `test_lkg2_syntax_check_with_node_when_available` | LKG2 | first failed (no default export); the node check passed on the stub (a comment is valid syntax): it is a guard against syntax errors, not a feature test |
| same | `test_lkg3_no_unlock_path_and_no_input_signal` | LKG3 | failed (no non-reactive card) |
| same | `test_lkg4_subprocess_without_shell_and_no_network` | LKG4 | failed (no spawn site) |
| same | `test_lkg5_three_states_three_fields_bounded_lines` | LKG5 | failed (no key list) |
| same | `test_lkg6_disable_cleans_up_and_callbacks_are_guarded` | LKG6 | failed (no `disable()`) |
| same | `test_lkg7_kill_switch_and_reduced_motion`, `test_lkg7_extension_change_set_leaves_pam_daemon_protocol_and_bridge_alone` | LKG7 | first failed (no kill switch); the change-set guard passed (the branch touches none of the guarded crates): a guard, not a feature test |
| same | `test_lkg8_palette_tokens_and_star_mirror_their_sources`, `test_lkg8_documentation_and_adr_are_present` | LKG8 | both failed (no colours; no documentation) |

Red run: `cargo test --locked -p soos-invariants lockscreen_gnome -- --test-threads=3` → 9 failed, 2 passed (the two
guards above).

Behaviour outside CI (not part of the contract, recorded for the walkthrough): a Node harness with mocked shell
modules drove the real `extension.js` through lock, Scanning with quarter turns stopping on a half turn after 4.8 s,
a split line, Not recognized with one shake, `available:false` as Idle, rejected lines (extra key, unknown state,
array, non-JSON, non-boolean), an overlong line (bridge killed, Idle, 1 s restart), EOF (2 s restart), unlock,
kill switch, refused user-owned and group-writable binaries, reduced motion and `disable()`/`enable()` while locked;
a GJS 1.90 run with real `GLib`/`Gio` and a fake `--watch` script exercised `Gio.Subprocess`, `read_bytes_async`,
split lines, EOF, the restart timer and `disable()`.

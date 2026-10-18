# Tester Contract — `soos-gui` motion system (GitHub #438 = PR 3 of #358, `feat/gui-motion`)

- **Date**: 2026-10-18
- **Contract source**: `AI/tester_contract_motion.md` Part B.2 (items 1–10) and B.2a; spec `AI/architect_spec_gui_motion.md`.
- **Files**: `crates/gui/tests/motion_tests.rs` (GMO1–GMO14), `tests/invariants/src/gui_motion_contract.rs` (GMO15–GMO19,
  registered in `tests/invariants/src/lib.rs`), migrated `tests/invariants/src/presence_unlock_contract.rs` (PAU17).

| Test (path::name) | Acceptance line / matrix ID | Red evidence (failure message) |
|---|---|---|
| `soos-gui::motion_tests::test_gmo1_theme_exposes_every_motion_token` | B.2.1 / GMO1 | compile error: `cannot find value EASE_STANDARD in module theme` (and every other new token) |
| `soos-gui::motion_tests::test_gmo2_*` (6 tests) | B.2.2 / GMO2 | compile error: `could not read src/motion.rs` (the specified module) |
| `soos-gui::motion_tests::test_gmo3_*` (3 tests) | B.2.3 / GMO3 | same (missing `motion::cubic_bezier`, `Tween`, `mix_color`) |
| `soos-gui::motion_tests::test_gmo4_*` (2 tests) | B.2.4 / GMO4 | same (missing `resolve_motion_prefs`, `GuiArgs::reduce_motion`) |
| `soos-gui::motion_tests::test_gmo5_*` (6 tests) | B.2.5 / GMO5 | compile error: `could not read src/motion/portal.rs` |
| `soos-gui::motion_tests::test_gmo6_*` (2 tests) | B.2.6 / GMO6 | missing `motion::install_prefs` and helpers |
| `soos-gui::motion_tests::test_gmo7_*` (4 tests) | B.2.7 / GMO7 | compile error: missing `motion::pending_star_rotation`, `theme::AMBIENT_REPAINT_MS` |
| `soos-gui::motion_tests::test_gmo8_face_overlay_is_never_interpolated` | B.2.8 / GMO8 | compile error: `could not read src/overlay.rs` |
| `soos-gui::motion_tests::test_gmo9_spinners_replaced_and_authorization_text_visible` | B.2.9 / GMO9 | compile error of the test file (missing `motion::install_prefs`, `src/motion.rs`); on the base tree `header.rs:374` and `camera_status.rs:154` still call `egui::Spinner` |
| `soos-gui::motion_tests::test_gmo10_motion_module_has_no_panic_path_or_allocation` | B.2.10 / GMO10 | compile error: `could not read src/motion.rs` |
| `soos-gui::motion_tests::test_gmo11_tab_pill_slides_between_tabs` | G3 / GMO11 | compile error: `cannot find function tab_pill in module header` |
| `soos-gui::motion_tests::test_gmo12_*` (3 tests) | G4 / GMO12 | missing `motion::entrance_at`, `entrance`, `fade_in_on_change` |
| `soos-gui::motion_tests::test_gmo13_star_turn_and_rotation_at_rest` | G8 / GMO13 | compile error: `cannot find function paint_star_rotated in module soos_gui::brand`, `star_tile_turned` |
| `soos-gui::motion_tests::test_gmo14_motion_control_reads_the_portal_only_when_needed` | B.2.4–5 / GMO14 | missing `motion::MotionControl` |
| `soos-invariants::gui_motion_contract::test_gmo15_zbus_is_confined_to_the_portal_settings_reader` | B.2a / GMO15 | `Cannot read crates/gui/src/motion/portal.rs: No such file or directory` |
| `soos-invariants::gui_motion_contract::test_gmo16_tokio_runtime_only_on_the_reader_thread` | B.2a / GMO16 | same |
| `soos-invariants::gui_motion_contract::test_gmo17_gui_manifest_declares_workspace_zbus_and_tokio` | B.2.5 / GMO17 | `assertion left == right failed: crates/gui/Cargo.toml: zbus = { workspace = true }` |
| `soos-invariants::gui_motion_contract::test_gmo18_portal_value_is_never_logged` | A8 / GMO18 | `Cannot read crates/gui/src/motion/portal.rs` |
| `soos-invariants::gui_motion_contract::test_gmo19_security_guideline_names_the_portal_exception` | B.2a / GMO19 | `Docs/SECURITY_AND_QUALITY_GUIDELINES.md D-Bus line must mention soos-gui` |
| `soos-invariants::gui_motion_contract::test_gmo_helper_self_test` | helpers can fail | green by design |

All `soos-gui` rows fail to compile because the test file needs the specified API (accepted by the tester skill when
the missing item is exactly the specified API). Red run (base `3f76090` plus the tests): `cargo test --locked -p soos-invariants --all-features gmo test_pau_zbus_is_used_only_by_the_daemon`
→ `1 passed; 6 failed`; `cargo test --locked -p soos-gui --all-features --test motion_tests --no-run` → `could not compile
soos-gui (test "motion_tests") due to 91 previous errors`, every one a missing specified item (new modules, tokens,
`header::tab_pill`, `brand::paint_star_rotated`, `widgets::star_tile_turned`, `widgets::star_turn_degrees`).

Two assertions of the new file were corrected before the implementation, while the tests were still Red (no contract
on `origin/main` was touched): `Context::style()` does not exist in egui 0.35 (`global_style()` is used), and the
ambient-repaint check accounts for egui reporting the requested `request_repaint_after` duration minus the predicted
frame time (`is_ambient_repaint`: never immediate, never absent, at least 33 ms once the frame time is added back).

### Migrated existing tests

| Test | Old assertion | New assertion | Mandating acceptance line |
|---|---|---|---|
| `soos-invariants::presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon` (PAU17) | `ALLOWED: [&str; 2]` = `crates/daemon/Cargo.toml`, `crates/remote/Cargo.toml` | `ALLOWED: [&str; 3]` adds `crates/gui/Cargo.toml`; the per-manifest assertions (exactly `zbus = { workspace = true }` in `[dependencies]`, zbus named once), the scan of every other manifest, the PAM assertion, the messages and the test name are unchanged | Planned Contract Migration B.2a of `AI/tester_contract_motion.md`; ADR 2026-10-13 "Motion System for soos-gui and soos-remote" item (6); #358 owner decision 4 of 2026-10-13 |

No other existing test was modified; `responsiveness_tests.rs`, `theme_tests.rs`, `brand_layout_tests.rs`,
`readme_snapshots.rs` and `card_id_stability_tests.rs` are unchanged and green.

### Flakiness check

`cargo test --locked -p soos-gui --all-features --test motion_tests -q -- --test-threads=3`, 10 runs in a row: 10/10
passed (2026-10-18). The wall-clock tests (GMO5) only bound waits from above by the deadline plus 2 s, and from below
by the deadline itself through the instant passed to `poll_at`.

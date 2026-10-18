# Walkthrough 234 — soos-gui Motion System With Portal-Driven Reduced Motion

- **Date**: 2026-10-18
- **Issue**: GitHub #438 = PR 3 of epic #358 (GitHub-only, no backlog id) — **Branch**: `feat/gui-motion`
- **Decision**: ADR 2026-10-13 "Motion System for soos-gui and soos-remote" items (2), (3), (5), (6), (8); no new ADR
  (item (6) already records the zbus widening and the PAU17 migration)
- **Matrix criteria**: GMO1–GMO19 (new, ✅), GMO20 (owner desktop check, ⬜), UIM9 (→ ✅); UIM7 (owner approval of the
  prototype) and UIM11 (owner GNOME Wayland / X11 check) stay pending
- **Merge gate**: not merged before the owner approves `Docs/prototypes/ui_motion_prototype.html` (UIM7, owner decision 3
  of 2026-10-13 on #358)

## 1. Context & Objectives

PR 1 of #358 (#377, walkthrough 203) fixed the motion specification `Docs/UI_MOTION.md`, the tokens, the spring and the
tester contract. This PR implements the GUI part: §7 (components G1–G10), §8 (reduced motion) and §9 (repaint budget),
following `AI/tester_contract_motion.md` Part B.2 and the Planned Contract Migration B.2a. The web part is #433
(`feat/remote-motion`).

## 2. Architect Design (`AI/architect_spec_gui_motion.md`)

- **Tokens** in `theme.rs`, as plain literals so UIM-S2 now cross-checks them live.
- **`motion.rs`, pure part**: `clamp_dt`; `SpringState` stepping the closed-form damped oscillator (exact for any
  `dt`, `dt` clamped to 0.05 s, non-finite input harmless, retarget keeps the velocity, retargeting to the same target
  is a no-op, a settled spring snaps exactly to its target, a 1.6 s termination guard); `cubic_bezier` (Newton then
  bisection, exact endpoints); `Tween`; `mix_color` (exact at both ends); `entrance_at` and `stagger_delay`;
  `pending_star_angle`; `MotionPrefs`, `PortalPreference`, `parse_reduce_motion_env`, `resolve_motion_prefs`.
- **`motion.rs`, egui glue**: one `Copy` state per animated widget in egui temp storage, mutated in place
  (`get_temp_mut_or_insert_with`, no per-frame allocation), stepped at most once per pass; `install_prefs` /
  `prefs` (nothing installed = reduced); `spring_value`, `spring_peek`, `tween_value`, `entrance` (an element not
  drawn in the previous pass enters, so a tab change enters the new tab while a poll refresh never replays),
  `fade_in_on_change`, `pending_star_rotation`, `push_opacity` / `with_opacity`; `MotionControl` (fixed, or from the
  switch, the variable and the portal reader).
- **`motion/portal.rs`**: `SettingsBus` (injectable), `PortalReader` (one named thread, `sync_channel(1)`, `try_recv`
  only, the deadline enforced by the UI side too, cached result, no retry), `interpret`, and `ZbusPortalBus` (one
  current-thread Tokio runtime, one session connection, `ReadOne` then `Read` on UnknownMethod, one overall
  `tokio::time::timeout`, the reply validated to one `u32` or `bool`, nothing logged).
- **Widgets**: daemon switch (spring thumb, track cross-fade, pending star instead of the spinner), sliding tab shape
  (`header::tab_pill`, labels cross-faded over `T_FAST`), right-column entrances per tab, stat value fade-in when what
  the tile shows changes, banner entry, kind cross-fade and `banner_slot` exit (daemon result), progress easing,
  camera status (pending star, new text at once with a fade-in, title colour cross-fade), star tiles turned 45° on a
  confirmed paused daemon (`widgets::star_tile_turned`, `brand::paint_star_rotated`), face overlay moved verbatim
  into `overlay.rs` with no motion, `Style::animation_time` = `T_SHORT` or 0.
- **Wiring**: `GuiArgs::reduce_motion`; `main.rs` calls `configure_motion(args.reduce_motion,
  var_os("SOOS_REDUCE_MOTION"))`; `SoosApp::new` and the README capture start with `MotionControl::fixed(REDUCED)`;
  `SoosApp::ui` calls `self.motion.apply(&ctx)` first.

Plan evaluation (`AI/plan_evaluator_report.md`, self-check, APPROVED) recorded three findings: G7 of
`Docs/UI_MOTION.md` asked the old camera status to fade out before the new one appears, which would keep stale text on
screen (principle 6, security truth first) — the new text is drawn at once and fades in, and G7 was reworded; G4
"stat values cross-fade" would have faded a live match score at camera rate (flicker and continuous repaints) — the
fade runs only when the caption or the presence of the value changes, and G4 was reworded; banner state uses hashed
slots (1024, collision-safe) so its memory stays bounded.

## 3. Tester Contract (Red) — `AI/tester_contract_gui_motion.md`

- `crates/gui/tests/motion_tests.rs`: 33 tests (GMO1–GMO14) with a headless `egui::Context` driven through
  `RawInput::predicted_dt`; repaints are read from `ViewportOutput::repaint_delay`; shapes from `FullOutput::shapes`.
- `tests/invariants/src/gui_motion_contract.rs`: GMO15–GMO19 plus a helper self-test.
- Red: the invariants failed `1 passed; 6 failed` (missing `portal.rs`, the manifest lines, the guideline needles,
  and the migrated PAU17 on the missing `crates/gui/Cargo.toml` zbus line); the GUI test file failed to compile with
  91 errors, every one a missing specified item.
- Contract Migration B.2a: PAU17 `ALLOWED` gains `crates/gui/Cargo.toml`; nothing else in that test changed.

## 4. Auditor Constraints — `AI/auditor_constraints_gui_motion.md`

A1–A15: no `unsafe`, no panic path (module-level `deny` in `motion.rs`), every float sanitized, no allocation in
`motion.rs`, the UI thread never blocks, one thread / one connection / at most four calls / one deadline, a read-only
D-Bus surface, the value validated and never logged, the variable read once and compared byte-exactly, animation never
leading the confirmed state, the repaint budget, bounded state, unchanged rest-state rendering (README capture),
no process spawn, no new lockfile package.

## 5. Developer Notes

- The pill is painted before the tabs and every widget at rest paints exactly its old shapes (exact end colours,
  opacity 1 changes nothing, angle 0 takes the old texture path), so `readme_snapshots` (RAC1, RAC2) stays green
  unmodified; the capture scenes also run with reduced motion.
- egui reports `request_repaint_after(d)` as `d` minus the predicted frame time; the tests account for it
  (`is_ambient_repaint`).
- `crates/gui/Cargo.toml` names zbus exactly once (PAU17) and its comment avoids the word "soos-remote" (RMC-S4 scans
  every manifest for that crate name).
- The face overlay loop moved from `app.rs` to `overlay::paint_detections` byte-for-byte in behaviour (same order and
  arithmetic); its colour comes only from `frame.pad_live`.
- `Cargo.lock`: only the `soos-gui` dependency list gains `tokio` and `zbus`; no package added (`cargo deny --locked
  check`: advisories, bans, licenses, sources ok).

## 6. Validation

| Command (with `LIBONNXRUNTIME_NO_PKG_CONFIG=1`, scoped target dir, 3 jobs) | Result |
|---|---|
| `cargo test --locked -p soos-gui --all-features -- --test-threads=3` | all suites green (motion_tests 33, readme_snapshots 9, responsiveness 11, theme 15, brand_layout 9, …) |
| `cargo test --locked -p soos-invariants --all-features -- --test-threads=3` | 793 passed, 0 failed |
| `cargo clippy --locked -p soos-gui -p soos-invariants --all-targets --all-features -- -D warnings` | clean |
| `cargo fmt -p soos-gui -p soos-invariants -- --check` | clean |
| `cargo deny --locked check` | ok |
| `motion_tests` 10 runs in a row | 10/10 |

The full workspace gates run in CI (`./save.sh` was not used: it requires the candid review report, which this run does
not write). The real session-portal read and the visual result on GNOME Wayland / X11 are owner checks (GMO20, UIM11):
the zbus path is exercised in CI only through its static contract (GMO15–GMO18) and the fake buses.

## 7. Documentation

`Docs/GUI_APPLICATION.md` §1, §5.2, §5.3, §5.5; `Docs/UI_MOTION.md` intro, G4, G7;
`Docs/SECURITY_AND_QUALITY_GUIDELINES.md` "Audited D-Bus client" (the read-only `soos-gui` portal exception, still
naming `soos-remote`); `AI/VERIFICATION_MATRIX.md` (component `gui-motion`, UIM9, PAU17 annotation).

# Auditor Constraints — `soos-gui` motion system (GitHub #438 = PR 3 of #358, `feat/gui-motion`)

- **Date**: 2026-10-18
- **Inputs**: `AI/architect_spec_gui_motion.md`, `AI/plan_evaluator_report.md` (APPROVED), tests
  `crates/gui/tests/motion_tests.rs` (GMO1–GMO14) and `tests/invariants/src/gui_motion_contract.rs` (GMO15–GMO19),
  migrated PAU17.
- **Verdict**: CLEARED for implementation under the constraints below.

| # | Constraint | Checked by |
|---|---|---|
| A1 | `motion.rs`, `motion/portal.rs` and `overlay.rs` start with `#![forbid(unsafe_code)]`; `motion.rs` also denies `clippy::indexing_slicing`, `unwrap_used`, `expect_used`, `panic` at module level | GMO10, clippy `-D warnings` |
| A2 | No `unwrap`/`expect`/`panic!`/indexing in the portal reader either; every zbus or Tokio error maps to "no value" (never a panic, never a retry) | clippy, GMO5 |
| A3 | Every float input of the pure helpers is sanitized: `dt` through `clamp_dt`, targets and states checked with `is_finite`, easing input clamped, outputs finite | GMO2, GMO3, GMO7 |
| A4 | No allocation in `motion.rs` production code (no `Vec`, `String`, `Box`, `format!`, `collect`, `insert_temp`); egui state mutated in place with `get_temp_mut_or_insert_with` | GMO10 |
| A5 | The UI thread never blocks: `PortalReader::poll` uses `try_recv` only; no `join`, `recv`, `recv_timeout` or lock held across a frame on the UI side | GMO5, review |
| A6 | One reader thread per process start at most, one `sync_channel(1)`, one session connection, at most four method calls, one overall `tokio::time::timeout` of `PORTAL_READ_DEADLINE_MS`; the runtime is dropped with the thread | GMO15, GMO16, review |
| A7 | The D-Bus surface is read-only: destination, path, interface and method names are constants; no object server, well-known name, match rule, proxy or signal stream; the session bus address comes from zbus's standard session lookup | GMO15 |
| A8 | The portal answer is validated to one `u32` (0 or 1) or one `bool`; anything else is "no value"; the value is never logged (no logging macro in the reader) | GMO5, GMO18 |
| A9 | `SOOS_REDUCE_MOTION` is read once at start-up as an `OsStr`, compared byte-exactly to `1` / `0`, ignored beyond 8 bytes; it is never logged | GMO4 |
| A10 | Animation never leads the confirmed state: the star turns only on a confirmed `DaemonState::Inactive`; the authorization text stays painted while pending; the camera status text is replaced at once; the face overlay uses only the frame being painted | GMO8, GMO9, review |
| A11 | Repaint budget: `request_repaint` only while an animation is unsettled; the pending star uses `request_repaint_after(AMBIENT_REPAINT_MS)` only; reduced motion requests nothing | GMO6, GMO7 |
| A12 | Bounded memory: animation state ids are bounded (fixed widget ids, 64 hashed banner buckets); the banner exit keeps at most one cloned text per slot | review |
| A13 | Rest-state rendering is unchanged (opacity 1, offset 0, exact end colours, angle 0 = old texture path), so the README capture (RAC1/RAC2) and the brand tests stay green unmodified | RAC1, RAC2, `theme_tests.rs`, `brand_layout_tests.rs` |
| A14 | No process is spawned; `app.rs` still contains no `Command::new` | GMO5, `responsiveness_tests.rs` |
| A15 | `Cargo.lock` gains no package: only the `soos-gui` dependency list changes (`tokio`, `zbus`); `cargo deny` stays green | `git diff Cargo.lock`, CI Security job |

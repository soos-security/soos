# Architect Spec — `soos-gui` motion system (GitHub #438 = PR 3 of #358, `feat/gui-motion`)

- **Date**: 2026-10-18
- **Base**: `origin/main` `3f76090`
- **Normative sources**: `Docs/UI_MOTION.md` §2, §3, §5, §7–§9; `AI/tester_contract_motion.md` Part B.2, B.2a, B.3;
  ADR "[2026-10-13] Motion System for soos-gui and soos-remote" items (2), (3), (5), (6), (8), (9).
- **Out of scope**: GUI dark variant, live `SettingChanged` subscription, web motion (`feat/remote-motion`, #433),
  visual-regression harness, any change to PAM, daemon, IPC, camera pipeline or CSP.
- **Merge gate**: not merged before the owner approves the prototype (matrix UIM7, owner decision 3 of 2026-10-13).

## 1. Affected invariants

| Invariant | Effect |
|---|---|
| `#![forbid(unsafe_code)]` in `soos-gui` | kept in every new module (`motion.rs`, `motion/portal.rs`, `overlay.rs`) |
| No `unwrap`/`expect`/indexing in motion code | `motion.rs` carries `#![deny(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used, clippy::panic)]` |
| zbus confined (PAU17) | Planned Contract Migration B.2a: `ALLOWED` gains `crates/gui/Cargo.toml` (exactly `zbus = { workspace = true }`, named once) |
| No Tokio in `pam_soos.so` | untouched: Tokio enters `soos-gui` only, confined to the portal reader thread |
| UI thread never blocks, spawns no process (`responsiveness_tests.rs`) | the UI thread only calls `try_recv`; no `Command::new` anywhere in the motion code |
| README capture determinism (RAC1/RAC2) | the capture scene runs with reduced motion and every widget at rest paints exactly what it painted before |
| No sensitive data logged | the portal value is never logged; `motion/portal.rs` has no logging macro at all |

## 2. Tokens (`crates/gui/src/theme.rs`)

Literal values (UIM-S2 parses the literal after `=`):
`T_FAST 0.12`, `T_SHORT 0.18`, `T_MEDIUM 0.26`, `T_LONG 0.38`, `T_SPRING 0.4`, `T_SHORT_EXIT 0.126`,
`T_MEDIUM_EXIT 0.182`, `T_LONG_EXIT 0.266`, `T_STAGGER 0.03`, `STAGGER_MAX_ITEMS: usize = 6`, `T_PENDING 1.2`,
`EASE_STANDARD [0.2, 0.0, 0.0, 1.0]`, `EASE_ENTER [0.16, 1.0, 0.3, 1.0]`, `EASE_EXIT [0.4, 0.0, 1.0, 1.0]`,
`PRESS_SCALE 0.98`, `RISE_PX 6.0`, `SLIDE_PX 8.0`, `STAR_TURN_DEG 45.0`, `SPRING_STIFFNESS 400.0`,
`SPRING_DAMPING 30.0`, `SPRING_MASS 1.0`, `SPRING_SETTLE_EPSILON 0.001`, `SPRING_SETTLE_VELOCITY 0.1`,
`MAX_MOTION_DT 0.05`, `AMBIENT_REPAINT_MS: u64 = 33`, `PORTAL_READ_DEADLINE_MS: u64 = 200`.
Durations are `f32` seconds, easings `[f32; 4]` cubic-bezier control points. No colour is added.

## 3. `crates/gui/src/motion.rs` (pure part, no allocation, no I/O)

- `clamp_dt(dt) -> f32`: `NaN`, negative, `-inf` → 0; `+inf` and anything above → `MAX_MOTION_DT`.
- `SpringState { value, velocity, target, span, elapsed }` (`Copy`): `new(value)` at rest (non-finite → 0);
  `retarget(target)` ignores a non-finite target and a target equal to the current one (no-op), otherwise keeps
  the velocity, sets `span = |target − value|` and restarts `elapsed`; `step(dt)` advances with the closed-form
  damped-oscillator solution over `clamp_dt(dt)` (displacement and velocity formulas of `Docs/UI_MOTION.md` §3),
  snaps exactly to the target once `settled()`, snaps on any non-finite intermediate, and also snaps after
  `4 × T_SPRING` of simulated time since the last retarget (a termination guard that never shortens a normal
  settle); `settled()` holds when the span is 0, or |target − value| ≤ `SPRING_SETTLE_EPSILON × span` and
  |velocity| ≤ `SPRING_SETTLE_VELOCITY × span`.
- `cubic_bezier(points, t) -> f32`: `NaN` → 0, t clamped to [0, 1], exact endpoints, x(s) = t solved by at most 8
  Newton steps then at most 24 bisection steps; a non-finite result falls back to t.
- `Tween { from, to, elapsed, duration, easing }` (`Copy`): time-based easing; `retarget` starts from the
  currently shown value; `step(dt)`; `settled()` once `elapsed ≥ duration`; a zero or non-finite duration settles
  at once.
- `mix_color(a, b, t)`: exact `a` at t ≤ 0 and `b` at t ≥ 1 (so a settled widget paints its old colours exactly).
- `Entrance { opacity, offset_y }`, `Entrance::REST`; `entrance_at(elapsed, index)`: delay
  `min(index, STAGGER_MAX_ITEMS − 1) × T_STAGGER`, then `EASE_ENTER` over `T_MEDIUM`; opacity 0 → 1 and
  `offset_y` `RISE_PX` → 0.
- `pending_star_angle(time_s) -> f32` (radians): a quarter turn per `T_PENDING`, `EASE_STANDARD` within each
  quarter, taken modulo a half turn (the mark has a two-fold symmetry).
- `MotionPrefs { reduced }` (`MotionPrefs::FULL`, `MotionPrefs::REDUCED`), `PortalPreference { Reduce,
  NoPreference, Unavailable, Pending }`, `REDUCE_MOTION_ENV = "SOOS_REDUCE_MOTION"`,
  `MAX_REDUCE_MOTION_ENV_BYTES = 8`, `parse_reduce_motion_env(Option<&OsStr>) -> Option<bool>` (exactly `1` or `0`;
  more than 8 bytes or anything else → `None`), `resolve_motion_prefs(cli_reduce, env, portal)`: CLI → reduced; env
  `1` → reduced, `0` → full; portal `Reduce` / `Pending` → reduced; `NoPreference` / `Unavailable` → full.

## 4. `motion.rs` egui glue

State lives in egui temp storage as `Copy` values, mutated in place through `get_temp_mut_or_insert_with` (no
per-frame allocation once inserted), stepped at most once per egui pass (`cumulative_pass_nr`), with `dt` =
`clamp_dt(stable_dt)`.

- `install_prefs(ctx, prefs)` stores the prefs and sets `Style::animation_time` to `T_SHORT` (full) or 0 (reduced)
  on every theme style; `prefs(ctx)` returns them, **reduced when none was installed** (pending = reduced).
- `spring_value(ctx, id, target)`, `tween_value(ctx, id, target, duration, easing)`: first sight is at rest on the
  target; reduced snaps to the target; `ctx.request_repaint()` only while unsettled.
- `entrance(ctx, id, index) -> Entrance`: an element not drawn in the previous pass (first sight included) enters;
  an element drawn every pass (poll refreshes) never re-enters; reduced or finished → `Entrance::REST`.
- `fade_in_on_change(ctx, id, key, duration) -> f32`: opacity 0 → 1 (`EASE_ENTER`) when `key` changes; first
  sight and reduced → 1.
- `pending_star_rotation(ctx) -> f32`: reduced → 0 and no repaint; otherwise `pending_star_angle(input.time)` and
  `request_repaint_after(AMBIENT_REPAINT_MS)` (never `request_repaint`).
- `with_opacity(ui, opacity, f)`: multiplies the `Ui` opacity for `f` and restores it (opacity 1 changes nothing).
- `MotionControl`: `fixed(prefs)` (capture scenes and the default) and `from_sources(cli_reduce, env, spawn)`
  (spawns the portal reader only when neither the switch nor the variable decides); `apply(ctx)` polls without
  blocking, installs changed prefs and, while the portal is pending, asks for one repaint after
  `AMBIENT_REPAINT_MS` so the 200 ms deadline is observed even on an idle window.

## 5. `crates/gui/src/motion/portal.rs` (read-only XDG portal reader)

- Constants: destination `org.freedesktop.portal.Desktop`, path `/org/freedesktop/portal/desktop`, interface
  `org.freedesktop.portal.Settings`, methods `ReadOne` then `Read` (only after `org.freedesktop.DBus.Error.UnknownMethod`),
  keys `org.freedesktop.appearance` / `reduced-motion` and `org.gnome.desktop.interface` / `enable-animations`.
- `SettingValue { U32(u32), Bool(bool), Other }`, `PortalReadings { reduced_motion, enable_animations }`,
  `interpret(readings) -> PortalPreference`: `reduced-motion` `U32(1)` → `Reduce`, `U32(0)` → `NoPreference`; any
  other value or no value → `enable-animations` `Bool(false)` → `Reduce`, `Bool(true)` → `NoPreference`; otherwise
  `Unavailable`.
- `trait SettingsBus: Send + 'static { fn read_settings(&mut self, deadline: Duration) -> PortalReadings; }`
  (injectable fake bus for the tests).
- `PortalReader::spawn(bus, deadline)`: one named background thread, one `sync_channel(1)`; a spawn failure is
  `Unavailable` at once. `poll()` / `poll_at(now)`: never block (`try_recv`); cached result, so no retry; empty
  channel before the deadline → `Pending`; after it → `Unavailable`; disconnected (thread ended or panicked
  without answering) → `Unavailable`.
- `ZbusPortalBus`: on the reader thread only, one current-thread Tokio runtime and one zbus session connection
  (`Builder::session()`, `method_timeout`, `max_queued`), at most four `call_method` calls, all under one
  `tokio::time::timeout(deadline)`; no object server, no well-known name, no match rule, no proxy; the reply body
  is decoded as one variant (one nested variant unwrapped for `Read`) and validated to a `u32` or `bool`.
- `PortalReader::spawn_session_portal()` = `spawn(ZbusPortalBus, PORTAL_READ_DEADLINE_MS)`.

## 6. Widgets (G2–G10)

| # | Change |
|---|---|
| G2 | `widgets::toggle_switch`: thumb on `spring_value`, track colour `mix_color(LINE, SUCCESS_TOGGLE, tween T_SHORT EASE_STANDARD)`; `header::daemon_panel` paints the pending star (18 pt, `BLUE` on the pale panel) instead of `egui::Spinner`; the label ("Waiting for authorization...") is painted unchanged |
| G3 | `header::tab_pill(ui, target, index)` paints the pale folder shape at a rect whose two edges tween (`T_MEDIUM`, `EASE_STANDARD`) when the active `index` changes and follow the rect at once otherwise (a resize never makes the shape trail its tab); `header::tab` no longer paints the background and cross-fades its icon and label colours (`T_FAST`); the pill is painted first, so the z-order at rest is the old one |
| G4 | the right-column card, stat tile and star tile of each tab enter with `entrance(ctx, (tab, slot), index)` (opacity and `RISE_PX`, stagger `T_STAGGER`); `stat_tile` fades its value in (`T_SHORT`) when its caption or the presence of its value or subtitle changes; live numeric updates are drawn at once (no count-up, no fade at camera rate) |
| G5 | `widgets::banner` enters (opacity and `SLIDE_PX` from above, `T_SHORT`, `EASE_ENTER`) and cross-fades its colours on a kind change; `widgets::banner_slot` adds the exit (`T_SHORT_EXIT`, `EASE_EXIT`) for a notice that is cleared (daemon result); nothing auto-dismisses |
| G6 | `widgets::progress_bar` with `animate`: `tween_value(T_MEDIUM, EASE_STANDARD)` instead of the fixed 0.25 s |
| G7 | `camera_status::render_status_banner`: pending star (22 pt, `PINK`) instead of the spinner for Info states; on a state change the new title and detail fade in (`T_SHORT`) and the title colour cross-fades from the previous severity's colour; the previous text is not kept (security truth first) |
| G8 | `widgets::star_tile_turned(ui, rect, turned)`: `STAR_TURN_DEG × spring_value(turned)`; the app passes `turned` = confirmed `DaemonState::Inactive`; `star_tile` = not turned |
| G9 | `overlay::paint_detections` (moved verbatim from `app.rs`): colours from `frame.pad_live` of the frame being painted, positions from its detections; no motion helper is called |
| G10 | `Style::animation_time` = `T_SHORT`, 0 when reduced (`install_prefs`) |

`brand::paint_star_rotated(ui, rect, color, angle)`: angle 0 is exactly `paint_star`; otherwise the cached texture
is painted as a mesh rotated about the rect centre (the mark stays inside its inscribed circle).

## 7. CLI and wiring

`GuiArgs::reduce_motion` (`--reduce-motion`); `main.rs` calls `app.configure_motion(args.reduce_motion,
std::env::var_os(REDUCE_MOTION_ENV).as_deref())`; `SoosApp::new` and `capture_app` start with
`MotionControl::fixed(MotionPrefs::REDUCED)` and install it; `SoosApp::ui` calls `self.motion.apply(&ctx)` first.

## 8. Dependencies

`soos-gui` `[dependencies]`: `tokio = { workspace = true }` and `zbus = { workspace = true }` (both already in
`Cargo.lock`, no new crate enters it; only the `soos-gui` dependency list of the lockfile changes).

## 9. Traceability

Matrix component `gui-motion` with rows GMO1–GMO14 (unused prefix), UIM9 → Verified, UIM11 stays an owner row;
walkthrough `AI/walkthroughs/234_gui_motion.md`; `Docs/GUI_APPLICATION.md` §5.3/§5.5, `Docs/UI_MOTION.md` §7 (G4 and
G7 wording aligned with §6 of this spec), `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` "Audited D-Bus client". No new
ADR: the motion ADR item (6) already records the zbus widening and the PAU17 migration.

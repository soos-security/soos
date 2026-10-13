# Architect Spec — GitHub #377 (PR 1 of #358): motion ADR, design direction and motion tokens

- **Date**: 2026-10-13
- **Branch**: `chore/ui-motion-spec` (GitHub-only issue, not registered in `scripts/sync_issue.py`;
  commit carries `Closes #377` and `Refs #358`)
- **Base**: `origin/main` `6f32ef8`
- **Owner decisions**: #358 comments of 2026-10-12 and 2026-10-13 (keep the brand, modernise toward
  Notion/Linear-class polish with fluid, organic motion; ADR amendment approved, still no gradients and
  no decorative shadows; spring overshoot at most 4 %; a visual prototype is approved before the
  implementation PRs merge; GUI reduced motion from the XDG portal plus `--reduce-motion` /
  `SOOS_REDUCE_MOTION`; GUI dark variant out of scope; four-PR split accepted).

This is a **documentation and specification** change. Phases 1–4 are condensed per
`.claude/skills/dev-workflow/SKILL.md` §1 ("docs-only change"): the "implementation" of this PR is the
set of documents and the static prototype; no production code, no `Cargo.toml`, no asset of
`crates/remote/assets` and no `crates/gui/src` file changes.

## 1. Scope & Blast Radius

| File | Change |
|---|---|
| `AI/DECISIONS.md` | New ADR appended at the end of "Active Decisions": "[2026-10-13] Motion System for soos-gui and soos-remote" (amends, without superseding, item (3) "Presentation only" of the 2026-10-06 brand ADR and complements the 2026-10-13 browser support ADR). |
| `Docs/UI_MOTION.md` (new) | Normative motion specification: principles, tokens, spring, choreography, signature move, per-component specs (web and GUI), reduced motion, egui repaint/CPU budgets, browser fallbacks, out of scope, owner approval procedure. |
| `Docs/README.md` | Index row for `UI_MOTION.md` (required by `maintainer_hygiene_contract::test_docs_index_lists_every_docs_page`). |
| `Docs/REMOTE_COMPANION.md` §2e | Motion paragraph (tokens, reduced motion, View Transition fallback, the brand rules kept); every needle of `test_rmc_s55_brand_is_documented` kept. |
| `Docs/GUI_APPLICATION.md` | One pointer paragraph to `Docs/UI_MOTION.md` (GUI motion, reduced-motion sources). |
| `Docs/prototypes/ui_motion_prototype.html` (new) | Static, self-contained motion prototype for the owner's approval (no external resource). |
| `AI/tester_contract_motion.md` (new) | Tester contract: this PR's invariants plus the contract of the later `feat/remote-motion` and `feat/gui-motion` PRs. |
| `tests/invariants/src/ui_motion_spec_contract.rs` (new) + `tests/invariants/src/lib.rs` | Static invariants UIM-S1 … UIM-S6 pinning the spec documents, the token values, the spring mathematics and the prototype's self-containment. Test-only code. |
| `AI/plan_evaluator_report.md`, `AI/auditor_constraints_ui_motion.md` | Phase 1.5 and Phase 3 records. |
| `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/203_ui_motion_spec.md` | Traceability (rows UIM1 … UIM12, walkthrough 203). |

Consumers: none at runtime. The later PRs (`feat/remote-motion`, `feat/gui-motion`,
`test/ui-motion-owner-check`) consume `Docs/UI_MOTION.md` and `AI/tester_contract_motion.md`.

## 2. Facts Verified Against the Current Code

| Fact | Location | Value |
|---|---|---|
| Only web motion token | `crates/remote/assets/style.css` `:root` | `--t-fast: 120ms` |
| Button transition | `style.css` `button` | `transition: background-color var(--t-fast), box-shadow var(--t-fast), transform var(--t-fast)`; press `transform: scale(0.98)` |
| Reduced motion | `style.css` | exactly one `@media (prefers-reduced-motion: reduce)` block (`transition: none !important`, press transform removed); S48 pins "exactly one" |
| `:has()` rules | `style.css`, `remote_cross_browser_contract::HAS_FALLBACKS` | exactly five `:has()` rules, each repeated in `@supports not selector(:has(*))`; RXB-S7 fails on a sixth `:has()` rule without a pinned fallback, so **motion adds no `:has()` selector** and keys on the `data-state` hook of `.stat-tile` instead |
| State hooks set by `app.js` | `app.js` `render()` | `stateNode.className = "state state-<state>"`, `stateNode.dataset.state`, `statTile.dataset.state` (known states only), synchronous, before the button `disabled` writes |
| Unlock confirmation | `app.js` | native `window.confirm("Unlock the PC now?")`, then a passkey assertion; no custom dialog exists |
| Battery | `app.js` | a text line (`p.detail.battery`), no level bar |
| Camera full screen | `app.js` `enterCameraFullscreen`/`exitCameraFullscreen` | class `camera-full` on the stage, `camera-lock` on `<html>`, element fullscreen when the browser has it |
| CSS parser of the brand contract | `remote_brand_contract::parse_css` | one level of at-rule nesting, no CSS nesting |
| `app.js` style ban | `remote_camera_contract.rs` | no `.style.`, `cssText`, `setAttribute("style")`, `"style"` |
| GUI motion today | `crates/gui/src/widgets.rs` | progress `animate_value_with_time(…, 0.25)`, switch `animate_bool_with_time(…, 0.15)` |
| GUI spinners | `camera_status.rs` l.154, `header.rs` l.374 | `egui::Spinner` (Info banner, daemon switch busy) |
| GUI shadows | `theme.rs` `visuals()` | egui window and popup shadows (`SHADOW`) exist for floating windows; motion adds none |
| egui | `Cargo.lock` | 0.35.0 (`animate_bool_with_time_and_easing`, `animate_value_with_time`, `request_repaint_after`, `Style::animation_time`) |
| zbus | root `Cargo.toml` | `zbus = { version = "5", default-features = false, features = ["tokio"] }` (5.19.0 in the lockfile); `soos-gui` has no zbus or Tokio dependency today |
| GUI toasts | `crates/gui/src` | no toast or auto-dismiss component exists |

## 3. Design Decisions (recorded in the ADR and in `Docs/UI_MOTION.md`)

- **D-1 Brand kept, motion allowed.** Motion is allowed under the 2026-10-06 brand: flat colour blocks,
  2 px borders, 20 px cards, system fonts. Still forbidden: gradients (except the existing `html` band
  split), glassmorphism, blur, glow, decorative or new shadows, neon accents, emoji icons, confetti,
  parallax, particles, web fonts, animation libraries, and bounce on everything.
- **D-2 Compositor-friendly properties.** Animate only `transform` (`translate`, `scale`, `rotate`) and
  `opacity`, plus colour cross-fades between existing role tokens. Never `transition: all`, never layout
  properties (`width`, `height`, `top`, `left`, `margin`, `padding`). `box-shadow` stays transitionable
  only because the existing secondary buttons draw their 2 px outline with `inset 0 0 0` shadows; no new
  `box-shadow` declaration is allowed.
- **D-3 Security truth first.** Text, `aria-live` regions, `.state-*` classes, `data-state` and
  `disabled` are written synchronously, before or independently of any animation; animation follows the
  confirmed state and never leads it. No optimistic "Unlocked". Exits of whole sections are instant
  (a hidden section never fades out showing stale content).
- **D-4 Tokens.** One set of names, kebab case in CSS and `SCREAMING_SNAKE` in `crates/gui/src/theme.rs`
  (durations as `f32` seconds, CSS in `ms`, value(ms) = value(s) × 1000):

  | CSS | Rust | Value | Use |
  |---|---|---|---|
  | `--t-fast` | `T_FAST` | `120ms` | hover, press-in, focus border, control colours (unchanged) |
  | `--t-short` | `T_SHORT` | `180ms` | switch track colour, small fades, banners in, button swaps |
  | `--t-medium` | `T_MEDIUM` | `260ms` | section/card enter, tab indicator, state word, list items |
  | `--t-long` | `T_LONG` | `380ms` | camera full-screen View Transition, status tile colour morph |
  | `--t-spring` | `T_SPRING` | `400ms` | duration paired with `--ease-spring` (the spring's 0.1 % settle time) |
  | `--t-short-exit` | `T_SHORT_EXIT` | `126ms` | 0.7 × `T_SHORT` |
  | `--t-medium-exit` | `T_MEDIUM_EXIT` | `182ms` | 0.7 × `T_MEDIUM` |
  | `--t-long-exit` | `T_LONG_EXIT` | `266ms` | 0.7 × `T_LONG` (camera full screen closing) |
  | `--t-stagger` | `T_STAGGER` | `30ms` | per list item, at most `STAGGER_MAX_ITEMS` = 6 items (≤ 150 ms of delay) |
  | `--t-pending` | `T_PENDING` | `1200ms` | one quarter turn of the pending star |
  | `--ease-standard` | `EASE_STANDARD` | `cubic-bezier(0.2, 0, 0, 1)` | default, colour cross-fades, tab indicator |
  | `--ease-enter` | `EASE_ENTER` | `cubic-bezier(0.16, 1, 0.3, 1)` | elements appearing |
  | `--ease-exit` | `EASE_EXIT` | `cubic-bezier(0.4, 0, 1, 1)` | elements leaving |
  | `--ease-spring` | `SPRING_*` | `linear(…)` below; fallback `cubic-bezier(0.37, 0.73, 0.1, 1.15)` | press release, switch thumb, star turn |
  | `--press-scale` | `PRESS_SCALE` | `0.98` | pressed buttons (existing value kept; #358 suggested 0.97) |
  | `--rise` | `RISE_PX` | `6px` | cards and sections entering |
  | `--slide` | `SLIDE_PX` | `8px` | banners entering |
  | `--star-turn` | `STAR_TURN_DEG` | `45deg` | signature star turn |

  Rust easing constants are `[f32; 4]` cubic-bezier control points (`[0.2, 0.0, 0.0, 1.0]`, …).
- **D-5 Spring (owner: overshoot ≤ 4 %).** `SPRING_STIFFNESS = 400.0`, `SPRING_DAMPING = 30.0`,
  `SPRING_MASS = 1.0` (N/m, N·s/m, kg on a normalized unit span): ωn = √(k/m) = 20 rad/s,
  ζ = c / (2√(km)) = 0.75, ωd = ωn√(1−ζ²) ≈ 13.229 rad/s; overshoot = exp(−ζπ/√(1−ζ²)) ≈ **2.84 %**
  at t ≈ 237 ms; the step response stays within 0.1 % of the target from **400 ms** on (`T_SPRING`).
  The margin to the 4 % bound absorbs sampling and rounding. Step response for a unit step from rest:
  x(t) = 1 − e^(−ζωn·t)·(cos ωd·t + (ζωn/ωd)·sin ωd·t).
- **D-6 CSS spring.** `--ease-spring` is sampled from x(t) over `T_SPRING` and simplified to 22 stops
  (maximum absolute error ≤ 0.005 against x(t), peak 1.0271):
  `linear(0, 0.0063 1.5%, 0.0263 3%, 0.0595 4.8%, 0.1065 6.6%, 0.1522 8.2%, 0.2104 10%, 0.4504 16.9%, 0.5562 20.1%, 0.6552 23.4%, 0.7391 26.6%, 0.8083 29.6%, 0.8658 32.8%, 0.9136 35.9%, 0.9524 39.3%, 0.9829 42.9%, 1.0052 46.8%, 1.0197 51%, 1.0271 55.9%, 1.0269 63.8%, 1.0085 84.4%, 1)`.
  Fallback `cubic-bezier(0.37, 0.73, 0.1, 1.15)` (least-squares fit, peak ≈ 1.030, maximum error ≈ 0.04).
  `style.css` declares the fallback once in a top-level `:root` and the `linear()` value once in
  `@supports (transition-timing-function: linear(0, 1)) { :root { … } }` (one at-rule level, parseable
  by the brand contract). This amends the #358 line "each declared exactly once in a top-level `:root`"
  for `--ease-spring` only. A spring curve is used on `transform` only, never on `opacity` or colour.
- **D-7 GUI spring.** `motion.rs` advances a `(value, velocity)` state toward its target with the
  **closed-form** damped-oscillator solution over `dt` (exact, frame-rate independent; no Euler drift).
  `dt` comes from `stable_dt`, is clamped to `[0, MAX_MOTION_DT = 0.05 s]`, and `NaN`/negative is 0,
  `+inf` is `MAX_MOTION_DT`. A non-finite target is ignored; a non-finite state snaps to the target.
  `settled()` holds when |target − value| ≤ `SPRING_SETTLE_EPSILON` (0.001) × span and |velocity| ≤
  `SPRING_SETTLE_VELOCITY` (0.1) × span per second, span = |target − value| at the last retarget (a
  zero span is settled at once); a settled spring snaps exactly to its target and requests no repaint.
  Retargeting mid-flight keeps the velocity (organic reversal of a switch); `retarget` with the target
  it already has is a no-op (span and settled state kept), so a caller that retargets every frame still
  settles and stops repainting.
- **D-8 Signature move.** The pink star (web `.stat-star`, GUI `brand::paint_star`) is at 0° in every
  state except a **confirmed** state that lowers protection (web: `unlocked`; GUI: daemon `Inactive`),
  where it is turned `--star-turn` (45°); it moves between the two with the spring. The state angle uses
  the CSS `rotate` property and the pending loop below animates `transform`, so the two compose and
  never fight over one property; when the pending loop ends, its `transform` returns to none on the same
  frame as the confirmed state change (the spring turn of the state angle carries the eye). Web hook:
  `.stat-tile[data-state="unlocked"] .stat-star` (no `:has()`). While an action waits for its answer, a
  pending star turns a quarter turn per `T_PENDING` (`ease-standard`, the four-point mark is
  quarter-turn symmetric enough to read as a calm tick, not a spinner). It replaces `egui::Spinner` in
  `camera_status.rs` and `header.rs`. The pending loop is scoped to `.is-pending` (web) or a pending
  flag (GUI), so it ends with the request. Web: `is-pending` is set when `lockRequestedAt` /
  `unlockRequestedAt` is set (for unlock, after the passkey assertion, never during the native confirm
  or the passkey sheet) and cleared with it, so it lasts at most `LOCK_CONFIRM_UI_MS` /
  `UNLOCK_CONFIRM_UI_MS` = 5000 ms (`app.js`), within WCAG 2.2.2's 5 s limit. GUI: the existing
  privileged-action pending and camera "opening" states. It never runs while idle, and it is removed
  entirely under reduced motion. The star is decorative (`aria-hidden`): the words remain the
  only statement of the state.
- **D-9 Web class hooks (later `feat/remote-motion`).** `app.js` may only: toggle `is-pending` on
  `#lock` / `#unlock` and on `.stat-tile` while a lock/unlock request is in flight (cleared on every exit
  path: confirmation, failure, deadline); toggle `stagger-once` on a list for its first render after its
  section becomes visible (removed on the next render, so refresh polls never replay it); and wrap the
  camera full-screen class change of the **CSS overlay path** (no element fullscreen, e.g. iPhone) in a
  feature-detected `document.startViewTransition`, skipped when `matchMedia("(prefers-reduced-motion:
  reduce)")` matches, with any throw falling back to the direct class change. The `cameraFull` flag is
  written synchronously before the call; the update callback applies `camera-full` / `camera-lock` from
  the **current** flag (never a captured argument), so a quick exit and re-entry cannot leave the
  overlay in a stale state; the `ready`, `updateCallbackDone` and `finished` promises get a no-op
  `catch` (a skipped transition rejects them); an exit caused by the view ending (`endCameraView`, error,
  stall) applies directly without a transition. No `.animate(`, no WAAPI,
  no `requestAnimationFrame` style writes, no `.style.`, no animation library, no new asset, no CSP
  change.
- **D-10 Unlock confirmation stays native.** `window.confirm` cannot be animated and is kept: the #358
  dialog item is not applicable. No custom confirmation dialog is introduced by motion work.
- **D-11 Progressive enhancement (browser support ADR 2026-10-13).** Every motion feature degrades to
  today's instant change: `linear()` → cubic-bezier fallback through `@supports`; `@starting-style`
  (not yet Baseline widely available) → sections appear instantly; View Transitions → instant overlay;
  `transition-behavior: allow-discrete` is not used (exits are instant by D-3). No polyfill.
- **D-12 Reduced motion, web.** The single `@media (prefers-reduced-motion: reduce)` block (S48 still
  finds exactly one) neutralises every `animation` (`animation: none !important`), removes transform
  transitions and stagger delays, and keeps only opacity and colour cross-fades of at most `--t-fast`;
  the only literal duration allowed outside `:root` is `0s` inside that block. The star sits at its
  final angle at once; `startViewTransition` is skipped. Under reduced motion `@starting-style` entries
  keep only an opacity fade (`--t-fast`).
- **D-13 Reduced motion, GUI (owner decision 4).** `MotionPrefs { reduced: bool }` resolved in this order:
  (1) `--reduce-motion` (CLI switch) → reduced; (2) `SOOS_REDUCE_MOTION` = `1` → reduced, `0` → full
  motion, any other value ignored; (3) XDG desktop portal `org.freedesktop.portal.Settings.ReadOne`
  `org.freedesktop.appearance` / `reduced-motion` (`u32`: 1 = reduce, 0 = no preference), then
  `org.gnome.desktop.interface` / `enable-animations` (`false` → reduced); (4) default full motion.
  A disconnected channel (the reader thread ended or panicked) or 200 ms elapsed without an answer is
  `Unavailable` (full motion).
  The portal is read once at start-up on a background thread through zbus with one overall
  `PORTAL_READ_DEADLINE_MS` = 200 ms deadline; no session bus, no portal, an error, an unexpected type
  or the deadline means "no preference" (full motion), with no retry. Until the answer arrives (at most
  200 ms) the GUI treats motion as reduced, so a user who asked for reduced motion never sees a first
  animation. The UI thread only polls a channel (`try_recv`); it never blocks. **Dependencies**: the
  workspace zbus has no blocking API and is built with its `tokio` feature, so `feat/gui-motion` adds
  `zbus` and `tokio` (current-thread runtime, on the reader thread only) as direct dependencies of
  `soos-gui`; both are already in `Cargo.lock`, so no new crate enters the lockfile (#358 had said "no
  other new dependency": recorded here and in the ADR). **zbus scope (auditor C-1)** (amends ADR 2026-10-05 item 7 and the "Audited D-Bus client, two crates only" rule of `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`): `soos-gui` may use `zbus = { workspace = true }` for one read-only session-bus exception only: one connection, on the portal reader thread only, calling `org.freedesktop.portal.Settings.ReadOne` (fallback `Read`) on `org.freedesktop.portal.Desktop` for the two documented keys, with no object server, no well-known name and no signal match. The session bus address comes from the environment (unlike the pinned system bus of `soos-daemon` and `soos-remote`); the worst a hostile session bus can do is answer a cosmetic preference, which is validated (one `u32` or `bool`) and never logged. The later `feat/gui-motion` PR applies a planned Contract Migration of `presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon` (PAU17): its `ALLOWED` list gains `crates/gui/Cargo.toml` with exactly `zbus = { workspace = true }` named once; every other assertion and the test name stay unchanged; the guideline line is updated and still names `soos-remote`. No process is spawned instead. A live
  `SettingChanged` subscription is a follow-up (changes apply at the next launch). Under reduced
  motion every helper returns its target on the first frame and `Style::animation_time` is 0.
- **D-14 GUI budgets.** No repaint from motion code once every animation has settled (idle = zero
  motion repaints); interactive transitions request a repaint per frame only while unsettled
  (display-rate, vsync-capped); the ambient pending star requests `request_repaint_after(33 ms)` (≤ 30 Hz);
  motion state is `Copy` data in egui temp storage (no per-frame allocation in `motion.rs`), O(1) per
  animated widget; no process, no blocking I/O on the UI thread; no wall-clock CPU assertion in tests
  (determinism rule) — the frame-time check is an owner row.
- **D-15 GUI scope.** Tab indicator, daemon switch, banners, cards and stat tiles, progress bar, camera
  status cross-fade, pending star. The face overlay is **not** interpolated: boxes and verdict colours
  are drawn from the latest detection on the same frame (truth over smoothness; #358 allowed ≤ 1 frame,
  zero is within it). No toast component is introduced (none exists; a new one would be a feature):
  existing feedback banners get banner motion and nothing auto-dismisses. The GUI dark variant is out of
  scope (owner decision 5, separate issue).
- **D-16 Prototype.** `Docs/prototypes/ui_motion_prototype.html`: one static file, inline CSS and JS, no
  network request, no `<link>`, no `src=`, no `url(`, no `@import`, no `data:` URI, no image, no font
  file; colours only from `crates/gui/src/theme.rs`; no gradient; `box-shadow` only `inset 0 0 0 …`;
  every token declared with its exact value; a reduced-motion toggle (initialised from
  `prefers-reduced-motion`) and a slow-motion review toggle; the camera mock is a flat placeholder (no
  face, no photo); the star and wordmark paths are the ones already published in
  `crates/remote/assets/index.html` (no file from the owner's private brand pack). Size ≤ 96 KiB. The
  owner approves it (matrix row UIM7) before `feat/remote-motion` and `feat/gui-motion` merge.

## 4. Types & Signatures (normative for the later `feat/gui-motion` PR; not added here)

```rust
// crates/gui/src/theme.rs (tokens; durations in seconds)
pub const T_FAST: f32 = 0.12;   pub const T_SHORT: f32 = 0.18;  pub const T_MEDIUM: f32 = 0.26;
pub const T_LONG: f32 = 0.38;   pub const T_SPRING: f32 = 0.40;
pub const T_SHORT_EXIT: f32 = 0.126; pub const T_MEDIUM_EXIT: f32 = 0.182; pub const T_LONG_EXIT: f32 = 0.266;
pub const T_STAGGER: f32 = 0.03; pub const STAGGER_MAX_ITEMS: usize = 6; pub const T_PENDING: f32 = 1.2;
pub const EASE_STANDARD: [f32; 4] = [0.2, 0.0, 0.0, 1.0];
pub const EASE_ENTER: [f32; 4] = [0.16, 1.0, 0.3, 1.0];
pub const EASE_EXIT: [f32; 4] = [0.4, 0.0, 1.0, 1.0];
pub const SPRING_STIFFNESS: f32 = 400.0; pub const SPRING_DAMPING: f32 = 30.0; pub const SPRING_MASS: f32 = 1.0;
pub const PRESS_SCALE: f32 = 0.98; pub const RISE_PX: f32 = 6.0; pub const SLIDE_PX: f32 = 8.0;
pub const STAR_TURN_DEG: f32 = 45.0;

// crates/gui/src/motion.rs (pure; #![forbid(unsafe_code)] is crate-wide)
pub const MAX_MOTION_DT: f32 = 0.05;
pub const SPRING_SETTLE_EPSILON: f32 = 0.001;
pub const SPRING_SETTLE_VELOCITY: f32 = 0.1;
pub const PORTAL_READ_DEADLINE_MS: u64 = 200;
pub const AMBIENT_REPAINT_MS: u64 = 33;
#[derive(Clone, Copy, Debug, PartialEq)] pub struct SpringState { /* value, velocity, target, span */ }
impl SpringState {
    pub fn at(value: f32) -> Self;                 // settled at `value`
    pub fn retarget(&mut self, target: f32);       // non-finite target ignored
    pub fn step(&mut self, dt: f32) -> f32;        // closed form, dt clamped, never non-finite
    pub fn value(&self) -> f32;
    pub fn settled(&self) -> bool;
}
pub fn cubic_bezier(points: [f32; 4], t: f32) -> f32; // t clamped to [0, 1]; f(0)=0, f(1)=1
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct MotionPrefs { pub reduced: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum PortalAnswer { Pending, Reduce, NoPreference, Unavailable }
pub fn resolve_motion_prefs(cli_reduce: bool, env: Option<&str>, portal: PortalAnswer) -> MotionPrefs;
```

Bounds: every `f32` input may be non-finite (handled as in D-7); `env` longer than 8 bytes is ignored;
the portal reply is one `u32` or one `bool` (anything else is `Unavailable`).

## 5. Error Taxonomy

No PAM, daemon, IPC or authorization path is touched. GUI motion failures are cosmetic and fail to
"no animation": portal errors → `Unavailable` → full motion after the deadline; a non-finite spring
input → snap to target; a View Transition throw → direct class change. None of them can change a
security decision, a confirmation or a displayed state.

## 6. Latency Budget

Not applicable (no auth path). UI rule: a state change is visible (text) on the same frame; motion is
at most `T_SPRING` = 400 ms behind it and never delays input handling.

## 7. Invariants Touched

- Brand ADR 2026-10-06 (3) "Presentation only": amended (class hooks + feature-detected View
  Transition). Kept: CSP, routes, the eleven-file asset set, element ids/labels/live regions, palette,
  no gradients, no decorative shadows, system fonts.
- Browser support ADR 2026-10-13: feature detection only, no User-Agent logic, no polyfill.
- Existing tests that must stay green **unmodified** in this PR and the later ones:
  `remote_brand_contract` (S44–S56), `remote_camera_contract`, `remote_cross_browser_contract`
  (notably RXB-S7's five pinned `:has()` rules), `crates/gui/tests/responsiveness_tests.rs`,
  `theme_tests.rs`, `brand_layout_tests.rs`.
- Biometric privacy: no frame is captured, blurred, cached or logged for an effect; a View Transition
  snapshot is the browser's own rendering of the already visible page, unreadable by script, discarded
  after the transition.

## 8. Test Hooks for Tester

This PR (static invariants, `tests/invariants/src/ui_motion_spec_contract.rs`):

- UIM-S1 ADR present (title, amendment of the brand ADR, unchanged CSP/routes/assets, no gradients,
  no decorative shadows, reduced motion mandatory, progressive enhancement); `Docs/UI_MOTION.md`,
  `AI/tester_contract_motion.md` and the prototype exist; `Docs/README.md` links `UI_MOTION.md`.
- UIM-S2 every pinned token appears in one row of the `Docs/UI_MOTION.md` token table with its CSS
  name, Rust name and CSS value; the exit tokens equal 0.7 × entry (rounded to the millisecond);
  forward-compatible live cross-check: a token already declared in `style.css` or `theme.rs` must have
  the pinned value (today only `--t-fast`).
- UIM-S3 spring mathematics from the pinned parameters: 0 < ζ < 1, overshoot ≤ 4 %, the 0.1 % settle
  time ≤ `T_SPRING` ≤ 450 ms; a closed-form stepping at 30, 60 and 144 Hz settles (D-7 criterion) within
  450 ms with overshoot ≤ 4 %; the pinned `linear()` string parses, starts at 0, ends at 1, has strictly
  increasing positions, peaks ≤ 1.04 and stays within 0.005 of x(t); the cubic-bezier fallback peaks
  ≤ 1.04, stays within 0.05 of x(t), and the three standard easings are monotonic with f(0)=0, f(1)=1;
  `Docs/UI_MOTION.md` carries the exact parameters, the exact `linear()` and fallback strings.
- UIM-S4 prototype self-contained and on brand (D-16 list), declares every token with its value, has the
  reduced-motion toggle and a `prefers-reduced-motion` media query, no `transition: all`.
- UIM-S5 reduced-motion sources and budgets documented (`prefers-reduced-motion`,
  `org.freedesktop.appearance`, `reduced-motion`, `enable-animations`, `--reduce-motion`,
  `SOOS_REDUCE_MOTION`, `200 ms`, repaint budget, dark variant out of scope).
- UIM-S6 `Docs/REMOTE_COMPANION.md` §2e points to `Docs/UI_MOTION.md` and the motion ADR and states the
  reduced-motion and fallback behaviour; `Docs/GUI_APPLICATION.md` points to `Docs/UI_MOTION.md`.

Later PRs: the tester contract records the assertions of `remote_motion_contract.rs` and
`crates/gui/tests/motion_tests.rs` (see `AI/tester_contract_motion.md`).

## 9. Plan Evaluation Follow-ups

Plan evaluator round 1 `VALIDATION_VERDICT: APPROVED` with eleven minor findings, all applied: m1/m2/m3
(stronger UIM-S4, UIM-S2 exit check against the document, UIM-S1 needles for routes and assets), m4
(D-9 View Transition state from the current flag, rejections caught, no transition on view end), m5
(zbus/tokio direct dependencies recorded), m6 (dead portal thread = `Unavailable`), m7 (the prototype's
English is reviewed by hand: `candid_review.sh` does not scan `.html`), m8 (`cargo fmt`), m9 (no-op
same-target retarget), m10 (D-8 wording and distinct properties), m11 (rows UIM7–UIM12 pending).

## 10. Documentation Drift / ADR

- #358 proposed `Spring { stiffness: 380, damping: 32 }` (ζ ≈ 0.82, overshoot ≈ 1.1 %, settle ≈ 443 ms
  at 0.1 %); replaced by 400/30 (ζ = 0.75, overshoot ≈ 2.84 %, settle 400 ms) to give the visible but
  light overshoot the owner chose while staying under 4 %.
- #358 press scale 0.97 vs the existing 0.98: 0.98 kept (no visual change of the brand redesign).
- #358 web items "unlock confirmation dialog" and "battery bar" do not exist in the code (native
  `confirm`, text battery line): not applicable; GUI "toasts" do not exist: not introduced.
- #358 "face overlay may interpolate ≤ 80 ms": decided zero interpolation.
- #358 "each token declared exactly once in a top-level `:root`": `--ease-spring` is declared twice
  (fallback + `@supports` `linear()`), see D-6.
- #358 walkthrough number 195 is taken; this PR uses 203.

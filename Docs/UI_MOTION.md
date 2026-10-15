# UI Motion — `soos-gui` and `soos-remote`

Normative motion specification for the desktop GUI (`crates/gui`, egui 0.35) and the phone web app
(`crates/remote/assets`). ADR "[2026-10-13] Motion System for soos-gui and soos-remote" in
`AI/DECISIONS.md`; GitHub #377 (PR 1 of #358); architect spec `AI/architect_spec_ui_motion.md`; tester
contract `AI/tester_contract_motion.md`; static invariants `tests/invariants/src/ui_motion_spec_contract.rs`
(matrix UIM1–UIM6).

This document is a specification. The motion itself lands in two later pull requests:
`feat/remote-motion` (`style.css`, class hooks in `app.js`) and `feat/gui-motion` (`theme.rs` tokens,
`motion.rs`, animated widgets). A third, `test/ui-motion-owner-check`, records the owner's device checks.

## 1. Direction

The owner's direction (2026-10-12): keep the soos brand (blue band, cards, pink star, big numerals)
and bring it to Notion/Linear-class polish with fluid, organic motion, without "AI slop".

Principles:

1. **Motion explains state; it never decorates.** Every animation answers "what changed, and where did
   it come from?". Nothing moves while the user is idle.
2. **Stay within the brand** (ADR 2026-10-06): flat colour blocks, 2 px borders, 20 px cards, system
   fonts. **Never**: gradients (except the existing `html` band split), glassmorphism, blur, glow,
   decorative or new shadows, neon accents, emoji icons, confetti, parallax, particles, web fonts,
   animation libraries, sound, haptics, bounce on everything.
3. **Compositor-friendly.** Animate `transform` (`translate`, `scale`, `rotate`) and `opacity`, plus
   colour cross-fades between existing role tokens. Never `transition: all`, never `width`, `height`,
   `top`, `left`, `margin`, `padding`. `box-shadow` is transitioned only where it already draws the
   2 px inset outline of secondary buttons; no new `box-shadow` is added.
4. **Fast in, faster out.** Exits run at 0.7 × the matching entry duration. Whole sections that are
   hidden disappear **instantly** (a hidden section never fades out while still showing stale content).
5. **One signature move**: the pink star (section 5).
6. **Security truth first.** Text, `aria-live` regions, `.state-*` classes, `data-state` and `disabled`
   are written synchronously, before or independently of any animation. Animation follows the
   confirmed state and never leads it; there is no optimistic "Unlocked". Animations never shorten,
   auto-confirm or hide a confirmation, and focus rings appear and disappear instantly.

## 2. Tokens

One set of names: kebab case in CSS (`style.css` `:root`), `SCREAMING_SNAKE` in
`crates/gui/src/theme.rs`, the same values (CSS durations in `ms`, Rust durations as `f32` seconds,
Rust easings as `[f32; 4]` cubic-bezier control points). `theme.rs` stays the single source of the
values once the GUI pull request adds them; the web mirrors them, as it already does for colours.

| CSS | Rust | Value | Use |
|---|---|---|---|
| `--t-fast` | `T_FAST` | `120ms` | hover, press-in, input border focus, control colours (unchanged) |
| `--t-short` | `T_SHORT` | `180ms` | switch track colour, small fades, banners in, button swaps |
| `--t-medium` | `T_MEDIUM` | `260ms` | section and card enter, tab indicator, state word, list items |
| `--t-long` | `T_LONG` | `380ms` | camera full-screen View Transition, status tile colour morph |
| `--t-spring` | `T_SPRING` | `400ms` | the duration paired with `--ease-spring` (spring settle time) |
| `--t-short-exit` | `T_SHORT_EXIT` | `126ms` | 0.7 × `--t-short` |
| `--t-medium-exit` | `T_MEDIUM_EXIT` | `182ms` | 0.7 × `--t-medium` |
| `--t-long-exit` | `T_LONG_EXIT` | `266ms` | 0.7 × `--t-long` (camera full screen closing) |
| `--t-stagger` | `T_STAGGER` | `30ms` | delay per list item; at most `STAGGER_MAX_ITEMS` = 6 items (≤ 150 ms of delay) |
| `--t-pending` | `T_PENDING` | `1200ms` | one quarter turn of the pending star |
| `--ease-standard` | `EASE_STANDARD` | `cubic-bezier(0.2, 0, 0, 1)` | default; colour cross-fades; tab indicator |
| `--ease-enter` | `EASE_ENTER` | `cubic-bezier(0.16, 1, 0.3, 1)` | elements appearing |
| `--ease-exit` | `EASE_EXIT` | `cubic-bezier(0.4, 0, 1, 1)` | elements leaving |
| `--press-scale` | `PRESS_SCALE` | `0.98` | pressed buttons (the existing value; #358 suggested 0.97) |
| `--rise` | `RISE_PX` | `6px` | cards and sections entering (translate from below) |
| `--slide` | `SLIDE_PX` | `8px` | banners entering |
| `--star-turn` | `STAR_TURN_DEG` | `45deg` | the signature star turn |

`--ease-spring` (section 3) is a curve, not a duration: it is always paired with `--t-spring`.

## 3. Spring

Owner decision (2026-10-13): light overshoot, **at most 4 %**.

| Parameter | Value | Note |
|---|---|---|
| `SPRING_STIFFNESS` | `400.0` | k, on a normalized unit span |
| `SPRING_DAMPING` | `30.0` | c |
| `SPRING_MASS` | `1.0` | m |
| natural frequency | 20 rad/s | ωn = √(k/m) |
| damping ratio | 0.75 | ζ = c / (2√(km)) |
| overshoot | 2.84 % | exp(−ζπ / √(1−ζ²)), peak at about 237 ms |
| settle time | 400 ms | within 0.1 % of the target from then on = `T_SPRING` |

Step response from rest (unit step): `x(t) = 1 − e^(−ζωn·t) · (cos ωd·t + (ζωn/ωd) · sin ωd·t)`, with
ωd = ωn·√(1−ζ²) ≈ 13.229 rad/s. The 1.16-point margin to the 4 % bound absorbs sampling and rounding.

**CSS.** `--ease-spring` is `x(t)` sampled over `T_SPRING` and simplified to 22 stops (maximum error
below 0.005, peak 1.0271):

```css
linear(0, 0.0063 1.5%, 0.0263 3%, 0.0595 4.8%, 0.1065 6.6%, 0.1522 8.2%, 0.2104 10%, 0.4504 16.9%, 0.5562 20.1%, 0.6552 23.4%, 0.7391 26.6%, 0.8083 29.6%, 0.8658 32.8%, 0.9136 35.9%, 0.9524 39.3%, 0.9829 42.9%, 1.0052 46.8%, 1.0197 51%, 1.0271 55.9%, 1.0269 63.8%, 1.0085 84.4%, 1)
```

Fallback for engines without `linear()`: `cubic-bezier(0.37, 0.73, 0.1, 1.15)` (least-squares fit, peak
about 1.030, maximum error about 0.04). `style.css` declares the fallback once in a top-level `:root`
and the `linear()` value once inside `@supports (transition-timing-function: linear(0, 1))` in a
`:root` rule (one at-rule level, parseable by the brand contract). A custom property is not validated
where it is declared, so without the `@supports` gate an engine that does not know `linear()` would
drop the whole `transition` declaration; with it, that engine gets the fallback. A spring curve is used
on `transform` only, never on `opacity` or colour.

**GUI.** `motion.rs` advances `(value, velocity)` toward the target with the **closed-form** solution of
the same damped oscillator over each frame's `dt` (exact and frame-rate independent; no Euler drift):

- `dt` comes from egui's `stable_dt`, is clamped to `[0, MAX_MOTION_DT]` (0.05 s); `NaN` or negative is
  0, `+inf` is 0.05 s. A long stall therefore never makes a value jump.
- A non-finite target is ignored; a non-finite state snaps to the target. Output is always finite.
- `settled()` holds when |target − value| ≤ `SPRING_SETTLE_EPSILON` (0.001) × span and |velocity| ≤
  `SPRING_SETTLE_VELOCITY` (0.1) × span per second; span is |target − value| at the last retarget (a
  zero span is settled at once). A settled spring snaps exactly to its target.
- Retargeting mid-flight keeps the velocity, so a switch toggled twice reverses smoothly. Retargeting
  to the target it already has is a no-op, so a widget that retargets every frame still settles and
  stops repainting.
- At 30, 60 and 144 Hz the state settles within 450 ms with an overshoot of at most 4 % (checked by
  `test_uim_s3_spring_and_easing_mathematics` now and by `motion_tests.rs` later).

## 4. Choreography

- **Order**: state text first (same frame), then colour (`--t-short`/`--t-medium`, standard easing),
  then position or rotation (`--ease-enter` or the spring). Nothing waits for an animation to finish.
- **Distance**: small. Entries rise `--rise` (6 px) or slide `--slide` (8 px); nothing crosses the screen.
- **Stagger**: lists and card groups enter `--t-stagger` (30 ms) apart, at most 6 items; items beyond
  the sixth enter with the sixth. First render only, never on refresh polls.
- **Exits**: transient elements (banners, button swaps) leave at 0.7 × entry with `--ease-exit`; whole
  sections hide instantly.
- **Concurrency**: one animation per property per element. A new state interrupts the running
  transition from its current value (CSS transitions and the GUI spring both do this).
- **Loops**: only the pending star (section 5), and only while a request is in flight.

## 5. Signature move: the pink star

- **Angle**: the star (web `.stat-star`, GUI `brand::paint_star`) is at 0° in every state except a
  **confirmed** state that lowers protection (web: `unlocked`; GUI: daemon `Inactive`), where it is
  turned `--star-turn` (45°). It moves between the two on the spring (`--t-spring`, `--ease-spring`).
  The state angle uses the CSS `rotate` property and the pending loop animates `transform`, so the two
  compose instead of fighting over one property. Web hook: `.stat-tile[data-state="unlocked"] .stat-star` — the `data-state` attribute
  `app.js` already sets synchronously. No new `:has()` selector is added: RXB-S7 pins exactly five.
- **Pending**: while an action waits for its answer, the star turns a quarter turn per `--t-pending`
  (1.2 s, standard easing): a calm tick, not a spinner. It replaces `egui::Spinner` in
  `camera_status.rs` and `header.rs`. Web: the loop is scoped to `.is-pending`, set when `app.js` sets
  `lockRequestedAt` / `unlockRequestedAt` (for unlock, after the passkey assertion, never during
  `window.confirm` or the passkey sheet) and cleared with it, so it lasts at most `LOCK_CONFIRM_UI_MS` /
  `UNLOCK_CONFIRM_UI_MS` = 5000 ms, within WCAG 2.2.2's 5 s limit.
- **Meaning**: the star is decorative (`aria-hidden`). The words remain the only statement of the state.
- **Reduced motion**: the star sits at its final angle at once and the pending loop does not run.

## 6. Web components (`feat/remote-motion`)

All motion lives in `style.css`. `app.js` may only toggle classes and call a feature-detected
`document.startViewTransition`; no `.animate(`, no Web Animations API (its CSP status under
`style-src-attr` is unclear), no `requestAnimationFrame` style writes, no `.style.`, no `cssText`, no
animation library, no new asset, no CSP change, no new element id.

| # | Component | Motion | Hook |
|---|---|---|---|
| W1 | Status tile | state word re-enters (opacity 0.4 → 1 and 4 px rise, `--t-medium`, `--ease-enter`) when the `.state-*` class changes (one keyframe name per state class, so an identical class rewrite on a poll replays nothing); dot and tile colours cross-fade `--t-medium`; star turn (section 5) | existing `.state-*`, `data-state` |
| W2 | `Lock now` / `Unlock now` | press-in `scale(var(--press-scale))` in `--t-fast` standard; release on the spring; background and outline colours `--t-fast`; `.is-pending` shows a 16 px star after the label (CSS `clip-path: path()` of the star mark, `currentColor`), so the label never moves | new `is-pending` on `#lock` / `#unlock` and `.stat-tile` |
| W3 | Unlock confirmation | none: the confirmation is the native `window.confirm`, which cannot be animated and stays native; the #358 dialog item does not apply | — |
| W4 | Sections and cards revealed from `[hidden]` (login, alerts, notifications, enroll, camera) | `@starting-style` entry: opacity 0 and `translateY(var(--rise))` → rest, `--t-medium`, `--ease-enter`; hiding is instant | existing `[hidden]` |
| W5 | Banners | coverage banner enters with `--slide` and a fade (`--t-short`); `.alerts-attention` cross-fades the summary to the danger banner (`--t-medium`) and nudges it once (two cycles of ±3 px `translateX`, `--t-medium` each, never infinite) | existing `.alerts-attention`, `[hidden]` |
| W6 | Alert history and device lists | staggered fade-up of the first 6 items, first render only | new `stagger-once` on the list, set for the first render after its section becomes visible and removed on the next render |
| W7 | Notifications card ("notifications toggle") | the card keeps **no switch** (brand ADR); the Enable/Disable button swap fades in the newly shown button (`--t-short`); `#push-state` text changes at once | existing `[hidden]` |
| W8 | Camera full screen | CSS overlay path only (no element fullscreen, e.g. iPhone): the class change runs inside `document.startViewTransition`; `::view-transition-group(camera-stage)` grows over `--t-long` (`html.camera-lock`) and shrinks back over `--t-long-exit` (`html:not(.camera-lock)`), standard easing. Browsers with element fullscreen keep their own animation (no View Transition, so `requestFullscreen` stays in the tap's activation) | existing `camera-full`, `camera-lock` |
| W9 | Enrollment code input | border colour `--t-fast`; the focus ring itself appears instantly | — |

View Transition guard (W8), as one helper in `app.js`:

```js
// Presentation only. The caller writes `cameraFull` first; `apply` reads the CURRENT flag
// (never a captured argument), so a quick exit and re-entry cannot leave a stale overlay.
function withViewTransition(apply) {
  const reduce = typeof window.matchMedia === "function"
    && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (reduce || typeof document.startViewTransition !== "function") {
    apply();
    return;
  }
  try {
    const transition = document.startViewTransition(apply);
    // A skipped transition rejects these promises: never leave them unhandled.
    [transition.ready, transition.updateCallbackDone, transition.finished].forEach(function (p) {
      if (p && typeof p.catch === "function") {
        p.catch(function () {});
      }
    });
  } catch (_) {
    apply();
  }
}
```

An exit caused by the view ending (`endCameraView`, an error, a stall) applies the classes directly,
without a transition.

The camera snapshot a View Transition takes is the browser's own rendering of the already visible
page, unreadable by script and discarded after the transition: no frame is captured, blurred, cached,
stored or logged for an effect. The battery is a text line (there is no level bar to animate) and the
page has no loading skeleton: neither gets motion.

**Progressive enhancement** (ADR 2026-10-13 "Browser Support Policy for the `soos-remote` Web App":
feature detection only, no polyfill, no User-Agent logic). Every motion feature falls back to today's
instant change:

| Feature | Without it |
|---|---|
| `linear()` | `--ease-spring` keeps the `cubic-bezier` fallback (`@supports` gate) |
| `@starting-style` (not yet Baseline widely available) | sections and items appear instantly |
| View Transitions (`startViewTransition`) | the overlay switches instantly (`withViewTransition` calls `apply()`) |
| `clip-path: path()` | the pending star in the button is not drawn; the button stays disabled with its label |
| `transition-behavior: allow-discrete` | not used: exits are instant by design |

## 7. GUI components (`feat/gui-motion`)

| # | Component | Motion |
|---|---|---|
| G1 | `motion.rs` | pure helpers: `SpringState` (section 3), `cubic_bezier(points, t)` (t clamped; f(0) = 0, f(1) = 1), `MotionPrefs`, `resolve_motion_prefs`; state stored as `Copy` data in egui temp storage |
| G2 | Header daemon switch | thumb on the spring (retargetable), track colour cross-fade `T_SHORT` standard (replaces the fixed 0.15 s); busy: the pending star replaces the `egui::Spinner`; the authorization text stays visible |
| G3 | Tabs | a pill indicator slides between tabs (`T_MEDIUM`, `EASE_STANDARD`); labels change colour `T_FAST` |
| G4 | Cards and stat tiles | on a mode or tab change, cards fade in and rise `RISE_PX` (`T_MEDIUM`, `EASE_ENTER`), staggered `T_STAGGER`, at most 6; stat values cross-fade `T_SHORT`, never count up |
| G5 | Banners (`widgets::banner`) | enter with a fade and `SLIDE_PX` (`T_SHORT`, `EASE_ENTER`), leave in `T_SHORT_EXIT` (`EASE_EXIT`); a kind change cross-fades the colours; nothing auto-dismisses (there is no toast component and none is added) |
| G6 | Progress bar | the fill eases toward its target in `T_MEDIUM` with `EASE_STANDARD` (replaces the fixed 0.25 s) |
| G7 | Camera status card (`camera_status.rs`) | cross-fade between banner states (old out `T_SHORT_EXIT`, new in `T_SHORT`); the pending star replaces the spinner of Info states |
| G8 | Star tiles | turn 45° with the spring when the confirmed daemon state changes (section 5) |
| G9 | Face overlay | **no motion**: boxes, landmarks and the live/spoof colours (`FACE_LIVE`, `FACE_SPOOF`) are drawn from the latest detection on the same frame, never interpolated |
| G10 | egui windows and popups | the built-in fade with `Style::animation_time` = `T_SHORT`; their existing shadows are unchanged and no shadow is added |

## 8. Reduced motion

Reduced motion is mandatory on both surfaces (WCAG 2.2 SC 2.3.3; Apple HIG "make motion optional").

**Web** — the user's `prefers-reduced-motion` setting:

- The single `@media (prefers-reduced-motion: reduce)` block of `style.css` (S48 still finds exactly
  one) sets `animation: none !important` on every element, removes every transform transition and
  stagger delay, and keeps only opacity and colour cross-fades of at most `--t-fast`. The only literal
  duration allowed outside `:root` is `0s` inside that block.
- `@starting-style` entries keep only the opacity fade; the star sits at its final angle; the pending
  loop does not run; `startViewTransition` is skipped (`withViewTransition`).

**GUI** — owner decision 4 (2026-10-13). `MotionPrefs { reduced: bool }` is resolved in this order:

1. `--reduce-motion` (a `soos-gui` command-line switch) → reduced;
2. `SOOS_REDUCE_MOTION`: `1` → reduced, `0` → full motion (overrides the desktop), any other value
   (or more than 8 bytes) is ignored;
3. the XDG desktop portal, `org.freedesktop.portal.Settings.ReadOne` (fallback `Read` for portal
   version 1): `org.freedesktop.appearance` / `reduced-motion` (`u32`: 1 = reduce, 0 = no preference),
   then `org.gnome.desktop.interface` / `enable-animations` (`false` → reduced);
4. default: full motion.

The portal is read **once at start-up** on a background thread through zbus (the workspace builds zbus
with its `tokio` feature, so that thread runs its own current-thread runtime; the UI thread never
does), under one overall deadline of **200 ms** (`PORTAL_READ_DEADLINE_MS`). No session bus, no portal,
an error, an unexpected type, the deadline, or a disconnected channel (the reader thread ended or
panicked) means "no preference" (full motion), with no retry. Because the workspace zbus has no
blocking API, `feat/gui-motion` adds `zbus` and `tokio` as direct dependencies of `soos-gui` (both
already in `Cargo.lock`; no new crate enters the lockfile). This widens the "two crates only" D-Bus rule to
a read-only session-bus portal exception for `soos-gui` (one connection on the reader thread, only
`org.freedesktop.portal.Settings.ReadOne` / `Read` on `org.freedesktop.portal.Desktop` for the two keys
above, no object server, no well-known name, no signal match); `feat/gui-motion` migrates the PAU17
invariant (`presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon`) as recorded in the ADR
and `AI/tester_contract_motion.md`. The UI
thread only polls a channel (`try_recv`) and never blocks. Until the answer arrives (at most 200 ms)
the GUI treats motion as reduced, so a user who asked for reduced motion never sees a first animation.
A desktop setting changed while the GUI runs applies at the next launch (a live `SettingChanged`
subscription is a follow-up).

When reduced: every motion helper returns its target on the first frame, `Style::animation_time` is 0,
the pending star is static, and colour changes are instant.

## 9. egui repaint and CPU budget

- **Idle = zero motion repaints.** Once every animation has settled, motion code requests no repaint;
  frames come only from the existing camera and poll cadences. `crates/gui/tests/responsiveness_tests.rs`
  stays green unmodified.
- **Interactive transitions** call `request_repaint()` once per frame only while unsettled
  (display rate, vsync-capped), and stop on the frame they settle.
- **Ambient loop** (the pending star) uses `request_repaint_after` with 33 ms (`AMBIENT_REPAINT_MS`,
  at most 30 Hz).
- **Per frame**: O(1) work per animated widget, no allocation in `motion.rs`, no blocking I/O, no
  process spawn, no lock held across a frame.
- Tests never assert wall-clock CPU time (CI runners vary); the frame-time check (no visible dropped
  frame at 60 Hz during the tab and state-tile transitions, GNOME Wayland and X11) is an owner row.

## 10. Out of scope

- The GUI **dark variant** (owner decision 5): out of scope, a separate issue later. The web app's
  existing dark variant uses the same motion unchanged.
- New features, routes, settings, components or data (no toast system, no custom confirmation dialog).
- Any change to PAM, the daemon, IPC, the camera pipeline or the CSP.
- Native apps, the lock-screen widget, README imagery, sound and haptics.
- A visual-regression harness and the `crate:remote` label (decided in the later pull requests).

## 11. Prototype and owner approval

`Docs/prototypes/ui_motion_prototype.html` is a static, self-contained page (inline CSS and script, no
network request, no image, no font file, brand colours from `crates/gui/src/theme.rs` only, the star
and wordmark vectors already published in `crates/remote/assets/index.html`, a flat camera
placeholder). Open it in any browser (`xdg-open Docs/prototypes/ui_motion_prototype.html`). It shows
the token sheet with the three spring curves, a phone mock of the web app (W1–W8) and a desktop mock of
the GUI (G2–G8, the GUI spring run by the same closed-form equations as `motion.rs`), with a
**Reduce motion** toggle (initialised from `prefers-reduced-motion`) and a **Slow ×4** review toggle.
Prototype-only liberties (inline `<style>`/`<script>`, a script spring writing `transform`) are not
allowed in the product, where the CSP and the rules of section 6 apply.

Owner decision 3: the owner approves this prototype (matrix row **UIM7**) before `feat/remote-motion`
and `feat/gui-motion` merge.

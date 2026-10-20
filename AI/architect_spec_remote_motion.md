# Architect Spec — Web motion system of the `soos-remote` app (GitHub #433 = PR 2 of #358)

- **Date**: 2026-10-18
- **Branch**: `feat/remote-motion` (GitHub-only sub-issue #433, not registered in `scripts/sync_issue.py`;
  `Closes #433`, `Refs #358`)
- **Base commit**: `3f76090`
- **Normative inputs**: `Docs/UI_MOTION.md` §1–§6 and §8 (web), ADR "[2026-10-13] Motion System for soos-gui and
  soos-remote", `AI/architect_spec_ui_motion.md` D-9, tester contract `AI/tester_contract_motion.md` Part B.1.
- **Merge gate**: the owner approval of the prototype (matrix UIM7, owner decision 3 of 2026-10-13). The PR is opened
  with "DO NOT MERGE before UIM7".

## 0. Acceptance lines (from #358, `Docs/UI_MOTION.md` §6 and Part B.1)

| # | Line | Spec element |
|---|---|---|
| A1 | The seventeen tokens of §2 and the spring of §3 in `style.css` | D-1 |
| A2 | W1 status tile: state word re-entry per `.state-*` class, dot cross-fade, star turn | D-2, D-3 |
| A3 | W2 Lock / Unlock press, release spring, pending star bound to the request flags | D-4, D-12 |
| A4 | W3 native confirmation unchanged | D-12 (no change) |
| A5 | W4 `@starting-style` entries for sections revealed from `[hidden]`, instant hide | D-5 |
| A6 | W5 coverage banner slide and fade, attention cross-fade and a two-cycle nudge | D-6 |
| A7 | W6 `stagger-once` on the alert history and device lists | D-7, D-13 |
| A8 | W7 notification button fade, no switch | D-8 |
| A9 | W8 `withViewTransition` for the CSS-overlay camera full-screen path only | D-9, D-14 |
| A10 | W9 code input border colour, instant focus ring | D-10 |
| A11 | One reduced-motion block that removes every animation and transition | D-11 |
| A12 | CSP, asset set, element ids and labels unchanged; class-only hooks | D-12–D-15 |
| A13 | Tester contract Part B.1 items 1–12 as `tests/invariants/src/remote_motion_contract.rs`, rows RMO1–RMO12, UIM8 flipped, owner row pending | §3 |

## 1. CSS design (`crates/remote/assets/style.css`)

- **D-1 Tokens.** The "Scale" top-level `:root` block declares each token of §2 once with its pinned value (the
  three tokens already added by the feature tour, `--t-medium`, `--ease-enter`, `--rise`, stay where they are and are
  not repeated). `--ease-spring` is declared twice: the fallback `cubic-bezier(0.37, 0.73, 0.1, 1.15)` in that
  top-level `:root`, and the pinned `linear()` in a `:root` rule inside
  `@supports (transition-timing-function: linear(0, 1))` (one at-rule level).
- **D-2 W1 state word.** Each known state class gets its own keyframe name:
  `.state-locked` → `soos-state-locked`, `.state-unlocked` → `soos-state-unlocked`, `.state-no_session` →
  `soos-state-no-session`, `.state-unavailable` → `soos-state-unavailable`, `.state-unreachable` →
  `soos-state-unreachable`, `.state-unknown` → `soos-state-unknown`; each runs once over `--t-medium` with
  `--ease-enter`. Every keyframe has a single `from { opacity: 0.4; translate: 0 4px; }` step, so the end values are
  the element's own (opacity 1, or 0.7 for `state-unknown` / `state-unreachable`). `render()` rewrites `className`
  every second; an identical class keeps the same `animation-name`, so nothing replays on a poll. The dot
  cross-fades `background-color` and its existing inset `box-shadow` over `--t-medium` (standard). The tile itself
  has no state colour, so nothing else cross-fades.
- **D-3 Star.** `.stat-star` gets `transform-origin: center` and `transition: rotate var(--t-spring)
  var(--ease-spring)`; `.stat-tile[data-state="unlocked"] .stat-star { rotate: var(--star-turn); }` (the attribute
  `render()` already writes, no `:has()`). The pending loop is
  `.stat-tile.is-pending .stat-star { animation: soos-pending-turn var(--t-pending) var(--ease-standard) infinite; }`
  with `@keyframes soos-pending-turn { from { transform: rotate(0deg); } to { transform: rotate(90deg); } }`, so
  the state angle (`rotate`) and the loop (`transform`) compose.
- **D-4 W2 buttons.** The `button` transition becomes `background-color`, `color` and `box-shadow` over `--t-fast`
  (standard) plus `transform var(--t-spring) var(--ease-spring)` (release); `button:active:not(:disabled)` keeps
  its background, sets `transform: scale(var(--press-scale))` and its own press-in transition
  (`background-color` and `transform` over `--t-fast`, standard). The existing `transform: none` overrides
  (`button:disabled`, the tour replay link, the reduced-motion block) keep working because the press stays on
  `transform`. `button.is-pending` is `position: relative`; its `::after` is a 16 px star drawn with
  `background: currentColor` and `clip-path: path(...)` (the star mark scaled from 508 to 16 units), absolutely
  placed at the right padding edge (`right: 20px`, vertically centred), so the centred label never moves; it runs
  `soos-pending-turn` infinitely, and only under `.is-pending`.
- **D-5 W4 entries.** `.card.login, .card.alerts, .card.push, .card.enroll, .card.camera` transition `opacity` and
  `transform` over `--t-medium` with `--ease-enter`; the single top-level `@starting-style` block sets them to
  `opacity: 0; transform: translateY(var(--rise));`. No rest `transform` is declared (a non-`none` transform would
  make the card the containing block of the fixed full-screen camera stage). The tour entry (`.tour`) stays as it
  is and moves into the same block. No `transition-behavior: allow-discrete`: hiding is instant.
- **D-6 W5 banners.** `.banner` cross-fades `background-color`, `border-color` and `color` over `--t-medium`
  (standard) and enters with `opacity` and `transform` over `--t-short` (`--ease-enter`); `@starting-style` sets
  `opacity: 0; transform: translateY(calc(-1 * var(--slide)));`. `.banner::before` cross-fades its colours over
  `--t-medium`. `.alerts-attention .alerts-summary` runs `soos-nudge var(--t-medium) var(--ease-standard) 2`
  (`25% { translate: -3px 0; } 75% { translate: 3px 0; }`): two cycles, on `translate`, so it never fights the
  entry `transform`; it starts when the class is added and an identical toggle replays nothing.
- **D-7 W6 stagger.** `.stagger-once li` runs `soos-rise-in var(--t-medium) var(--ease-enter) backwards`
  (`from { opacity: 0; translate: 0 var(--rise); }`); `li:nth-child(2)` to `li:nth-child(5)` are delayed
  1–4 × `--t-stagger` and `li:nth-child(n+6)` 5 × `--t-stagger` (150 ms): the stagger stops at the sixth item and
  later items enter with it (§4).
- **D-8 W7.** `.push button` repeats the button transition and adds `opacity var(--t-short) var(--ease-enter)`;
  `@starting-style` sets `.push button { opacity: 0; }`. `#push-state` text is unchanged (set at once).
- **D-9 W8.** `.camera-stage { view-transition-name: camera-stage; }` (one stage only);
  `::view-transition-group(camera-stage)` runs over `--t-long` (standard),
  `html:not(.camera-lock)::view-transition-group(camera-stage)` over `--t-long-exit`, and the root cross-fade
  (`::view-transition-old(root), ::view-transition-new(root)`) over `--t-medium`.
- **D-10 W9.** `#enroll-code` transitions `border-color` over `--t-fast` (standard); `outline` is never transitioned.
- **D-11 Reduced motion.** The single `@media (prefers-reduced-motion: reduce)` block keeps its universal rule with
  `transition: none !important` (pinned by PTR-S8) and adds `animation: none !important`; the
  `button:active:not(:disabled) { transform: none; }` rule stays. Reduced motion is therefore fully instant: entries,
  the star angle, the pending loop, the nudge and the stagger all resolve at once. This is the strict end of §8 ("at
  most `--t-fast`"; the tour contract pins `none`).

## 2. `app.js` hooks (class toggles and one feature-detected call)

- **D-12 Pending.** One helper `syncPending()` toggles `is-pending` on `lockButton`, `unlockButton` and `statTile`
  from `lockRequestedAt !== 0` / `unlockRequestedAt !== 0` only. Every assignment to `lockRequestedAt` or
  `unlockRequestedAt` (set in `requestLock` / after the passkey assertion in `requestUnlock`; reset in
  `confirmLock`, `confirmUnlock`, `unlockFailed`, the non-202 and failure branches of `requestLock`, and both
  confirmation timers) is followed by `syncPending();`. No other `is-pending` write exists. `window.confirm` and the
  passkey sheet happen before `unlockRequestedAt` is set, so the star never runs during them.
- **D-13 Stagger.** `renderAlerts` and `renderPush` read the section's `hidden` flag before revealing it and call
  `list.classList.toggle("stagger-once", wasHidden)` right after `section.hidden = false`, before any early return:
  set for the first render after the section becomes visible, removed on the next one.
- **D-14 View Transition.** `withViewTransition(apply)` is the helper of `Docs/UI_MOTION.md` §6, verbatim.
  `applyCameraFull()` applies `camera-full`, `camera-lock` and the close button from the current `cameraFull` flag.
  `elementFullscreenAvailable(stage)` is `typeof (stage.requestFullscreen || stage.webkitRequestFullscreen) ===
  "function"`. `enterCameraFullscreen()` writes `cameraFull = true`, then applies directly and requests element
  fullscreen when it is available (inside the tap's activation), otherwise calls
  `withViewTransition(applyCameraFull)`. `exitCameraFullscreen(animate)` writes `cameraFull = false`, then uses
  `withViewTransition(applyCameraFull)` only when `animate === true` and element fullscreen is unavailable; the tap
  toggle and the Escape key pass `true`; `endCameraView` and `cameraFullscreenChanged` call it without argument
  (direct). A skipped transition still runs the callback, which reads the current flag.
- **D-15 Unchanged.** No `.animate(`, Web Animations API, `requestAnimationFrame`, `.style.`, `cssText`, new
  element id, asset, route or CSP change; `index.html` is not modified.

## 3. Test plan (Part B.1, module `remote_motion_contract`)

Twelve contract tests `test_rmo_s1` … `test_rmo_s12` (items 1–12 of Part B.1, in order) plus
`test_rmo_helper_self_test` (own one-level CSS parser, JS function extractor, duration and selector helpers). Rows
RMO1–RMO12 in a new `remote-motion` component of `AI/VERIFICATION_MATRIX.md`; RMO13 the owner device check (⬜
Pending, the same check as UIM10). Theme cross-check: `theme.rs` has no motion constants yet; the test compares
them when present (forward compatible), the pinned table otherwise.

## 4. Security invariants touched

None of PAM, daemon, IPC or the server. Page: security truth first (state words, classes, `data-state`,
`disabled` before any motion class), no optimistic "Unlocked", the native confirmation and passkey unchanged, no
loop over 5 s (WCAG 2.2.2), no frame captured for an effect (the View Transition snapshot is the browser's own).

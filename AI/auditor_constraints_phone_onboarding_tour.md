# Auditor Constraints — GitHub #414: first-run onboarding tour in the `soos-remote` web app

- **Date**: 2026-10-17
- **Inputs audited**: `AI/architect_spec_phone_onboarding_tour.md`, `AI/plan_evaluator_report.md`, the new contract `tests/invariants/src/remote_tour_contract.rs` (red: 10 of 11 tests fail on missing markers, functions, tokens and documents; the helper self-test passes), the lead's migration 73c24ee (`app_without_tour_storage`), and the code about to change: `crates/remote/assets/app.js`, `style.css`.
- **State before the change**: `app.js` and `sw.js` contain no `localStorage`, `sessionStorage`, `indexedDB`, `caches`, `document.cookie`, `globalThis`, `Reflect.`, `window[`, `.style.`, template literal or `console.` (grep, 0 hits each).

## Audit Constraints — Issue #414

| # | Constraint | Applies to (file::fn) | Verified by (test / lint / invariant / grep) |
|---|---|---|---|
| 1 | Exactly one storage block, markers on their own lines at column 0 and at top level; every `localStorage` use inside it; no comment anywhere in `app.js` that repeats the key literal or the word `localStorage` outside the block (the migrated RMC checks read raw text). | `app.js` top level | PTR-S1, PTR-S2, RMC-S18/S30/S38 |
| 2 | The block is the spec §3 code: one key, one fixed value `seen`, one `getItem`, one `setItem`, no `removeItem`/`clear`/enumeration/JSON; the `window.localStorage` getter itself is inside the `try` (it throws `SecurityError` when site data is blocked). | `app.js::tourStorage` | PTR-S2, PTR-S3 |
| 3 | Fail-safe order: `closeTour` hides the card before `tourStorage(true)` and ignores its result; `offerTour` opens on `false`. A blocked storage therefore shows the tour once per page load and never stops the rest of `app.js` (lock button, status stream). | `app.js::closeTour`, `offerTour` | PTR-S3, PTR-S7 |
| 4 | No other persistence or side channel: no `sessionStorage`, `indexedDB`, `caches`, cookies, `navigator.storage`; no dynamic global access (`window[`, `globalThis`, `Reflect.`) that could reach storage by a computed name. | `app.js`, `sw.js`, `index.html` | PTR-S4, RMC-S18 |
| 5 | Fixed text only: every tour string is a literal of `app.js` (plain ASCII, no markup, no escape, no `!`), every `setText` of the tour functions passes a fixed constant, a `step` field or the step caption built from two integers. No server view, response, event payload, input value, `location`, `navigator` or clock is read by the tour. | `app.js::TOUR_*`, tour functions | PTR-S5, PTR-S6, RXB-S5 |
| 6 | No request and no switch: the tour functions call no `fetch`/`postJson`/`EventSource`/`sendBeacon`, no action handler and no `.click()`; the in-page link only closes the tour and calls `focus()` (adding `tabindex="-1"` to a non-focusable card). | tour functions | PTR-S6, PTR-S9 |
| 7 | Accuracy of the off instructions: every manual `remote.toml` instruction names its dependants (`camera_view_funnel` with `allow_funnel` / `camera_view`; `push_notifications` and `offline_alerts` with `password_alerts`; `offline_alerts` with `push_notifications`; the `[android]` table with `allow_funnel`), so following it never makes `soos-remote` exit 78. | `app.js::TOUR_STEPS` (`where`) | PTR-S5; review against `crates/remote/src/config.rs` |
| 8 | Same wording: the six shared steps reuse `Feature::{title, explanation, when_off}` of `crates/gui/src/onboarding.rs` verbatim. | `app.js::TOUR_STEPS` | PTR-S5 |
| 9 | CSP and markup: `index.html`, `http.rs` (CSP), `sw.js`, `assets.rs` and the asset set unchanged; no inline style, no `.style.`, no `cssText`, no Web Animations, no `startViewTransition` in the tour; elements created are `section`, `p`, `h2`, `div`, `button` only; the only attributes set are `aria-label` (fixed) and `tabindex="-1"`. | `app.js`, `index.html` | PTR-S9, RXB-S8, RMC-S49, RMC-S50 |
| 10 | Style: role tokens only (no colour literal outside the palette), motion tokens `--t-medium`, `--ease-enter`, `--rise` declared once with the `Docs/UI_MOTION.md` values, no literal duration, exactly one reduced-motion block that still removes every transition; the footer button keeps the 48 px touch height and a visible focus ring; contrast pairs stay WCAG AA. | `style.css` | PTR-S8, RMC-S45–S48, UIM-S2 |
| 11 | Feature detection (browser support ADR): the tour uses only DOM features present in every supported engine (`createElement`, `addEventListener`, `focus`, `hidden`, `querySelector`); no new guarded API, no User-Agent logic. | tour functions | RXB-S1, RXB-S2, RXB-S6 |
| 12 | Tests immutable: no existing test file is edited; only `remote_tour_contract.rs` (new) and its registration in `tests/invariants/src/lib.rs` are added. | `tests/` | `git diff --stat origin/main -- tests/` shows 73c24ee plus these two files only |

### Pre-existing observation (not introduced by this change)

- `soos-gui` turning `allow_funnel` off (`feature_control.rs` `RemoteSwitch::Funnel` clears `allow_funnel` and `camera_view_funnel`) does not remove an `[android]` table; `soos-remote` then refuses the file (`[android] requires allow_funnel`, exit 78). The phone tour names the manual step; the GUI gap is a follow-up for the lead.

### Clearance: CLEARED

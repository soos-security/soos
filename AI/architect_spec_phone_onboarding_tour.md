# Architect Spec — GitHub #414: First-Run Onboarding Tour in the `soos-remote` Web App

- **Issue**: GitHub-only #414 (PR 2 of epic #363; PR 1, the `soos-gui` tour, merged as #389 /
  GitHub #379), no backlog id. **Branch**: `feat/phone-onboarding-tour` (not registered in
  `scripts/sync_issue.py`; the commit carries `Closes #414` and `Refs #363`).
- **Owner decisions**: #363 (2026-10-12): a short tour on the phone with the same wording as the
  PC tour, brand and motion system, reduced motion respected, English UI text. #363 (2026-10-17):
  narrow exception to the web app's no-storage contract (RMC-S18, RMC-S30, RMC-S38): one
  `localStorage` key `soos-remote-tour-v1` (seen / not seen, no personal data) through a single
  guarded helper in `app.js`; a new contract proves it is the only storage use. The lead applied
  the matching migration in commit 73c24ee (`app_without_tour_storage` strips the one block
  between `// soos-tour-storage:begin` and `// soos-tour-storage:end`).
- **Out of scope**: the same tour in the native Android app (#357), a follow-up; any server,
  route, CSP, `index.html` or asset-set change; showing or changing a feature's state.

## 1. Blast Radius

| File | Change |
|---|---|
| `crates/remote/assets/app.js` | tour data, the single storage block, tour card and footer button built at run time, first-run offer in `showApp`, Escape handler; header comment updated |
| `crates/remote/assets/style.css` | tour card, navigation row, footer replay button; motion tokens `--t-medium`, `--ease-enter`, `--rise` (values of `Docs/UI_MOTION.md` §2) and a W4 `@starting-style` entry |
| `tests/invariants/src/remote_tour_contract.rs` | new contract (PTR-S1 to PTR-S10) |
| `tests/invariants/src/lib.rs` | registers `mod remote_tour_contract` |
| `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md` (PTR1–PTR12), `Docs/REMOTE_COMPANION.md` §2k, `AI/walkthroughs/222_phone_onboarding_tour.md` | traceability |

Unchanged: `index.html` (RMC-S50 pins the id set, so every new element is built by `app.js`,
like the camera and offline cards), `sw.js`, `http.rs` (CSP), `assets.rs`, routes, the eleven-file
asset set (RXB-S8). No Rust production code changes; no dependency.

## 2. Behaviour

- **Pages**: index 0 is the introduction, 1..8 the feature steps (`TOUR_STEPS`, 8 entries).
  A caption "Step N of 9", the title (`h2`, `tabIndex = -1`), the explanation, the "Off:" line, a
  "where to turn it off" info banner, a **Show it on this page** button, **Back** / **Next**
  (**Done** on the last page) and **Skip tour** (hidden on the last page). Labels are the GUI's.
- **First run**: `showApp` (tailnet page, or Funnel after a successful sign-in) calls
  `offerTour()`: once per page lifetime (`tourOffered`), it opens the tour when
  `tourStorage()` is `false`. The Funnel sign-in screen never opens it on its own.
- **Dismiss**: Skip, Done and Escape (while no camera full screen is open) call `closeTour()`,
  which hides the card first, gives the focus back to **Feature tour** when the tour was opened
  from it, and then calls `tourStorage(true)`, ignoring the result. `showLogin` hides an open
  tour without writing the flag, so the tour never stays over the sign-in screen.
- **Replay**: a **Feature tour** button in the footer (before the source link), at any time,
  signed in or not, opens the tour on the introduction.
- **Links**: `target` names a card of this page: `status` (status card), `logout` (Sign out),
  `alerts`, `push`, `camera` (runtime card), `offline` (runtime card) or `""`. **Show it on this
  page** is visible only when the target and all its ancestors are not `hidden`; it closes the
  tour (counts as dismissed), gives a non-focusable target `tabindex="-1"` and calls `focus()`,
  which scrolls it into view. It never clicks, submits or changes anything.
- **Blocked storage** (private window, site data disabled, `SecurityError` on access): the
  helper catches every exception and answers `false`, so the tour opens once per page load, is
  closed by Skip / Done / Escape as usual, and no error reaches the console.

## 3. Storage Block (exact code, comments free)

```js
// soos-tour-storage:begin
const TOUR_STORAGE_KEY = "soos-remote-tour-v1";
const TOUR_STORAGE_SEEN = "seen";
function tourStorage(markSeen) {
  try {
    if (markSeen === true) {
      window.localStorage.setItem(TOUR_STORAGE_KEY, TOUR_STORAGE_SEEN);
      return true;
    }
    return window.localStorage.getItem(TOUR_STORAGE_KEY) === TOUR_STORAGE_SEEN;
  } catch (_) {
    return false;
  }
}
// soos-tour-storage:end
```

- The block exists once; its comment-free code is exactly the above (whitespace-insensitive).
- Outside it: `localStorage`, `TOUR_STORAGE_KEY`, `TOUR_STORAGE_SEEN` and the key literal never
  appear; `tourStorage(` appears exactly twice: `tourStorage(true);` in `closeTour` (after the
  card is hidden) and `!tourStorage()` in `offerTour`.
- Value domain: only the fixed string `seen` is ever written; any other stored value reads as
  "not seen". No `removeItem`, `clear`, `key()`, enumeration or JSON.

## 4. Tour Text (fixed English, no user data)

`TOUR_STEPS` is an array literal of object literals whose five fields (`title`, `text`, `off`,
`where`, `target`) are plain double-quoted literals: printable ASCII, no backslash, backtick,
`!`, `<`, `>`, `{`, `}`, `$` or `&`. The six features shared with `soos-gui` (Phone companion,
Remote lock and unlock, Internet access, Push alerts, Live camera view, Offline alerts) keep the
exact `title()`, `explanation()` and `when_off()` strings of `crates/gui/src/onboarding.rs`, in the
GUI order. Two phone-only steps: **Battery level** (`battery_status`) and **Failed-password
history** (`password_alerts`). Each `where` names the way to turn the feature off: the
`soos-gui` Features tour where it has a switch, and the exact `remote.toml` key(s) with their
dependants (a manual edit that leaves a dependant on makes `soos-remote` exit 78):

| Step | Off by hand | Dependants also set to `false` |
|---|---|---|
| Phone companion | `systemctl --user disable --now soos-remote.service` | — |
| Battery level | `battery_status = false` | — |
| Remote lock and unlock | `allow_unlock = false` (locking stays) | — |
| Internet access | `allow_funnel = false` | `camera_view_funnel`; the `[android]` table removed |
| Failed-password history | `password_alerts = false` | `push_notifications`, `offline_alerts` |
| Push alerts | `push_notifications = false` (or Disable notifications on this phone only) | `offline_alerts` |
| Live camera view | `camera_view = false` | `camera_view_funnel` |
| Offline alerts | `offline_alerts = false` | — |

Every `setText` call of the tour functions passes a `TOUR_*` constant, a `step.<field>`, `" "`,
the Next/Done ternary of two `TOUR_*` labels or the caption `"Step " + (index + 1) + " of " +
total`. The tour functions never read `latest`, `alertsView`, `pushView`, `batteryView`, a
response, an event payload, an input value, `location`, `navigator` or the clock.

## 5. No Request, No Switch

The tour functions (`buildTourCard`, `tourTarget`, `shownOnPage`, `renderTour`, `openTour`,
`closeTour`, `tourNext`, `tourBack`, `showTourTarget`, `offerTour`, `tourKeydown`) contain no
`fetch(`, `postJson(`, `EventSource`, `XMLHttpRequest`, `sendBeacon`, `.register(`,
`window.confirm(`, `.click(`, `.submit(` and call no action handler (`requestLock`,
`requestUnlock`, `requestLogin`, `requestLogout`, `requestEnroll`, `requestAlertsAck`,
`enableNotifications`, `sendTestNotification`, `disableNotifications`, `startCameraView`,
`stopCameraView`, `fetchStatus`, `fetchAlerts`, `fetchOffline`, `fetchPush`, `fetchCamera`,
`resume`).

## 6. Style and Motion

- Role tokens only (RMC-S45 already enforces palette-only colours); the footer button reads as a
  link (`--link`, underline, transparent background) with the 48 px `--touch` height; its hover
  and press overrides keep a transparent background (the hover override sits in the existing
  `@media (hover: hover)` block).
- Motion (Docs/UI_MOTION.md §6 W4): `.tour` transitions `opacity` and `transform` over
  `var(--t-medium)` with `var(--ease-enter)`, from `@starting-style { opacity: 0;
  transform: translateY(var(--rise)) }`. Tokens added to the scale `:root` with the pinned values
  `260ms`, `cubic-bezier(0.16, 1, 0.3, 1)`, `6px` (UIM-S2 cross-checks them). No literal duration,
  no keyframes, no JavaScript animation, no `.style.`. Without `@starting-style` the card appears
  at once; under `prefers-reduced-motion: reduce` the single existing block already removes every
  transition, so the card appears at once.
- The `h2` focus target gets the brand focus ring (`.tour h2:focus-visible` joins the existing
  `:focus-visible` rule).

## 7. Security Invariants Touched

- CSP unchanged (no `unsafe-inline`), no inline style or handler, no `innerHTML`, no template
  literal, no new origin, no network call, no new asset, no User-Agent logic.
- The stored datum is one fixed word; it identifies nothing and is readable by the page's own
  origin only. Clearing site data resets it.
- The tour shows no server data, so it cannot leak alert, push, camera or battery details.

## 8. Acceptance Mapping

| #363 / #414 line | Element |
|---|---|
| Phone: short tour on first sign-in in the web app | §2 first run, PTR-S7 |
| Same wording | §4 shared strings, PTR-S5 |
| Explains each phone-side feature and its exposure, links to where it can be turned off | §4, §2 links, PTR-S5, PTR-S6 |
| Switches nothing on its own | §5, PTR-S6 |
| One key, fixed value, guarded, only storage use | §3, PTR-S1–PTR-S4 |
| Brand, motion, reduced motion | §6, PTR-S8 |
| CSP, no innerHTML, feature detection, no new origin, no network | §7, PTR-S9 and RXB/RMC contracts |
| English text, docs, ADR, matrix, walkthrough | PTR-S10 |
| Native app | follow-up (out of scope) |

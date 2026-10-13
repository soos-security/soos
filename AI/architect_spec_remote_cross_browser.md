# Architect Spec — soos-remote: browser capability detection and graceful degradation (GitHub #366, PR 1 of #355)

- **Date**: 2026-10-13
- **Issue**: GitHub #366 (PR 1 of GitHub #355, planned split item 1). GitHub-only issue: no `AI/BACKLOG.md` entry,
  **not** registered in `scripts/sync_issue.py` (dev-workflow Phase 0 rule); the commit carries `Closes #366`
  (and `Refs #355`).
- **Branch**: `feat/remote-cross-browser` (from `origin/main` `1b6d211`)
- **Owner decisions applied** (#355 comment "Owner decisions (2026-10-13)"):
  OD-1 mandatory browsers Safari iOS/macOS, Chrome, Firefox, Samsung Internet; best effort with clean
  degradation for every other browser. OD-2 (Playwright, PR 2) undecided and **out of scope** here: no Node/npm.
  OD-3 Edge desktop push stays out of scope: a clear "push not supported here" message, no WNS, no new outbound
  host. OD-4 floor = Baseline widely available, no explicit Firefox ESR 115 commitment. OD-5 per-feature
  messages only, no "This browser" capability panel.
- **Walkthrough**: 197 (196 is reserved by the parallel branch `feat/network-lock`).

## 0. Summary of decisions

| ID | Decision |
|---|---|
| D-1 | One pure, synchronous capability probe `browserCapabilities()` in `app.js`, feature detection only, recomputed on every call (no cache, no storage, no network, no DOM write). |
| D-2 | Every call of `navigator.credentials`, `navigator.serviceWorker`, `.pushManager`, `Notification.`, `new EventSource(`, `.getReader(` and `createImageBitmap(` sits in a top-level function that checks the matching capability before the first call (static rule, §8 RXB-S2). |
| D-3 | **No User-Agent at all** (stricter than the issue's "UA for wording only"): iOS wording uses the `"standalone" in navigator` feature, Edge desktop wording comes from the server's existing `unsupported_push_service` answer. The server keeps never reading `User-Agent`. |
| D-4 | No WebAuthn: login, unlock, enroll and camera-start buttons stay disabled with the fixed `PASSKEY_UNSUPPORTED_TEXT`; every handler re-checks before its first request; lock and status are untouched. |
| D-5 | Push: distinct fixed texts for iOS outside the home-screen app, no service worker (private window), no working push service (Brave, ungoogled builds, `AbortError`/`NotSupportedError`), permission denied (`NotAllowedError`, kept `PUSH_BLOCKED_TEXT`), a stale subscription (`InvalidStateError`), the PC not answering in time, and the server's `unsupported_push_service` (naming Edge on a computer). The whole tap after the permission answer is bounded by the existing `SERVICE_WORKER_READY_TIMEOUT_MS` (10 s). A subscription the PC refuses as `unsupported_push_service` is dropped locally (best effort). |
| D-6 | User messages are fixed strings: a browser exception's `message` is never displayed; `fixedError(text)` marks this file's own texts and `failureText(error, names, fallback)` is the only reader of `.message`. |
| D-7 | No `EventSource`: the page polls `GET /api/status` every `STATUS_POLL_MS` (15 000 ms, the server keep-alive cadence) instead of the stream; one timer, cleared by `closeStream`. |
| D-8 | Camera: without streamed fetch bodies (`ReadableStream`, `Response.body`, `AbortController`) or `createImageBitmap`, the start button is hidden and the state line shows `CAMERA_UNSUPPORTED_TEXT`. Parser bounds unchanged. |
| D-9 | CSS: every `:has()` rule gets an identical-declaration fallback inside `@supports not selector(:has(*))`, keyed on hooks `app.js` sets (`data-state` on `.stat-tile`, class `feedback-last` on `#feedback`); the portrait camera canvas gets a `@supports not (container-type: size)` fallback; `100vh` keeps preceding `100dvh`. Tokens, palette and contrast unchanged. |
| D-10 | `sw.js`, `index.html`, `manifest.webmanifest`, every Rust source, CSP, `MANDATORY_HEADERS`, `HARDENING_HEADERS`, CSRF, cookie, WebAuthn verifier, push allowlist and the 11-file asset set stay byte-identical. |

## 1. Verified assumptions (against the code at `1b6d211`)

| # | Fact | Location | Verified |
|---|---|---|---|
| V-1 | `navigator.credentials.get` has a single call site, inside `getAssertion`; `getAssertion(` occurs exactly 4 times (definition + 3 calls); `allowCredentials` exactly once; `.register(` exactly once; the words `conditional`, `https://`, `http://`, `blob:`, `innerHTML` never occur in `app.js` (comments included) | `app.js:288-313`; pins in `remote_direct_biometric_contract.rs:136-207`, `remote_passkey_contract.rs:313-370`, `remote_companion_contract.rs:1045-1056`, `remote_camera_contract.rs:784-795` | Yes |
| V-2 | `PASSKEY_PROMPT_TEXT`, `PUSH_UNSUPPORTED_TEXT`, `PUSH_BLOCKED_TEXT`, `PUSH_WORKER_FAILED_TEXT` are pinned byte-exact (RAN-S6); "Face ID" must always be followed by ", fingerprint or screen lock"; "Touch ID", "iPhone settings", "Passwords app" forbidden | `remote_android_contract.rs:81-98, 904-960` | Yes |
| V-3 | `enableNotifications`: first `await` is `Notification.requestPermission(`; contains `await readyRegistration()`, `PUSH_WORKER_FAILED_TEXT` (after the ready, before `pushManager.subscribe(`, with `=== null` between), `PUSH_BLOCKED_TEXT`, `PUSH_UNSUPPORTED_TEXT`, `finally`; never `serviceWorker.ready` | `remote_android_contract.rs:1040-1078`, `remote_push_contract.rs:719-730` | Yes |
| V-4 | `readyRegistration` must contain `navigator.serviceWorker.ready`, `setTimeout(`, `clearTimeout(`, `SERVICE_WORKER_READY_TIMEOUT_MS`, `resolve(null)`, `registerServiceWorker()`; `navigator.serviceWorker.ready` occurs once; `syncSubscription` has exactly one `.unsubscribe(` | `remote_android_contract.rs:990-1040` | Yes |
| V-5 | `pushSupported()` = `serviceWorker in navigator && PushManager in window && Notification in window`; no test pins its body | `app.js:981-983`; grep of `tests/` | Yes |
| V-6 | `:has()` rules: four `.stat-tile:has(.state-*) .stat-dot` rules and the selector list `.card > .feedback:last-child, .card > .feedback:has(+ [hidden]:last-child)` (one rule: an engine without `:has()` drops the **whole** list, so the five last-child feedback lines lose their margin too, PE F-1); the only non-last feedback is `#feedback`, followed by `#logout` (hidden on the tailnet) | `style.css:400-412, 693-696`, `index.html:63-68` | Yes (corrected in round 2) |
| V-7 | `.camera-canvas` already declares `width: 100%; height: 100%` before `100cqw/100cqh`; the portrait rule `width: 100cqh; height: 100cqw` has no fallback (dropped declarations fall back to 100 % x 100 %, a wrong box); `body` declares `min-height: 100vh` before `100dvh` | `style.css:787-803, 230-233` | Yes |
| V-8 | Brand CSS parser accepts one level of at-rule nesting; colour properties outside token blocks must be role/scale `var()` values; BINDINGS pin `.stat-tile`, `.card` backgrounds; no `url(`; exactly one `prefers-color-scheme: dark` and one `prefers-reduced-motion` block | `remote_brand_contract.rs:563-617, 797-910, 1419-1990` | Yes |
| V-9 | The server never reads `User-Agent` (no `user-agent` in `crates/remote/src`); the only `User-Agent` in the workspace's companion crates is the sender's own outbound constant | grep `crates/remote/src`, `crates/push-sender/src/lib.rs:41` | Yes |
| V-10 | `unsupported_push_service` is the server answer for a non-allowlisted endpoint host (Edge desktop `*.notify.windows.com`) | `crates/remote/src/push.rs:1460` | Yes |
| V-11 | `Docs/REMOTE_COMPANION.md` has `## 2h.` followed by `## 3.`; RAN-S9 requires `2g < 2h < 3`; sections are read up to the next `## ` | `remote_android_contract.rs:1193-1199` | Yes |
| V-12 | Error texts shown today include browser `DOMException` messages (`error.message`) in login, enroll, push enable, camera start and camera stream | `app.js:867, 955, 1197, 1726, 1779` | Yes |

## 2. Scope and blast radius

| File | Change |
|---|---|
| `crates/remote/assets/app.js` | probe, guards, fixed texts, bounded push enable, SSE polling fallback, camera degradation, CSS hooks |
| `crates/remote/assets/style.css` | two `@supports` fallback blocks (append only) |
| `tests/invariants/src/remote_cross_browser_contract.rs` | new (RXB-S1 … RXB-S11 + helper self-test), self-contained JS/CSS lexers (the android helpers stay private and untouched) |
| `tests/invariants/src/lib.rs` | register the module (`#[cfg(all(test, unix))]`) |
| `Docs/REMOTE_COMPANION.md` | new `## 2i. Browser support` (between 2h and 3), troubleshooting rows (DNS-over-HTTPS, push texts), §8 residuals, §9 pointer |
| `AI/DECISIONS.md` | ADR "[2026-10-13] Browser Support Policy for the `soos-remote` Web App" (appended last) |
| `AI/VERIFICATION_MATRIX.md` | component section `remote-cross-browser`, rows RXB1–RXB20 (appended last) |
| `AI/walkthroughs/197_remote_cross_browser.md` | new |
| `AI/plan_evaluator_report.md`, `AI/tester_contract_remote_cross_browser.md`, `AI/auditor_constraints_remote_cross_browser.md` | phase deliverables |

Not touched: every Rust source of every crate, `sw.js` (reviewed: no fetch handler, no cache, `pushsubscriptionchange`
is a no-op where browsers never fire it), `index.html`, manifest, icons, packaging, scripts, CI. No consumer
outside the asset files: the server embeds the assets with `include_bytes!` and serves them unchanged.

## 3. `app.js` design

> Round 2 (plan evaluator N-1): where §3.2, §3.4, §4 or §8 differ from §14 (R-1 … R-7), §14 is normative.

### 3.1 Probe (D-1)

```js
// The browser features the page relies on, from feature detection only (never the User-Agent).
// Pure: no request, no DOM write, no storage; recomputed on every call.
function browserCapabilities() {
  const nav = window.navigator;
  const store = nav.credentials;
  const display = typeof window.matchMedia === "function" ? window.matchMedia("(display-mode: standalone)") : null;
  return {
    webauthn: typeof window.PublicKeyCredential !== "undefined" && !!store &&
      typeof store.get === "function" && typeof store.create === "function",
    serviceWorker: "serviceWorker" in nav,
    push: "PushManager" in window,
    notifications: "Notification" in window,
    standalone: nav.standalone === true || (display !== null && display.matches === true),
    homeScreen: "standalone" in nav,          // iOS/iPadOS WebKit: wording only
    eventSource: typeof window.EventSource !== "undefined",
    streams: typeof window.ReadableStream !== "undefined" && typeof window.AbortController !== "undefined" &&
      typeof window.Response !== "undefined" && "body" in window.Response.prototype,
    imageBitmap: typeof window.createImageBitmap === "function",
  };
}
```

Bounds/sentinels: every flag is a boolean; an absent global is `false`; `matchMedia` absent ⇒ `standalone` from
`navigator.standalone` only. The issue's `platformAuthenticatorHint` (async `isUserVerifyingPlatformAuthenticatorAvailable`)
and `privateLikely` (not detectable synchronously) are **dropped** (A-1): no message needs them under OD-5.
`PublicKeyCredential` and `PushManager` occur **only** inside the probe. `pushSupported()` becomes
`caps.serviceWorker && caps.push && caps.notifications` from one `browserCapabilities()` call.

### 3.2 Fixed texts and error mapping (D-6)

```js
function fixedError(text)                       // Error whose message is one of this file's texts, .fixed = true
function failureText(error, names, fallback)    // fixed ⇒ message; own-property name in `names` ⇒ names[name]; else fallback
```
`failureText` is the only reader of `.message`. Tables (exact):

```js
const PASSKEY_ERROR_TEXTS = {
  NotAllowedError: "The passkey check was cancelled or timed out",
  SecurityError: "This address cannot use the passkey: open the exact address the passkey was made for",
  InvalidStateError: "This passkey is already registered",
  NotSupportedError: "This browser or device cannot create this kind of passkey",
};
const PUSH_ERROR_TEXTS = {
  NotAllowedError: PUSH_BLOCKED_TEXT,
  AbortError: PUSH_NO_SERVICE_TEXT,
  NotSupportedError: PUSH_NO_SERVICE_TEXT,
  InvalidStateError: PUSH_STALE_TEXT,
  SecurityError: PUSH_NO_WORKER_TEXT,
};
```

New constants (exact values; ASCII apart from none; each defined once, each distinct):

| Constant | Value |
|---|---|
| `PASSKEY_UNSUPPORTED_TEXT` | `"This browser cannot use passkeys, so sign-in, unlock, passkey enrollment and the camera view are off here; lock and status still work on the tailnet"` |
| `PUSH_HOME_SCREEN_TEXT` | `"On iPhone and iPad, notifications only work in the home-screen app: tap Share, then Add to Home Screen, and open soos from the new icon (iOS 16.4 or later)"` |
| `PUSH_NO_WORKER_TEXT` | `"Notifications need a service worker, which this window does not allow (private windows block it): open the page in a normal window"` |
| `PUSH_NO_SERVICE_TEXT` | `"This browser has no working push service: in Brave, turn on Use Google services for push messaging; otherwise use Chrome, Firefox, Safari or Samsung Internet"` |
| `PUSH_STALE_TEXT` | `"An older notification subscription is in the way: reload the page, then enable notifications again"` |
| `PUSH_TIMEOUT_TEXT` | `"The PC did not confirm the subscription in time: check the connection and try again"` |
| `CAMERA_UNSUPPORTED_TEXT` | `"This browser cannot show the live camera view (no streamed download or image decoding): use Safari, Chrome, Firefox or Samsung Internet"` |
| `PUSH_REASONS.unsupported_push_service` (changed value) | `"The PC does not accept this browser's push service (Edge on a computer, for example): use Chrome, Firefox, Safari or Samsung Internet for notifications"` |

The four pinned texts (V-2) are unchanged. Every `throw new Error(<user text>)` whose message may be displayed
becomes `throw fixedError(...)` (login, enroll, push enable, camera start, camera stream reader/opener); the
internal `throw new Error("status …")` of fetch helpers stays (never displayed: their catches show fixed texts).
Displays: `requestLogin` → `failureText(error, PASSKEY_ERROR_TEXTS, "Sign-in cancelled")`; `requestEnroll` →
`(…, "Enrollment cancelled")`; `requestUnlock` → `(…, "Unlock cancelled or failed")`; `startCameraView` →
`(…, "Camera view cancelled")`; `openCameraStream` → `failureText(error, {}, "The camera view ended")`;
`enableNotifications` → `failureText(error, PUSH_ERROR_TEXTS, PUSH_NO_SERVICE_TEXT)`.

### 3.3 Passkey degradation (D-4)

- `showLogin(message)`: `loginButton.disabled = !browserCapabilities().webauthn`; feedback = `PASSKEY_UNSUPPORTED_TEXT`
  when unsupported, else `message || " "`.
- `showApp(state)`: when unsupported, `enrollButton.disabled = true`, `enrollFeedback` and `feedbackNode` show
  `PASSKEY_UNSUPPORTED_TEXT`.
- `render()`: `unlockButton.disabled = !(reachable && view.state === "locked" && browserCapabilities().webauthn)`.
- `renderCamera()`: when unsupported, `startButton.disabled = true` and the camera feedback shows the text.
- Rule (static, RXB-S3): every assignment `loginButton.disabled =`, `enrollButton.disabled =`,
  `unlockButton.disabled =`, `startButton.disabled =` is either `= true` or contains `browserCapabilities().webauthn`
  (the old `= false` resets in `finally`/`endCameraView` become `= !browserCapabilities().webauthn`).
- `requestLogin`, `requestUnlock`, `requestEnroll`, `startCameraView` start with
  `if (!browserCapabilities().webauthn) { setText(<feedback>, PASSKEY_UNSUPPORTED_TEXT); return; }` before any
  `window.confirm`, `postJson` or `fetch` (no challenge is issued for a browser that cannot answer it).
- `getAssertion` and the `navigator.credentials.create` path in `requestEnroll` keep a guard right before the call
  (`Promise.reject(fixedError(PASSKEY_UNSUPPORTED_TEXT))` in `getAssertion`).
- `requestLock`, `fetchStatus`, `openStream` never reference `webauthn` (lock and status independent of passkeys).

### 3.4 Push degradation (D-5)

```js
function pushUnsupportedText(fallback)   // homeScreen && !standalone ⇒ PUSH_HOME_SCREEN_TEXT;
                                          // !serviceWorker ⇒ PUSH_NO_WORKER_TEXT; else fallback
function beforeDeadline(promise, deadline, text)  // settles like promise, or rejects fixedError(text) at
                                                    // `deadline` (Date.now() ms, delay max(0, deadline-now)); timer always cleared
```
- `renderPush` and `enableNotifications` show `pushUnsupportedText(PUSH_UNSUPPORTED_TEXT)` when `!pushSupported()`.
- `enableNotifications`: unchanged order (permission first, then `readyRegistration`, null ⇒ `PUSH_WORKER_FAILED_TEXT`);
  `const deadline = Date.now() + SERVICE_WORKER_READY_TIMEOUT_MS;` right after a granted permission;
  `pushManager.subscribe(…)` wrapped in `beforeDeadline(…, deadline, PUSH_NO_SERVICE_TEXT)`; the subscribe
  `postJson` wrapped in `beforeDeadline(…, deadline, PUSH_TIMEOUT_TEXT)`; on `result === "unsupported_push_service"`
  `subscription.unsubscribe()` best effort (not awaited, rejection swallowed) then `throw fixedError(reasonFor(…))`.
  Bound: from the permission answer the button is re-enabled at most `SERVICE_WORKER_READY_TIMEOUT_MS` (+ timer
  slack) later on every path (the ready wait arms its own 10 s timer after `deadline` is fixed, so it ends no later).
  The permission prompt itself is user-paced (A-2).
- `readyRegistration`, `disableNotifications` gain a `pushSupported()` guard before their first push/worker API.
- Sentinel: `deadline - Date.now() <= 0` ⇒ the timer fires at once (0 ms) and the step fails with its fixed text; a
  late-settling subscribe is kept locally and re-sent by `syncSubscription` on the next open (existing behaviour).

### 3.5 Status without EventSource (D-7)

`const STATUS_POLL_MS = 15000;` `let statusPoll = null;`. `openStream()`: after `closeStream()`, when
`!browserCapabilities().eventSource` ⇒ `statusPoll = setInterval(function () { fetchStatus(true); }, STATUS_POLL_MS); return;`.
`closeStream()` clears `statusPoll`. The battery line stays hidden (it only comes from the stream; fail closed).

### 3.6 Camera degradation (D-8)

`renderCamera`, after the reachability branch: `if (!(caps.streams && caps.imageBitmap))` ⇒ state line
`CAMERA_UNSUPPORTED_TEXT`, start and stop hidden, return. `readCameraStream` starts with a `streams` guard
(reject `fixedError(CAMERA_UNSUPPORTED_TEXT)`); `showCameraFrame` returns early unless `imageBitmap`.

### 3.7 CSS hooks (D-9)

- `const statTile = statusCard.querySelector(".stat-tile");` `render()` sets `statTile.dataset.state` to the state
  when it is a `LABELS` key, else `"unknown"` (server data never reaches an attribute unfiltered).
- `function setLogoutHidden(hidden)`: `logoutButton.hidden = hidden; feedbackNode.classList.toggle("feedback-last", hidden);`
  used by `showLogin` (`true`) and `showApp` (`state.mode !== "funnel"`).

## 4. `style.css` design (D-9), appended after the camera rules

```css
/* Fallbacks for engines without :has() (same visual state, hooks set by app.js). */
@supports not selector(:has(*)) {
  .stat-tile[data-state="locked"] .stat-dot { background: var(--dot-locked); }
  .stat-tile[data-state="unlocked"] .stat-dot { background: var(--banner-warn-border); box-shadow: inset 0 0 0 var(--stroke-card) var(--dot-unlocked); }
  .stat-tile[data-state="unavailable"] .stat-dot,
  .stat-tile[data-state="unreachable"] .stat-dot { background: var(--dot-alarm); }
  .card > .feedback.feedback-last { margin-bottom: -16px; }
}
/* Engines without size containers: the portrait canvas box is the stage turned by 90 degrees. */
@supports not (container-type: size) {
  .camera-stage.camera-portrait .camera-canvas { width: 133.333%; height: 75%; }
  .camera-stage.camera-full.camera-portrait .camera-canvas { width: 100vh; height: 100vw; }
}
```
Card portrait stage is 3:4, so its height is 4/3 of its width: the canvas box (stage height x stage width) is
133.333 % x 75 % of the stage (exact while `max-height: 70vh` does not cap the stage; when it caps,
`object-fit: contain` keeps the image undistorted and `overflow: hidden` clips the box, A-3). Full screen covers
the viewport, so viewport units give the turned box. Each `:has()` fallback declares exactly the declarations of
its `:has()` rule (pinned by RXB-S7). No colour value, token or contrast pair changes.

## 5. Constants and sentinel semantics

| Name | Location | Value | Semantics |
|---|---|---|---|
| `SERVICE_WORKER_READY_TIMEOUT_MS` | `app.js` (existing) | 10000 | also the whole post-permission budget of `enableNotifications` |
| `STATUS_POLL_MS` | `app.js` (new) | 15000 | only without `EventSource`; one interval, cleared by `closeStream` |
| texts of §3.2 | `app.js` | fixed | never concatenated with browser or server text |

## 6. Error taxonomy

Client-side only. Browser exception → `failureText` table (§3.2) → fixed text. Server refusal → existing
`reasonFor` tables (`PUSH_REASONS`, `PASSKEY_REASONS`, `CAMERA_REASONS`). No server status, verdict or route changes.

## 7. Latency budget / invariants touched

Not on the PAM path. Invariants: CSP unchanged (RLC-S6, RAN-S5 pins), RMC-S8 (textContent only, no URL), RMC-S18
(modal WebAuthn, counts), RAN-S6/S7 (pinned texts, bounded worker wait), RDB-S1/S2 (call-site counts), brand
RMC-S44–S54 (tokens, contrast, CSS parse). Security invariants of the issue: a client capability flag never unlocks
a server path (all server checks unchanged, no Rust change); no UA logic; fixed-string messages; no new origin;
no sensitive data in logs (the page and worker have no `console` call; RXB-S5 keeps it so).

## 8. Test plan (Phase 2) — new `tests/invariants/src/remote_cross_browser_contract.rs`

Self-contained comment stripper (strings, template literals, regex literals kept), top-level function splitter
(brace depth outside literals) and a one-level CSS parser.

| Test | Rows | Asserts (red on `1b6d211` unless noted) |
|---|---|---|
| `test_rxb_helper_self_test` | — | lexer keeps `"//"`, regex `/^[0-9]{1,7}$/`, splits top-level functions; CSS parser reads `@supports` blocks |
| `test_rxb_s1_single_pure_capability_probe` | RXB1 | one `function browserCapabilities(`; returns the nine keys of §3.1; body has none of `fetch(`, `setText(`, `textContent`, `.register(`, `addEventListener`, `postJson(`, `new `, `localStorage`, `userAgent`, `console`; `PublicKeyCredential`/`PushManager` only inside it; `pushSupported` body = one `browserCapabilities()` + `.serviceWorker`, `.push`, `.notifications` |
| `test_rxb_s2_browser_apis_are_called_behind_the_probe` | RXB2 | for every top-level function except the probe, the guard (§0 D-2: `browserCapabilities().webauthn` for `navigator.credentials`; `pushSupported()` for `navigator.serviceWorker`, `.pushManager`, `Notification.`; `browserCapabilities().eventSource` for `new EventSource(`; `browserCapabilities().streams` for `.getReader(`; `browserCapabilities().imageBitmap` for `createImageBitmap(`) occurs before the first API token; no API token in top-level statements |
| `test_rxb_s3_passkey_controls_degrade_without_webauthn` | RXB3 | `PASSKEY_UNSUPPORTED_TEXT` exact, once; assignment rule of §3.3; the four handlers start with the guard before `window.confirm`/`postJson(`/`fetch(`; `showLogin`, `showApp`, `renderCamera`, `getAssertion`, `requestEnroll` use the text; `requestLock`, `fetchStatus`, `openStream` never mention `webauthn` |
| `test_rxb_s4_push_failures_map_to_distinct_fixed_texts` | RXB4 | the five new push texts exact, once, pairwise distinct and distinct from the four pinned; `PUSH_ERROR_TEXTS` exact; `unsupported_push_service` text names Edge; `pushUnsupportedText` logic; `enableNotifications` sets `deadline` from `SERVICE_WORKER_READY_TIMEOUT_MS` after the permission and before `readyRegistration`, wraps subscribe and post in `beforeDeadline(` with `PUSH_NO_SERVICE_TEXT`/`PUSH_TIMEOUT_TEXT`, unsubscribes on `unsupported_push_service`, displays via `failureText(error, PUSH_ERROR_TEXTS, PUSH_NO_SERVICE_TEXT)`; `beforeDeadline` clears its timer on both outcomes; `readyRegistration`/`disableNotifications` guarded |
| `test_rxb_s5_user_messages_are_fixed_strings` | RXB5 | `.message` only inside `failureText`; `fixedError` sets `fixed = true`; `PASSKEY_ERROR_TEXTS` exact; no `console` in `app.js`/`sw.js`; no template literal in `app.js`; every `setText(` argument is not an `error`/`err` expression |
| `test_rxb_s6_no_user_agent_logic` | RXB6 | guard (green on base): no `userAgent`, `userAgentData`, `navigator.vendor`, `navigator.platform`, `appVersion` in `app.js`/`sw.js`; no `user-agent`/`user_agent` (case-insensitive) in `crates/remote/src` production code |
| `test_rxb_s7_css_fallbacks` | RXB7 | every selector with `:has(` (outside the fallback block) maps to its fallback selector inside `@supports not selector(:has(*))` with identical declarations; no `:has(` inside; every `cqw`/`cqh` property has a non-container value earlier in the rule or a fallback for the same selector in `@supports not (container-type: size)`; the full-screen portrait fallback uses `100vh`/`100vw`; every `dvh`/`svh`/`lvh` value is preceded in its rule by a `vh` value; `render` sets `statTile.dataset.state`, `setLogoutHidden` toggles `feedback-last` |
| `test_rxb_s8_no_new_network_surface` | RXB8 | guard (green on base): pinned CSP once in `http.rs`; exactly 11 files in `crates/remote/assets`; no `http://`/`https://` in `app.js`/`sw.js`/`style.css`/manifest; `index.html` only the source link; no `importScripts`, `blob:`, `unsafe-inline`, `url(` |
| `test_rxb_s9_status_without_event_source` | RXB9 | `STATUS_POLL_MS = 15000`; `openStream` polls with `setInterval(` + `fetchStatus(true)` when `!browserCapabilities().eventSource` before `new EventSource(`; `closeStream` clears `statusPoll` |
| `test_rxb_s10_camera_degrades_without_streams` | RXB10 | `CAMERA_UNSUPPORTED_TEXT` exact, once; `renderCamera` hides `startButton` under `.streams`/`.imageBitmap`; `readCameraStream` and `showCameraFrame` guarded; parser bounds `CAMERA_MAX_PART_BYTES = 524288`, `CAMERA_MAX_BUFFER_BYTES = 1048576` unchanged; `openCameraStream`/`startCameraView` display through `failureText` |
| `test_rxb_s11_docs_adr_and_walkthrough` | RXB11 | `## 2i. Browser support` between 2h and 3 with the mandatory list, Edge desktop out of scope, Brave setting, private windows, iOS home screen, Baseline floor, every hardware row RXB12–RXB20 cited; troubleshooting mentions DNS-over-HTTPS and MagicDNS; ADR title; walkthrough 197 exists and names #366 |

Test hooks: none (static checks; the project has no JS runtime in CI). Behavioural evidence in browsers is the
owner hardware rows (and Playwright later, PR 2).

## 9. Amendments to existing tests

**None.** Every existing pin of V-1 … V-4, V-8, V-11 holds by construction (texts kept, call-site counts kept,
helpers of `remote_android_contract.rs` left private and untouched; the new module carries its own lexer).

## 10. Documentation

- `Docs/REMOTE_COMPANION.md` `## 2i. Browser support`: policy (mandatory/best effort, Baseline floor, feature
  detection only, no polyfill/CDN), matrix (page+SSE+lock, passkey, push, camera per browser; each cell is either
  verified by a cited row (RMC43, RMC44, RMC88 …) or "expected" with its pending hardware row RXB12–RXB20), the
  per-feature messages, iOS home-screen requirement (also from third-party iOS browsers, iOS 16.4+), Edge desktop
  push out of scope, Brave setting, private windows, browser secure DNS vs MagicDNS, owner hardware checks.
- §7 troubleshooting rows: DNS-over-HTTPS/secure DNS hides `*.ts.net`; each new push text.
- §8 residual: capability flags are client-side hints only; polling fallback; ESR without `:has()` best effort.
- §9: Edge desktop push pointer to §2i.

## 11. Verification matrix rows (component `remote-cross-browser`)

RXB1–RXB11 static (as §8); hardware (owner, pending): RXB12 iOS Safari tab and a third-party iOS browser show
`PUSH_HOME_SCREEN_TEXT`, the home-screen app subscribes; RXB13 Safari macOS; RXB14 Chrome desktop and Android;
RXB15 Firefox desktop (push, private window message, secure DNS note); RXB16 Firefox Android; RXB17 Samsung
Internet; RXB18 Brave (push setting message, then works when enabled); RXB19 Edge desktop (page, passkey, camera
work; push shows the Edge message, nothing stays subscribed); RXB20 a private window (Chrome incognito: blocked
text; Firefox private: service-worker text) while lock/status still work.

## 12. Assumptions (decided here)

- A-1: probe keys reduced to what per-feature messages need (no `platformAuthenticatorHint`, no `privateLikely`).
- A-2: *(withdrawn in round 2, PE F-2)* the button is disabled only once the permission is granted, so no path, the permission prompt included, keeps it disabled beyond the 10 s budget (§14 R-2).
- A-3: the non-container portrait fallback is exact for the card stage and approximate when `max-height: 70vh`
  caps it (box 4W/3 x 52.5vh instead of 70vh x W: no distortion, possible clipping); the full-screen `100vh`/`100vw`
  box ignores the `env(safe-area-inset-*)` padding and `100vh` is the large viewport on mobile (PE F-6). Engines
  without container queries are below the Baseline floor: best effort, listed in the Docs residuals.
- A-4: no User-Agent use at all (D-3) instead of "wording only".
- A-5: unsubscribing a refused (`unsupported_push_service`) subscription is best effort, never awaited.
- A-6: `sw.js` unchanged (issue design item 5).

## 13. Residual risks

- Feature detection cannot see a push service that is present but broken until `subscribe` answers (bounded 10 s).
- Firefox ESR 115 and other pre-Baseline engines get the fallbacks only where `@supports selector()` is understood.
- A browser without `EventSource` refreshes the Funnel web session idle timer every 15 s while the page is open
  (same as an open stream).
- No automated cross-engine run until PR 2 (Playwright); hardware rows RXB12–RXB20 carry the browser evidence.

## 14. Round 2 revisions (plan evaluator round 1, REVISION_REQUIRED; normative, supersede §3–§8 where they differ)

- **R-1 (F-1)** `style.css`: the rule `.card > .feedback:last-child, .card > .feedback:has(+ [hidden]:last-child)`
  is split into two rules with the same declaration (`margin-bottom: -16px`): a `:has()`-free
  `.card > .feedback:last-child` rule and a `:has()`-only rule. A selector list never mixes a `:has()` selector with
  another selector. RXB-S7 asserts: every rule containing a `:has(` selector contains only `:has(` selectors;
  every `:has(` selector maps (table `HAS_FALLBACKS`, §4) to a fallback in `@supports not selector(:has(*))`
  with identical declarations; `.card > .feedback:last-child` keeps `margin-bottom: -16px` in a rule outside the
  blocks.
- **R-2 (F-2)** `enableNotifications`: `pushEnableButton.disabled = true` and `const deadline = …` move to right
  after a `"granted"` answer (the first `await` stays `Notification.requestPermission(`; no comment containing the
  word "await" precedes it). The button is never disabled while the prompt is open, so every path keeps it disabled
  at most `SERVICE_WORKER_READY_TIMEOUT_MS` (+ timer slack). RXB-S4 asserts the order
  `requestPermission` < `"granted"` < `pushEnableButton.disabled = true` < `const deadline` < `await readyRegistration()`
  and that `pushEnableButton.disabled = true` occurs once in the function. A second tap during the prompt is harmless
  (the browser answers the same permission; at worst two identical subscribe calls, idempotent on the PC).
- **R-3 (F-3, F-5)** Push texts: new `PUSH_UNREACHABLE_TEXT = "The PC could not be reached to save the subscription:
  check the connection and try again"` and `PUSH_SLOW_TEXT = "The browser's push service did not answer in time: try
  again"`. The subscribe call is `beforeDeadline(…subscribe(…), deadline, PUSH_SLOW_TEXT)`; the PC hand-off is
  `beforeDeadline(postJson(…).catch(function () { throw fixedError(PUSH_UNREACHABLE_TEXT); }), deadline,
  PUSH_TIMEOUT_TEXT)`. `PUSH_NO_SERVICE_TEXT` is reached only through `PUSH_ERROR_TEXTS` (`AbortError`,
  `NotSupportedError` of `subscribe`). The `enableNotifications` fallback is the neutral `"Notifications not enabled"`
  (`failureText(error, PUSH_ERROR_TEXTS, "Notifications not enabled")`). There are now seven new push texts, all
  distinct (RXB-S4).
- **R-4 (F-4)** Test precision: RXB-S2 requires each guard to be a negated test inside an `if (` condition whose block
  contains `return`, `throw` or `Promise.reject` (the block of a combined condition such as
  `if (cameraDecoding || !browserCapabilities().imageBitmap)` counts); RXB-S5 parses every `setText(` call and
  asserts no argument contains `.message` and no argument is a bare `error`/`err`; every remaining
  `throw new Error(` is a `"status …"` throw (already in §8); `Math.max(0, deadline - Date.now())` pinned.
- **R-5 (F-7)** RXB-S6 and RXB-S8 are regression guards (green on `1b6d211` because the tree already complies); the
  red evidence for the "fail on the current tree" acceptance line is carried by RXB-S1–S5, S7, S9, S10, S11. Recorded
  in the tester contract and the matrix rows RXB6, RXB8.
- **R-6 (F-8)** `PASSKEY_ERROR_TEXTS` keeps three names (`NotAllowedError`, `SecurityError`, `NotSupportedError`);
  enrollment uses `const ENROLL_ERROR_TEXTS = Object.assign({}, PASSKEY_ERROR_TEXTS, { InvalidStateError:
  PASSKEY_REASONS.already_registered });` (`requestEnroll` → `failureText(error, ENROLL_ERROR_TEXTS, "Enrollment
  cancelled")`); `get()` ceremonies never show "already registered".
- **R-7 (F-6)** Approximations of the container-unit fallback named in A-3 and in `Docs/REMOTE_COMPANION.md` §8.
- **R-8 (round 2 N-1, N-2)** §14 supersedes the earlier prose (note at the top of §3). `ENROLL_ERROR_TEXTS` is declared
  after both `PASSKEY_ERROR_TEXTS` and `PASSKEY_REASONS` (`const` temporal dead zone); `PUSH_ERROR_TEXTS` after every
  push text it names.

Plan evaluator round 2: `VALIDATION_VERDICT: APPROVED` (`AI/plan_evaluator_report.md`).

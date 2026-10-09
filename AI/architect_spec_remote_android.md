# Architect Spec — GitHub #349: Android Support for the `soos-remote` Phone Companion and Web Push

- **Date**: 2026-10-09
- **Round**: 1, amended by the Phase 1.5 plan evaluator (2026-10-09; see `AI/plan_evaluator_report.md`, items PE-1..PE-6)
- **Branch**: `feat/remote-android`, created from `origin/main` at `24d0d8c` (#348 battery level merged on top of
  #347 live camera). GitHub-only issue: not registered in `scripts/sync_issue.py`, no `AI/BACKLOG.md` entry; the
  squash commit carries `Closes #349`. Nothing is installed, restarted or deployed by agents; no agent runs
  `tailscale`.
- **Owner decisions (2026-10-09, binding)**: (O-1) full Android parity, every acceptance line of #349; (O-2) the
  invariant-test assertions that pin the iPhone-only shape may be amended, each old value replaced by a new **exact**
  value, never loosened (§12 lists each one).
- **Inputs read**: issue #349; `origin/main:crates/remote/{assets/*,src/{assets,routes,server,push,webauthn,http}.rs}`,
  `crates/push-protocol/src/lib.rs`, `crates/push-sender/src/lib.rs`, `crates/remote/tests/{routes,push_server,
  webauthn}_tests.rs`, `crates/remote/tests/common/{push,passkey}.rs`,
  `tests/invariants/src/remote_{companion,passkey,push,brand,camera,battery}_contract.rs`, `AI/DECISIONS.md` (remote
  ADRs 2026-10-05 … 2026-10-07), `AI/ARCHITECTURE.md` §13, `AI/VERIFICATION_MATRIX.md` (RMC1–RMC88, RLC, RBS),
  `AI/architect_spec_remote_{web_push,brand}.md`, `Docs/REMOTE_COMPANION.md`, `scripts/install_remote.sh`.
- **Matrix**: new rows **RAN1–RAN12** (§13), RAN12 is the owner Android hardware check. Dedicated prefix `RAN`
  (remote Android), as #345 (`RLC`) and #346 (`RBS`) did; settled by the plan evaluator (PE-1), binding for every
  later phase (rows, test ids, walkthrough, tester contract).
- **Walkthrough**: **`AI/walkthroughs/193_remote_android.md`** (drift: the issue says 191, but 191 is
  `191_remote_live_camera.md` and 192 is `192_remote_battery_status.md` on `main`; see §15).
- **Test ids**: static invariants `test_ran_s1_*` … `test_ran_s10_*` in a new module
  `tests/invariants/src/remote_android_contract.rs`; Rust behaviour tests `test_ran_*` in new test files
  (§11). No existing test is weakened; §12 lists the owner-approved exact replacements.

---

## 0. Decisions

### 0.1 Facts verified in the code (not assumed)

| Fact | Where | Consequence |
|---|---|---|
| `PUSH_HOSTS` already allowlists `fcm.googleapis.com` and `updates.push.services.mozilla.com`; the path alphabet accepts `:` (FCM tokens) and `=` (Mozilla tokens) | `crates/push-protocol/src/lib.rs` L24, L120–L127 | No wire change; Android endpoints need tests only |
| `SubscribeBody` accepts `expirationTime` null or a number and refuses unknown members | `crates/remote/src/push.rs` L1121–L1130 | Chrome `toJSON()` shape parses; tests only |
| `parse_client_data` ignores unknown members (L3 §5.8.1), requires `crossOrigin` absent or `false` | `crates/remote/src/webauthn.rs` L95–L140 | Chrome's `other_keys_can_be_added_here` member verifies; tests only |
| `verify_registration` accepts BE=1/BS=1; `verify_assertion` accepts a counter that stays `0` on both sides | `webauthn.rs` L271–L310, L453 | Google Password Manager passkeys verify; tests only |
| `PushService::of` ends with `_ => Self::Mozilla` | `push.rs` L997–L1004 | Any future host would be mislabelled: made exhaustive (§2.1) |
| Push CSRF: exactly one `X-Soos-Action` equal to the route action; `Sec-Fetch-Site` absent or exactly `same-origin`; `Origin` absent or exactly `https://<host>[:443]` | `routes.rs` L310–L340 (`check_action_csrf`) | A same-origin service-worker `fetch` passes **unchanged** checks (§3.3) |
| Funnel: every non-public route needs a valid `__Host-soos_session` (`SameSite=Strict`) or answers `403 login_required` before routing | `server.rs` L1188–L1205, `websession.rs` L223 | The worker re-subscription works on Funnel only while the web session is valid; otherwise it is refused and the worker stays silent (§3.3) |
| CSP `img-src 'self'; connect-src 'self'; manifest-src 'self'`, `worker-src` falls back to `script-src 'self'` | `http.rs` L263–L266 | Same-origin PNG icons, badge and the worker POST are allowed; **CSP unchanged** |
| `PUSH_URGENCY = High`, `PUSH_TTL_S = 43_200` | `crates/remote/src/lib.rs` L399–L407 | FCM delivers high-urgency Web Push during Doze; no change |
| `enableNotifications` awaits `navigator.serviceWorker.ready` when `pushRegistration` is null | `app.js` L1078 | Hangs forever after a failed registration: fixed (§4.3) |
| `sameServerKey` returns `false` when `options.applicationServerKey` is absent, and `syncSubscription` then **unsubscribes** | `app.js` L934–L950, L1018–L1040 | A browser without `options.applicationServerKey` loses its subscription on every load: fixed (§4.2) |
| `png 0.18.1` is already in `Cargo.lock` (through `image`, used by `soos-gui`) | `Cargo.lock` | A dev-dependency `png = "=0.18.1"` adds no crate to the lock (§2.4) |

### 0.2 Spec-level decisions

- **D-1 Exhaustive host mapping lives in `soos-push-protocol`.** A new `PushHost` enum is the single source of the
  three hosts; `PUSH_HOSTS` is derived from it (value unchanged); `PushEndpoint` stores a `PushHost`, so
  `PushService::of` in `soos-remote` is a three-arm `match` with no fallback and a fourth host becomes a compile
  error in both crates.
- **D-2 Maskable background is `#0047BB` (brand blue), not `#EDF1FF`.** The star mark is a blue square tile whose
  only pale areas are two concave spikes reaching the tile edge midpoints. Rendered on blue, the blue parts of the tile
  merge with the background and the visible mark is the two pale spikes, whose farthest points are the edge
  midpoints. Scaling the 508-unit tile to 408 px puts those points 204 px from the centre, inside the maskable safe
  zone (circle of radius 40 % = 204.8 px of 512). Measured on the generated file: the farthest non-blue pixel centre
  is 203.5 px from the centre. On `#EDF1FF` the mark would have to keep its square outline whose corners
  (√2 · 204 ≈ 289 px from the centre) lie outside the safe zone, so circular and squircle launcher masks would cut
  the blue corner squares asymmetrically. Blue also equals `theme_color`, so the Android splash and launcher read
  as one surface.
- **D-3 Badge = alpha-only white silhouette of the two pale spikes.** Android draws the badge from its alpha channel
  only; the full tile would be a solid square. The badge is derived from `icon.svg` by mapping pale → opaque and
  blue → transparent, all RGB = white.
- **D-4 One network request in the worker, only for `pushsubscriptionchange`.** The `push` path keeps zero network
  access; the worker gains exactly one `fetch(`, a single bounded POST, never retried, result ignored.
- **D-5 No new server route, no server-check change.** The worker uses `/api/push/subscribe` with the exact page
  request shape. The server's CSRF, host, identity and Funnel-session gates are untouched; the worker succeeds
  only where the page itself would succeed.
- **D-6 Platform-neutral wording uses one passkey phrase**: "Face ID, fingerprint or screen lock". "Face ID" may
  appear in user-visible files only immediately followed by `, fingerprint or screen lock`.
- **D-7 Pixel properties are tested in `crates/remote/tests` with the `png` dev-dependency**; the zero-dependency
  invariants crate checks PNG structure only (signature, IHDR, chunk order, size bounds), like RMC84.

---

## 1. Scope & Blast Radius

### 1.1 Files

| File | Change |
|---|---|
| `crates/push-protocol/src/lib.rs` | `PushHost` enum, `PUSH_HOSTS` derived from it, `PushEndpoint.host: PushHost`, new `PushEndpoint::push_host()` (§2.1) |
| `crates/remote/src/push.rs` | `PushService::of` exhaustive over `PushHost`, made `pub` (§2.2) |
| `crates/remote/src/assets.rs` | four `AssetId` variants + `include_bytes!` arms (§2.3) |
| `crates/remote/src/routes.rs` | four `read_route` arms (§2.3) |
| `crates/remote/Cargo.toml` | `[dev-dependencies] png = "=0.18.1"` (§2.4) **only if the owner approves AM-7** (§12, PE-4); otherwise unchanged |
| `crates/remote/assets/sw.js` | `renotify`, `icon`, `badge`; `pushsubscriptionchange` handler (§3) |
| `crates/remote/assets/manifest.webmanifest` | `id`, three PNG icons (§5) |
| `crates/remote/assets/icon-192.png`, `icon-512.png`, `icon-maskable-512.png`, `badge-96.png` | **new**, generated by the §6 commands |
| `crates/remote/assets/app.js` | neutral texts, `PUSH_SERVICES` labels, key tri-state, bounded worker wait (§4) |
| `crates/remote/assets/index.html` | login button label only (§4.1) |
| `scripts/install_remote.sh` | neutral comments and echo lines (§7.2) |
| `Docs/REMOTE_COMPANION.md` | new `## 2h.` Android section, neutral text in §1, §2a, §2b, §2d, §2e, §4, §6, §7, §8 (§7.1) |
| `AI/DECISIONS.md` | new ADR (§14, text drafted there) |
| `AI/ARCHITECTURE.md` | §13 remote paragraph: "phone web app (iPhone home-screen app or Android Chrome, Samsung Internet, Firefox)", owner checks RAN12 |
| `AI/VERIFICATION_MATRIX.md` | new component block, rows RAN1–RAN12; RMC85/RMC86 text gains "(amended by RAN3/RAN5, ADR 2026-10-09)" |
| `tests/invariants/src/remote_android_contract.rs` + `lib.rs` `mod` line | new static contracts (§11.1) |
| `tests/invariants/src/remote_brand_contract.rs`, `remote_push_contract.rs` (and `remote_battery_contract.rs` only under AM-7) | owner-approved exact amendments only (§12) |
| `crates/push-protocol/tests/android_endpoint_tests.rs`, `crates/push-sender/tests/android_sender_tests.rs`, `crates/remote/tests/android_{push,webauthn,assets}_tests.rs` | new behaviour tests (§11.2) |
| `AI/walkthroughs/193_remote_android.md` | traceability phase |

### 1.2 Consumers of changed public items

- `soos_push_protocol::PUSH_HOSTS`: `crates/push-sender/src/lib.rs` L30, L148 (`filter_addresses`),
  `crates/remote/src/webpush.rs` L22, L424 (`JwtCache` bound), `crates/push-protocol/tests/protocol_tests.rs` L47 (pins
  the literal array; value unchanged, so the test stays green without edit).
- `PushEndpoint::host()`: `push.rs` L503 (`sub.endpoint.host()`), `push-protocol` tests L101–L102; signature and return
  value unchanged (`&'static str`).
- `PushService`: serialized as `"apple" | "google" | "mozilla"` in `GET /api/push` `devices[].service`; JSON unchanged;
  `push_server_tests.rs` L1824 keeps passing.
- `AssetId`: matched only in `assets.rs` (`asset`) and produced only in `routes.rs`; `server.rs` L1231 serves any
  `Route::Asset(id)` generically; `is_funnel_public(Route::Asset(_))` already covers new variants.
  `routes_tests.rs` L125 enumerates a fixed list (not exhaustive), so it compiles and passes unchanged.
- No daemon, PAM, GUI, enrollment-cli or protocol (`soos-protocol`) change. No new binary, unit or packaging entry
  (assets are `include_bytes!`).

### 1.3 Out of scope (issue #349)

Native Android app, FCM direct API keys, Edge desktop (`*.notify.windows.com`), Android Shortcuts/Tasker integration
(documented only as a pointer in §2h), any new server route, any change of the push wire, VAPID, encryption, store,
rate limits, CSP or CSRF rules.

---

## 2. Rust Types & Signatures

### 2.1 `crates/push-protocol/src/lib.rs`

```rust
/// One allowlisted push service host (single source of truth of `PUSH_HOSTS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PushHost {
    /// `web.push.apple.com` (Safari, iOS / iPadOS 16.4+ home-screen apps).
    Apple,
    /// `fcm.googleapis.com` (Chrome, Samsung Internet and other Chromium browsers, Android included).
    Google,
    /// `updates.push.services.mozilla.com` (Firefox, Android included).
    Mozilla,
}

impl PushHost {
    /// Every host, in the `PUSH_HOSTS` order.
    pub const ALL: [PushHost; 3] = [PushHost::Apple, PushHost::Google, PushHost::Mozilla];

    /// The exact DNS name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            PushHost::Apple => "web.push.apple.com",
            PushHost::Google => "fcm.googleapis.com",
            PushHost::Mozilla => "updates.push.services.mozilla.com",
        }
    }
}

/// Unchanged value; now derived.
pub const PUSH_HOSTS: [&str; 3] = [
    PushHost::Apple.name(),
    PushHost::Google.name(),
    PushHost::Mozilla.name(),
];

pub struct PushEndpoint {
    raw: String,
    host: PushHost, // was `&'static str`
}

impl PushEndpoint {
    // parse(): `PushHost::ALL.iter().copied().find(|h| h.name().as_bytes() == authority.as_bytes())
    //            .ok_or(EndpointError::HostNotAllowed)?` — same rule, same error, same order.

    /// The allowlisted host (a `PUSH_HOSTS` element). Signature unchanged.
    #[must_use]
    pub fn host(&self) -> &'static str { self.host.name() }

    /// The allowlisted host as an enum (exhaustive matching for consumers).
    #[must_use]
    pub fn push_host(&self) -> PushHost { self.host }
}
```

Bounds: unchanged (`MAX_PUSH_ENDPOINT_BYTES = 1024`, path alphabet, no port/userinfo/dot segments). `Debug` of
`PushEndpoint` stays redacted. `#![forbid(unsafe_code)]` unchanged. No new dependency.

### 2.2 `crates/remote/src/push.rs`

```rust
impl PushService {
    /// The service of a validated endpoint; exhaustive, each variant from its own host only.
    #[must_use]
    pub fn of(endpoint: &PushEndpoint) -> Self {
        match endpoint.push_host() {
            PushHost::Apple => Self::Apple,
            PushHost::Google => Self::Google,
            PushHost::Mozilla => Self::Mozilla,
        }
    }
}
```

No `_` arm and no string match. Doc comments of the variants gain the browser families of §2.1. Visibility `pub`
(was private) so `test_ran_push_service_maps_each_host` calls it directly. JSON and `serde` unchanged.

### 2.3 Assets and routes

`crates/remote/src/assets.rs` gains, after `ServiceWorker` (doc comments name the path):

| Variant | Path | `content_type` | Source |
|---|---|---|---|
| `Icon192` | `/icon-192.png` | `image/png` | `include_bytes!("../assets/icon-192.png")` |
| `Icon512` | `/icon-512.png` | `image/png` | `include_bytes!("../assets/icon-512.png")` |
| `IconMaskable512` | `/icon-maskable-512.png` | `image/png` | `include_bytes!("../assets/icon-maskable-512.png")` |
| `Badge96` | `/badge-96.png` | `image/png` | `include_bytes!("../assets/badge-96.png")` |

`crates/remote/src/routes.rs` `route()` `read_route` gains the four exact paths (string literals, like
`"/apple-touch-icon.png"`). Semantics inherited, no new code path: `GET`/`HEAD` → `Route::Asset(id)`; `POST`/other →
`405` with `Allow: GET, HEAD`; query string ignored; `/icon-192.png/` and case variants → `404`; Funnel-public through
`is_funnel_public(Route::Asset(_))` (an anonymous Funnel caller may fetch icons, as it may fetch the page; needed
because Chrome fetches manifest icons without credentials). Every response keeps `MANDATORY_HEADERS` (exact CSP,
`nosniff`, `no-store`, `Referrer-Policy: no-referrer`).

### 2.4 `crates/remote/Cargo.toml`

`[dev-dependencies]` gains `png = "=0.18.1"` (exact pin to the locked version: no crate added to `Cargo.lock`, license
MIT OR Apache-2.0 already accepted by `deny.toml`, no duplicate version). `[dependencies]` unchanged, so
the `[dependencies]` half of `test_rbs_s6_no_new_dependency` and RMC-S4 (`soos-*` keys) stay green. **Correction
(PE-4)**: the second half of `test_rbs_s6_no_new_dependency` (`tests/invariants/src/remote_battery_contract.rs`
L519–L526) asserts that every `[dev-dependencies]` key of `crates/remote` is one of `tokio`, `tempfile`, `proptest`;
adding `png` turns it red. The dev-dependency therefore requires owner amendment **AM-7** (§12); without it, the
fallback of §12 applies and `crates/remote/Cargo.toml` is not changed. The Phase 3 auditor confirms with
`cargo deny check` and `git diff Cargo.lock` (expected: only the `soos-remote` entry's dependency list changes).

### 2.5 Error taxonomy

No new error type or variant. `EndpointError::HostNotAllowed` keeps its meaning and position in the rule order. No
change on the PAM or daemon side; this issue does not touch the authentication path (no `Verdict`, no `PAM_*` result).

### 2.6 Latency budget

Not applicable: no change on PAM → daemon → camera → vision. Client-side bounds are in §3.2 and §4.3.

---

## 3. Service Worker (`crates/remote/assets/sw.js`)

### 3.1 Notification options (acceptance line 1)

```js
const ICON_PATH = "/icon-192.png";
const BADGE_PATH = "/badge-96.png";

self.addEventListener("push", (event) => {
  const shown = readNotification(event);
  event.waitUntil(
    self.registration.showNotification(shown.title, {
      body: shown.body,
      tag: shown.tag,
      renotify: true,
      icon: ICON_PATH,
      badge: BADGE_PATH,
      lang: "en",
    })
  );
});
```

- `readNotification` is unchanged (fallback title/body, `soos-alerts` / `soos-test` tags). `renotify: true` is valid
  because the tag is never empty. Safari ignores `renotify`, `icon` and `badge`; behaviour on iOS is unchanged.
- The `push` handler and `readNotification` contain no `fetch(`, no `caches`, no storage; every push still calls
  `showNotification` inside `waitUntil`.
- (PE-3) Precise wording: the **worker code** issues no request on `push`. The **browser** loads the two same-origin
  images `/icon-192.png` and `/badge-96.png` when it displays the notification (Funnel-public assets, no credentials
  needed, no identity or alert data in the URL). A failed image load (phone off-tailnet, PC down) does not suppress the
  notification in Chrome, Samsung Internet or Firefox: it is shown without the image. Documents and the ADR must say
  "the worker makes no request on push", never "the push path makes no network access". RAN12 step (5b) checks it.
- The header comment is rewritten: "It shows notifications and, on `pushsubscriptionchange` only, re-sends the renewed
  subscription once to `/api/push/subscribe`" (no "It only shows notifications" claim left).

### 3.2 `pushsubscriptionchange` (acceptance line 2)

```js
const SUBSCRIBE_PATH = "/api/push/subscribe";
const SUBSCRIBE_ACTION = "push-subscribe";
// One attempt, bounded: no retry, no timer left behind.
const RESUBSCRIBE_TIMEOUT_MS = 10000;

// The renewed subscription: the browser's own, else one made with the old key; null when the
// old key is unknown (the page re-subscribes on its next "Enable notifications").
function renewedSubscription(event) {
  if (event.newSubscription) {
    return Promise.resolve(event.newSubscription);
  }
  const old = event.oldSubscription;
  const key = old && old.options ? old.options.applicationServerKey : null;
  if (!key) {
    return Promise.resolve(null);
  }
  return self.registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });
}

// The exact request shape of the page's postJson(PUSH_SUBSCRIBE_PATH, "push-subscribe", ...).
function postSubscription(subscription) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), RESUBSCRIBE_TIMEOUT_MS);
  return fetch(SUBSCRIBE_PATH, {
    method: "POST",
    headers: { "X-Soos-Action": SUBSCRIBE_ACTION, "Content-Type": "application/json" },
    body: JSON.stringify(subscription.toJSON()),
    credentials: "same-origin",
    cache: "no-store",
    signal: controller.signal,
  }).finally(() => clearTimeout(timer));
}

self.addEventListener("pushsubscriptionchange", (event) => {
  event.waitUntil(
    renewedSubscription(event)
      .then((subscription) => (subscription === null ? null : postSubscription(subscription)))
      .catch(() => null)
  );
});
```

Rules (each tested statically, §11.1 `test_ran_s2`):

1. **Same key**: only the browser's `newSubscription` or the old subscription's `applicationServerKey` is used; the
   worker never fetches `/api/push` for a key and never hard-codes one.
2. **Same request shape as the page** (`postJson`, `app.js` L213–L230): `POST`, header `X-Soos-Action: push-subscribe`,
   `Content-Type: application/json`, body `subscription.toJSON()`, `cache: "no-store"`; `credentials: "same-origin"`
   is the page's implicit default, written out explicitly. No other header is set.
3. **Silent and bounded**: exactly one `fetch(` in the whole file, in `postSubscription`; the response is not read, no
   status is inspected, no notification is shown, nothing is stored; the outer `.catch(() => null)` swallows every
   failure (subscribe refusal, abort, network error, `4xx`/`5xx` are a resolved fetch and are ignored). No loop,
   `setInterval`, recursion, re-dispatch or second attempt. `RESUBSCRIBE_TIMEOUT_MS = 10000` aborts a hung request.
4. **Sentinels**: `newSubscription` null and `oldSubscription` null, or no `options`, or a null key → no request at all.
   `pushManager.subscribe` rejecting (permission revoked) → silent.
5. The old endpoint is **not** unsubscribed by the worker (a second request would hit the shared 1 s route gate,
   `PUSH_ROUTE_MIN_INTERVAL_MS`); it is removed by the existing delivery path on the push service's `404`/`410`
   (`LastDelivery::Gone`). Until then it occupies one of `MAX_PUSH_SUBSCRIPTIONS = 4` slots; with four stored the
   POST gets `409 too_many_subscriptions` and the worker stays silent (documented in §2h and §8).

### 3.3 What the server receives from the worker (no check loosened)

A `fetch` issued by the service worker of `https://<host>` to its own origin carries, per Fetch and Service Worker
specs and as shipped by Chrome, Samsung Internet, Firefox and Safari:

| Header | Value | Server check (unchanged) | Result |
|---|---|---|---|
| `X-Soos-Action` | `push-subscribe` (set by the worker) | exactly one, equal to the route action | pass |
| `Origin` | `https://<host>` (always sent on `POST`) | absent or `https://<normalized host>[:443]` | pass |
| `Sec-Fetch-Site` | `same-origin` (initiator = the worker's origin) | absent or exactly `same-origin` | pass |
| `Host` | `<host>` (forwarded by `tailscale serve`/`funnel`) | `check_host` against `allowed_hosts` | pass as for the page |
| Tailscale identity headers | added by `tailscaled` per source node (tailnet path) | `classify_request` allowlist | pass as for the page |
| `Cookie: __Host-soos_session` | sent with `credentials: "same-origin"`; the worker's site for cookies is its own origin, so `SameSite=Strict` does not strip it | Funnel: valid session required before routing | pass only while the session is valid |

**Explicit statement (D-5):** the handler re-subscribes and posts with the page's exact request shape; the server
accepts it only where it accepts the same request from the page. On the tailnet that is whenever the phone is on the
tailnet with an allowlisted login; over Funnel only while the web session (15 min idle, 8 h at most) is valid. In every
other case (no session → `403 login_required`, off-tailnet, sender down, rate gate → `429`, store full → `409`) the
request is refused by the **unchanged** server checks and the worker stays silent; the subscription is re-sent by the
page on its next load (`syncSubscription`, §4.2) or by the owner's next "Enable notifications". No server check, CSRF
rule, gate or route is modified, and the behaviour tests of §11.2 prove both outcomes against the real server.

(PE-6) Side effect accepted: over Funnel, a worker POST that carries a still-valid session refreshes its idle timer
(`Touch::Refresh` in `server.rs`, as any page request does) without the owner looking at the page; the 8 h absolute
lifetime is unchanged and the request is at most one per browser-initiated `pushsubscriptionchange`. Stated in the ADR
accepted risks and §2h item 8.

---

## 4. Page Script (`crates/remote/assets/app.js`) and Markup

### 4.1 Platform-neutral texts (acceptance line 5)

Exact constants (each defined once, used everywhere the old literal was):

```js
// Every passkey ceremony (login, enroll, unlock, camera view).
const PASSKEY_PROMPT_TEXT = "Confirm with your passkey (Face ID, fingerprint or screen lock)…";
const PUSH_UNSUPPORTED_TEXT =
  "On iPhone, notifications need the home-screen app (iOS 16.4 or later); this browser does not support them";
const PUSH_BLOCKED_TEXT = "Notifications are blocked: allow them for this site in the browser or phone settings";
const PUSH_WORKER_FAILED_TEXT =
  "Notifications could not start: the service worker did not register (reload the page and try again)";
```

| Location (`origin/main` line) | Old | New |
|---|---|---|
| `PASSKEY_REASONS.passkey_required` (L117) | `"Face ID is required for every unlock"` | `"A passkey check (Face ID, fingerprint or screen lock) is required for every unlock"` |
| `PASSKEY_REASONS.registration_conflict` (L126–L127) | `… from the phone's Passwords app` | `… from the phone's password manager` |
| `requestUnlock` (L751), `requestLogin` (L802), `requestEnroll` (L843), camera start (L1630) | `"Confirm with Face ID…"` | `PASSKEY_PROMPT_TEXT` |
| `renderPush` (L964), `enableNotifications` (L1063) | `"Notifications need the home-screen app (iOS 16.4 or later)"` | `PUSH_UNSUPPORTED_TEXT` |
| `enableNotifications` (L1075) | `"Notifications are blocked in the iPhone settings"` | `PUSH_BLOCKED_TEXT` |
| `index.html` L44 `#login-button` | `Sign in with Face ID` | `Sign in with your passkey` |
| Comments L11, L22, L29, L38, L721, L746, L892, L1060, L1625 | "Face ID", "Face ID / Touch ID", "home-screen app, iOS 16.4 or later", "Safari requires a user gesture" | neutral: "the passkey check (Face ID, fingerprint or screen lock)", "push notifications (iPhone home-screen app or a supporting browser)", "browsers require a user gesture" |

Rule (tested, `test_ran_s6`): in `app.js` and `index.html`, every occurrence of `Face ID` is immediately followed by
`, fingerprint or screen lock`; `Touch ID`, `iPhone settings` and `Passwords app` do not occur. The comment at L1292
("iPhone Safari has no element fullscreen") is a technical fact about the overlay fallback and may keep "iPhone".

### 4.2 Subscription key tri-state (acceptance line 6, first half)

`sameServerKey(subscription, publicKey) -> boolean` is replaced by:

```js
// "same" | "different" | "unknown": "unknown" when the browser does not expose
// options.applicationServerKey (or the PC key is missing); such a subscription is kept.
function serverKeyMatch(subscription, publicKey) { … }
```

- `subscription.options` absent, or `options.applicationServerKey` null/undefined, or `publicKey` not a string →
  `"unknown"`. Byte lengths differ or any byte differs → `"different"`. Equal → `"same"`.
- `syncSubscription`: `"same"` or `"unknown"` → `postJson(PUSH_SUBSCRIBE_PATH, "push-subscribe", subscription.toJSON())`
  (keeps the PC in sync); only `"different"` → `unsubscribe()` and "Notifications must be re-enabled on this phone"
  (text unchanged, pinned by RMC-S38). No other caller.
- Accepted consequence: a subscription of unknown key made before a `soos-remote push reset` is kept and its next
  delivery is refused by the push service (`last_delivery = "rejected"`, existing UI hint); the owner re-enables.

### 4.3 Bounded wait for the worker (acceptance line 6, second half)

```js
// Longest wait for an active service worker before subscribing.
const SERVICE_WORKER_READY_TIMEOUT_MS = 10000;

// The registration with an active worker, or null when registration failed or the
// worker did not activate within SERVICE_WORKER_READY_TIMEOUT_MS.
function readyRegistration() {
  return registerServiceWorker().then(function (registration) {
    if (registration === null) {
      return null;
    }
    if (registration.active) {
      return registration;
    }
    return new Promise(function (resolve) {
      const timer = setTimeout(function () { resolve(null); }, SERVICE_WORKER_READY_TIMEOUT_MS);
      navigator.serviceWorker.ready.then(function (ready) {
        clearTimeout(timer);
        resolve(ready);
      });
    });
  });
}
```

`enableNotifications` keeps `Notification.requestPermission()` as its **first** `await` (RMC-S38, user gesture), then:

```js
const registration = await readyRegistration();
if (registration === null) {
  setText(pushFeedback, PUSH_WORKER_FAILED_TEXT);
  return;
}
```

The `finally` block (re-enables the button, `fetchPush()`) is unchanged, so the button never stays disabled.
`navigator.serviceWorker.ready` occurs in `app.js` only inside `readyRegistration`, next to the `setTimeout`.
`registerServiceWorker` is unchanged (a failed registration is retried on the next call, i.e. the next tap: bounded
by user action). `app.js` still contains exactly one `.register(` (RMC passkey contract).

### 4.4 Device labels (acceptance line 7)

```js
const PUSH_SERVICES = {
  apple: "Apple",
  google: "Android / Chrome (Google)",
  mozilla: "Firefox (Mozilla)",
};
```

The list item text stays `<label> + " device, enabled " + <time>` through `textContent`.

---

## 5. Manifest (`crates/remote/assets/manifest.webmanifest`, acceptance line 3)

Exact file (key order, two-space indent, `"key": "value"` with one space, as today so
`test_rmc_s8_app_js_is_textcontent_only_and_carries_the_ui_constants` keeps matching):

```json
{
  "id": "/",
  "name": "soos remote",
  "short_name": "soos",
  "description": "Lock status and remote lock of the soos desktop over the owner's tailnet",
  "display": "standalone",
  "start_url": "/",
  "scope": "/",
  "background_color": "#EDF1FF",
  "theme_color": "#0047BB",
  "icons": [
    { "src": "icon.svg", "sizes": "any", "type": "image/svg+xml" },
    { "src": "apple-touch-icon.png", "sizes": "180x180", "type": "image/png" },
    { "src": "icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any" },
    { "src": "icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any" },
    { "src": "icon-maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
  ]
}
```

`id` `/` is stable (equals `start_url`), so a later `start_url` change never creates a second installed app. Chrome's
installability (HTTPS, `name`/`short_name`, `start_url`, `display: standalone`, 192 and 512 PNG icons) is met; Chrome
no longer requires a `fetch` handler, so the worker keeps none. All `src` values are relative same-origin (no
`http://`/`https://`). `index.html` is unchanged except the login label (the iOS `apple-touch-icon` link and
`apple-mobile-web-app-*` metas stay).

---

## 6. Icon Generation (deterministic, `Docs/REMOTE_COMPANION.md` §2e)

Run from the repository root on a developer machine (verified with rsvg-convert 2.63.2 and ImageMagick 7.1.2-32; two
consecutive runs give byte-identical files). Same flags as the existing `apple-touch-icon.png` command: `-strip`,
`png:exclude-chunks=date,time,tIME`, fixed color type, so the files carry only `IHDR`, `IDAT`, `IEND`.

```bash
A=crates/remote/assets
# Full-bleed "any" icons (opaque RGB), like apple-touch-icon.png.
for n in 192 512; do
  rsvg-convert -w "$n" -h "$n" "$A/icon.svg" \
    | magick png:- -background '#EDF1FF' -alpha remove -alpha off -strip \
        -define png:color-type=2 -define png:exclude-chunks=date,time,tIME \
        "png:$A/icon-$n.png"
done
# Maskable: the 508-unit mark at 408 px, centred on the brand blue (spike tips 204 px from the
# centre, inside the 204.8 px safe-zone circle of a 512 px icon).
rsvg-convert -w 408 -h 408 "$A/icon.svg" \
  | magick -size 512x512 'xc:#0047BB' png:- -gravity center -composite -alpha off -strip \
      -define png:color-type=2 -define png:exclude-chunks=date,time,tIME \
      "png:$A/icon-maskable-512.png"
# Badge: alpha-only white silhouette of the pale star spikes (blue -> transparent, pale -> opaque).
rsvg-convert -w 96 -h 96 "$A/icon.svg" \
  | magick png:- -alpha off -colorspace Gray -level 30%,90% -background white -alpha shape -strip \
      -define png:color-type=6 -define png:exclude-chunks=date,time,tIME \
      "png:$A/badge-96.png"
```

Measured results (architect dry run in the scratchpad, files not committed):

| File | IHDR | Bytes | Pixel facts |
|---|---|---|---|
| `icon-192.png` | 192×192, depth 8, color type 2 | 2 885 | corners `#0047BB` |
| `icon-512.png` | 512×512, depth 8, color type 2 | 8 121 | corners `#0047BB` |
| `icon-maskable-512.png` | 512×512, depth 8, color type 2 | 6 926 | every pixel whose centre is > 204.8 px from (256, 256) is exactly `#0047BB` (max non-blue radius 203.5); (266, 246) and (246, 266) are `#EDF1FF` |
| `badge-96.png` | 96×96, depth 8, color type 6 | 1 098 | every pixel with alpha > 0 is RGB `#FFFFFF`; opaque (alpha ≥ 128) fraction 10.76 %; alpha 0 at the four corners, (24, 24) and (72, 72); alpha 255 at (52, 44) and (44, 52) |

The `-level 30%,90%` window maps the gray of `#0047BB` (25.2 %) to 0 and of `#EDF1FF` (94.6 %) to 1. Bounds tested
(§11): `icon-192.png` ≤ 8 KiB, `icon-512.png` ≤ 16 KiB, `icon-maskable-512.png` ≤ 16 KiB, `badge-96.png` ≤ 4 KiB.

---

## 7. Documentation and Installer Text

### 7.1 `Docs/REMOTE_COMPANION.md`

- **New `## 2h. Android phones (Chrome, Samsung Internet, Firefox)`**, placed after `## 2g.` and before `## 3.` (the
  `## 2d.`, `## 2e.`, `## 2f.`, `## 2g.`, `## 9.` sections keep their headings; RMC-S39, RMC-S55, RLC-S8 and the battery
  docs test keep passing). Content (each item a short paragraph or list):
  1. **Tailnet access**: install the Tailscale app (Google Play or F-Droid), sign in to the same tailnet with an
     allowlisted login, keep the VPN on for tailnet use; Funnel access (§2b) needs no Tailscale app.
  2. **Install the web app**: open `https://<pc>.<tailnet>.ts.net` in Chrome → menu → **Install app** (or **Add to Home
     screen**); Samsung Internet → menu → **Add page to** → **Home screen**; the maskable icon is used by the launcher.
     If Chrome only offers a shortcut, it still opens the page standalone. Push works in a normal Chrome tab too
     (unlike iPhone, no installation is needed for notifications).
  3. **Notification permission**: tap **Enable notifications**; on Android 13 or later the system also asks whether
     Chrome may post notifications (if refused: Settings → Apps → Chrome → Notifications). Site permission: Chrome →
     site settings → Notifications.
  4. **Passkey**: the passkey is stored by **Google Password Manager** (or Samsung Pass) and unlocked with
     **fingerprint or screen lock**; a screen lock is required. It is synced to the Google account (BE/BS flags set),
     the same accepted risk as iCloud Keychain in §8; enroll over the tailnet with `soos-remote enroll-code` as in §2b.
  5. **Battery optimisation**: pushes use `Urgency: high` and reach a phone in Doze, but vendor battery savers can
     still delay or drop them: set Chrome (and Firefox if used) to **Unrestricted** battery usage (Samsung: "Never
     sleeping apps").
  6. **Push services**: Chrome and Samsung Internet use `fcm.googleapis.com` (device list label "Android / Chrome
     (Google)"); Firefox uses `updates.push.services.mozilla.com` ("Firefox (Mozilla)"); both are in the existing
     allowlist; Google or Mozilla see delivery metadata, never the content (§2d).
  7. **Firefox caveats**: home-screen install opens like a browser shortcut on some versions; passkey support depends
     on the Firefox and Android versions (if no passkey prompt appears, use Chrome); Firefox must not be battery
     restricted for its push connection; `pushsubscriptionchange` is fired by Firefox, the page re-sends the
     subscription on every open anyway.
  8. **Subscription renewal**: when the browser renews a push subscription, the worker re-sends it once to the PC with
     the page's request shape; it fails silently when the PC refuses (off-tailnet, expired Funnel session, four devices
     already registered); opening the app re-sends it.
  9. **Shortcuts/Tasker**: not integrated; pointer only — the `POST /api/lock` call of §4 can be issued by any HTTP
     automation app over the tailnet with the same `X-Soos-Action: lock` header (unlock still needs the page).
  10. **Owner hardware check (matrix row RAN12)**: the steps of §13 RAN12.
- **Neutral text elsewhere** (the `## 4.` subsection `### iPhone home screen and Shortcuts` keeps its iPhone content;
  each other iOS-only phrase that describes the general product is generalised): §1 intro "passkey (Face ID,
  fingerprint or screen lock)"; §2a/§2b "Face ID / Touch ID" → "user verification (Face ID, fingerprint or screen
  lock)", "the iPhone's passkey" → "the phone's passkey", "Safari or the home-screen app" → "the browser or the
  installed web app"; §2d first paragraph: "the home-screen web app on iPhone (iOS/iPadOS 16.4 or later; Safari tabs
  cannot receive push) or a supporting browser on Android (Chrome, Samsung Internet, Firefox; see §2h)", keeping the
  strings `home-screen`, `16.4`, `Show Previews` and `web.push.apple.com` (RMC-S39), and naming `fcm.googleapis.com`
  and `updates.push.services.mozilla.com`; §2e "Icons" paragraph: the §6 command block, the four new files, the
  maskable safe-zone and badge rationale (D-2, D-3) and "`id`"; §4 step 4 "add it to the home screen (iPhone: Safari →
  Share → Add to Home Screen; Android: §2h)"; §6 asset row lists the four new paths; §7 troubleshooting gains rows
  "Android: no notification while the screen is off → battery optimisation (§2h)" and "Notifications could not start:
  the service worker did not register → reload; check `/sw.js` answers 200"; §8 accepted risks name Google Password
  Manager next to iCloud Keychain; §9 out of scope gains one bullet "A native Android app, FCM direct API keys, Edge
  desktop push (`*.notify.windows.com`), Android Shortcuts/Tasker integration (pointer in §2h only)" while keeping
  "push hosts other than Apple, Google and Mozilla" and the live-camera bullet (RMC-S39) (PE-5). §2f keeps the substring `Face ID` (RLC-S8), written as "Face ID, fingerprint or
  screen lock".

### 7.2 `scripts/install_remote.sh`

| Line (`main`) | Old | New |
|---|---|---|
| 91 | `# Passkeys (Face ID / Touch ID) and internet access through Tailscale Funnel` | `# Passkeys (Face ID, fingerprint or screen lock) and internet access through Tailscale Funnel` |
| 107 | `… to the home-screen app (section 2d).` | `… to the phone web app (section 2d; Android: section 2h).` |
| 112 | `# What the iPhone lock screen shows: …` | `# What the phone lock screen shows: …` |
| 115 | `… Needs rp_id, a fresh Face ID per view,` | `… Needs rp_id, a fresh passkey check per view,` |
| 148 | `run soos-remote enroll-code and register Face ID from the phone over the tailnet,` | `run soos-remote enroll-code and register the phone's passkey over the tailnet,` |
| 154 | `open the home-screen app and tap Enable notifications.` | `open the web app on the phone (the home-screen app on iPhone) and tap Enable notifications.` |

Template keys, `echo`-only rule, no `sudo`/`/etc` writes (RMC-S6, RLC-S9) unchanged. Rule tested (`test_ran_s10`):
no `iPhone lock screen`, no `register Face ID`, no `Touch ID`; every `Face ID` followed by `, fingerprint or screen
lock`.

---

## 8. Invariants Touched

| Invariant | Effect |
|---|---|
| RC-1 (`soos-remote` opens no network socket) | Unchanged: the worker's POST is a browser request to the same origin; the only outbound component stays `soos-push-sender` |
| RC-2 (allowlisted identity or Funnel session) | Unchanged; new assets are Funnel-public like every asset |
| RC-4 (every read/write/count bounded) | Unchanged server side; client side: one worker POST bounded by `RESUBSCRIBE_TIMEOUT_MS`, page wait bounded by `SERVICE_WORKER_READY_TIMEOUT_MS` |
| RC-5 (no identity, exact view shapes) | `GET /api/push` JSON unchanged (`service` values `apple`/`google`/`mozilla`) |
| Web Push ADR item (4) SSRF allowlist | Unchanged hosts and rules; enum is a refactor of the same list (D-1) |
| Web Push ADR "worker has no `fetch` handler and stores nothing" | Kept; amended to allow exactly one same-origin POST in `pushsubscriptionchange` (§14) |
| Brand ADR "seven-file asset set", RMC84–RMC86 | Amended: eleven files, five manifest icons (§12, §14) |
| CSP exact string (`http_tests.rs` L25, `server_tests.rs` L94) | Unchanged |
| RMC-S38 first `await` is `Notification.requestPermission()` | Kept |
| No password, embedding, frame, credential in logs/IPC | Unchanged; no new log line |

---

## 9. Constants & Config

| Name | Location | Value | Sentinel semantics |
|---|---|---|---|
| `PushHost::ALL` | `push-protocol/src/lib.rs` | `[Apple, Google, Mozilla]` | order = `PUSH_HOSTS` order |
| `PUSH_HOSTS` | same | unchanged literal value, derived from `PushHost::name` | — |
| `ICON_PATH` | `sw.js` | `"/icon-192.png"` | — |
| `BADGE_PATH` | `sw.js` | `"/badge-96.png"` | — |
| `SUBSCRIBE_PATH` | `sw.js` | `"/api/push/subscribe"` (equals `PUSH_SUBSCRIBE_PATH` of `routes.rs` and `app.js`; tested equal) | — |
| `SUBSCRIBE_ACTION` | `sw.js` | `"push-subscribe"` (equals `ACTION_PUSH_SUBSCRIBE` of `crates/remote/src/lib.rs`; tested equal) | — |
| `RESUBSCRIBE_TIMEOUT_MS` | `sw.js` | `10000` | one attempt; abort → silent |
| `SERVICE_WORKER_READY_TIMEOUT_MS` | `app.js` | `10000` | elapsed → `null` → `PUSH_WORKER_FAILED_TEXT` |
| `PASSKEY_PROMPT_TEXT`, `PUSH_UNSUPPORTED_TEXT`, `PUSH_BLOCKED_TEXT`, `PUSH_WORKER_FAILED_TEXT` | `app.js` | §4.1 exact strings | — |
| Icon byte bounds | tests | 8 / 16 / 16 / 4 KiB | over → test failure |

No `remote.toml` key is added or changed. JS duplicates of server constants (`SUBSCRIBE_PATH`, `SUBSCRIBE_ACTION`) are
unavoidable across the language boundary and are pinned equal to the Rust constants by `test_ran_s2` (one source of
truth enforced by test).

---

## 10. Acceptance-Line Map (#349)

| # | Acceptance line | Spec | Tests | Row |
|---|---|---|---|---|
| A1 | `renotify`, raster `icon`, monochrome `badge`, no fetch/cache in push path, every push shows | §3.1, §6 | `test_ran_s1`, `test_ran_badge_is_white_on_transparent` | RAN1 |
| A2 | `pushsubscriptionchange` same key, same CSRF + credentials, silent, bounded | §3.2, §3.3 | `test_ran_s2`, `test_ran_worker_resubscribe_shape_is_accepted`, `test_ran_worker_resubscribe_without_funnel_session_is_refused` | RAN2 |
| A3 | Manifest installable: `id`, 192/512 `any`, 512 `maskable`, deterministic, touch icon kept | §5, §6 | `test_ran_s3`, `test_ran_s4`, `test_ran_maskable_icon_keeps_the_mark_in_the_safe_zone`, `test_ran_any_icons_are_full_bleed_brand_renders` | RAN3, RAN4 |
| A4 | New assets embedded, `image/png`, GET/HEAD only, CSP unchanged | §2.3 | `test_ran_s5`, `test_ran_assets_are_routed_as_png`, `test_ran_assets_are_served_with_the_unchanged_csp` | RAN5 |
| A5 | Platform-neutral text | §4.1, §7 | `test_ran_s6`, `test_ran_s10`, amended `test_rmc_s50`, amended `test_rmc_s38` | RAN6 |
| A6 | Unknown key kept; failed registration shows an error | §4.2, §4.3 | `test_ran_s7` | RAN7 |
| A7 | Exhaustive `PushService`, labels | §2.1, §2.2, §4.4 | `test_ran_s8`, `test_ran_push_service_maps_each_host`, `test_ran_push_host_is_the_single_source_of_the_allowlist` | RAN8 |
| A8 | FCM `/fcm/send/` + `/wp/`, Mozilla `/wpush/v1/` + `/wpush/v2/` end to end; Chrome subscription JSON; Chrome clientDataJSON; GPM registration | §0.1 | §11.2 push-protocol, push-sender, `android_push_tests.rs`, `android_webauthn_tests.rs` | RAN9, RAN10 |
| A9 | Docs Android section, neutral push text; installer neutral | §7 | `test_ran_s9`, `test_ran_s10` | RAN11 |
| A10 | ADR, matrix rows (owner Android check), walkthrough | §13, §14 | `test_ran_s9` (ADR title) | RAN11, RAN12 |

---

## 11. Test Hooks and Tests for the Tester (Phase 2)

### 11.1 Static invariants — `tests/invariants/src/remote_android_contract.rs` (new; `mod remote_android_contract;` in `lib.rs`)

Helpers may reuse `remote_companion_contract::{read, read_bytes, exists, workspace_root}` and, after making
`remote_brand_contract::{png_info, compact_json}` `pub(crate)` (visibility only, no assertion change), the PNG parser.
A small `body_of_function(js, name)` helper (brace matching outside strings/comments, like `body_of` of the push
contract) scopes function checks.

| Test | Asserts (red before implementation) |
|---|---|
| `test_ran_s1_service_worker_notification_options` | `sw.js` contains `const ICON_PATH = "/icon-192.png";`, `const BADGE_PATH = "/badge-96.png";`, and inside the `push` listener's `showNotification` options `renotify: true`, `icon: ICON_PATH`, `badge: BADGE_PATH`, `tag: shown.tag`; the `push` listener body and `readNotification` contain no `fetch(`; whole file still free of `addEventListener("fetch"`, `importScripts`, `caches`, `localStorage`, `indexedDB`, `http://`, `https://`, `eval(`, `innerHTML`, `Function(`, `setInterval` |
| `test_ran_s2_service_worker_resubscribes_once_with_the_page_request_shape` | `addEventListener("pushsubscriptionchange"` once, wrapped in `event.waitUntil(` with a terminal `.catch(() => null)`; exactly one `fetch(` in `sw.js`, inside `postSubscription`; `postSubscription` contains `method: "POST"`, `"X-Soos-Action": SUBSCRIBE_ACTION`, `"Content-Type": "application/json"`, `JSON.stringify(subscription.toJSON())`, `credentials: "same-origin"`, `cache: "no-store"`, `signal: controller.signal`, `clearTimeout(timer)`; `const RESUBSCRIBE_TIMEOUT_MS = 10000;`; `SUBSCRIBE_PATH`/`SUBSCRIBE_ACTION` literal values equal `PUSH_SUBSCRIBE_PATH` parsed from `crates/remote/src/routes.rs` (L90) and `ACTION_PUSH_SUBSCRIBE` parsed from `crates/remote/src/lib.rs` (L425, PE-2); `renewedSubscription` uses `event.newSubscription`, `oldSubscription.options.applicationServerKey`, `userVisibleOnly: true`, and contains no `fetch(`; no `while (`, `for (`, `setInterval`, `showNotification` in either function; no `"/api/push"` GET |
| `test_ran_s3_manifest_is_installable_on_android` | compacted manifest contains `"id":"/"`; exactly five icon objects, in the §5 order, each with exactly the listed members (`src`, `sizes`, `type`, and `purpose` only on the three PNG entries: `any`, `any`, `maskable`); no `http`; `index.html` still has `<link rel="apple-touch-icon" href="apple-touch-icon.png">` |
| `test_ran_s4_android_icons_are_well_formed_pngs` | for each new PNG: signature, IHDR dims (192², 512², 512², 96²), depth 8, color type 2/2/2/6, no interlace, chunks exactly `IHDR, IDAT+, IEND`, size bounds of §6; `Docs/REMOTE_COMPANION.md` §2e contains each of the four output paths, `-w 408 -h 408`, `xc:#0047BB`, `-gravity center -composite`, `-alpha shape`, `png:color-type=6`, `exclude-chunks=date,time,tIME`, `safe zone` |
| `test_ran_s5_android_assets_are_embedded_and_routed` | `assets.rs` has the four variants and `include_bytes!("../assets/<file>")` with `content_type: "image/png"` in the same arm; `routes.rs` `route()` maps the four exact paths to `Route::Asset(AssetId::…)`; `http.rs` CSP literal is byte-identical to the pinned string of `http_tests.rs` (`img-src 'self'`, no `worker-src`/`blob:`/`data:` added) |
| `test_ran_s6_user_text_is_platform_neutral` | in `app.js` and `index.html` every `Face ID` is followed by `, fingerprint or screen lock`; no `Touch ID`, `iPhone settings`, `Passwords app`; `app.js` contains the four §4.1 constants with their exact values and `PASSKEY_PROMPT_TEXT` is used at least four times; `index.html` `#login-button` text is `Sign in with your passkey` |
| `test_ran_s7_push_client_keeps_unknown_key_subscriptions_and_bounds_the_worker_wait` | `function serverKeyMatch(` exists and returns `"unknown"` before any byte comparison when `applicationServerKey` is falsy; `sameServerKey` absent; in `syncSubscription`, `unsubscribe()` occurs only under `=== "different"`; `const SERVICE_WORKER_READY_TIMEOUT_MS = 10000;`; `navigator.serviceWorker.ready` occurs once in `app.js`, inside `readyRegistration`, which also contains `setTimeout(` and `clearTimeout(`; `enableNotifications` contains `await readyRegistration()` and `PUSH_WORKER_FAILED_TEXT`, and no `serviceWorker.ready`; first `await` still `Notification.requestPermission(` |
| `test_ran_s8_push_service_classification_is_exhaustive` | comment-stripped `impl PushService` block of `push.rs` contains `PushHost::Apple => Self::Apple`, `PushHost::Google => Self::Google`, `PushHost::Mozilla => Self::Mozilla` and no `_ =>` nor `"web.push.apple.com"`-style string arm; `push-protocol/src/lib.rs` `parse` uses `PushHost::ALL`; `app.js` `PUSH_SERVICES` is exactly the §4.4 object |
| `test_ran_s9_android_is_documented` | `Docs/REMOTE_COMPANION.md` has `## 2h.` between `## 2g.` and `## 3.`, containing `Tailscale`, `Install app`, `Android 13`, `Google Password Manager`, `fingerprint`, `screen lock`, `Unrestricted`, `Firefox`, `Samsung Internet`, `fcm.googleapis.com`, `updates.push.services.mozilla.com`, `pushsubscriptionchange`, `Tasker`, `RAN12`; §2d contains `Android`, `fcm.googleapis.com`; §6 lists the four PNG paths; `AI/DECISIONS.md` contains the §14 ADR title; `AI/tester_contract_remote_android.md` exists |
| `test_ran_s10_installer_text_is_platform_neutral` | `scripts/install_remote.sh`: no `iPhone lock screen`, `register Face ID`, `Touch ID`; every `Face ID` followed by `, fingerprint or screen lock`; contains `register the phone's passkey over the tailnet` and `(the home-screen app on iPhone)` |

### 11.2 Behaviour tests (Rust)

**`crates/push-protocol/tests/android_endpoint_tests.rs`** (new)

- `test_ran_endpoint_shapes_are_accepted`: real-shaped endpoints accepted with the right `host()`, `origin()`
  and `push_host()`:
  `https://fcm.googleapis.com/fcm/send/<token with ':' and '-' '_'>` → `Google`;
  `https://fcm.googleapis.com/wp/<token with ':'>` → `Google`;
  `https://updates.push.services.mozilla.com/wpush/v1/<b64url>` → `Mozilla`;
  `https://updates.push.services.mozilla.com/wpush/v2/<b64url ending '='>` → `Mozilla`;
  each also at exactly `MAX_PUSH_ENDPOINT_BYTES` (padded token) accepted and at +1 refused (`TooLong`).
- `test_ran_push_host_is_the_single_source_of_the_allowlist`: `PushHost::ALL.map(PushHost::name) == PUSH_HOSTS`;
  `push_host().name() == host()` for every allowlisted shape; a look-alike host (`fcm.googleapis.com.evil.example`,
  `xfcm.googleapis.com`, `FCM.googleapis.com`) still `HostNotAllowed`.

**`crates/push-sender/tests/android_sender_tests.rs`** (new)

- `test_ran_sender_accepts_android_endpoints`: for the four §11.2 endpoints, a `DeliveryRequest` frame through
  `serve_connection` with a fake `Deliverer` reaches the deliverer with the endpoint unchanged and answers its scripted
  outcome; `filter_addresses(endpoint.host(), lookup)` with a public address keeps it and with a private one refuses
  (same as Apple).

**`crates/remote/tests/android_push_tests.rs`** (new, `mod common;` harness of `push_server_tests.rs`)

- `test_ran_push_service_maps_each_host`: `PushService::of` on one endpoint per host → `Apple`, `Google`,
  `Mozilla`; serialized `"apple"`, `"google"`, `"mozilla"`.
- `test_ran_endpoints_subscribe_store_and_dispatch`: on a push-enabled harness, subscribe the four Android
  endpoints (FCM `/fcm/send/`, FCM `/wp/`, Mozilla v1, Mozilla v2; plus one Apple to reach the limit is **not** used:
  four total) → each `200 subscribed`; `GET /api/push` lists four devices with `google`, `google`, `mozilla`,
  `mozilla`; the store file round-trips; one live failed-password attempt dispatches one request per endpoint to the
  fake transport with VAPID `aud` = `https://fcm.googleapis.com` / `https://updates.push.services.mozilla.com`,
  `Urgency: high`, and a payload that decrypts with each UA key; a `410` reply removes that endpoint only.
- `test_ran_chrome_subscription_json_parses`: the exact Chrome `toJSON()` shape
  `{"endpoint":"https://fcm.googleapis.com/wp/…","expirationTime":null,"keys":{"p256dh":"…","auth":"…"}}` and the same
  with members reordered (`keys` first) → `200 subscribed`.
- `test_ran_worker_resubscribe_shape_is_accepted`: tailnet caller with headers `X-Soos-Action: push-subscribe`,
  `Content-Type: application/json`, `Origin: https://<host>`, `Sec-Fetch-Site: same-origin`, `Sec-Fetch-Mode: cors`,
  `Sec-Fetch-Dest: empty` → `200`; same over Funnel with a valid session cookie → `200`.
- `test_ran_worker_resubscribe_without_funnel_session_is_refused`: the same request over Funnel without or with
  an expired session → `403 login_required`, store unchanged; with `Sec-Fetch-Site: same-site` or `cross-site` →
  `403 forbidden`; `X-Soos-Action` missing → `403 forbidden` (regression guard that nothing was loosened).

**`crates/remote/tests/android_webauthn_tests.rs`** (new, `mod common;` passkey fixtures)

- `test_ran_chrome_client_data_with_extra_members_verifies`: clientDataJSON
  `{"type":"webauthn.get","challenge":"…","origin":"https://pc.tail1234.ts.net","crossOrigin":false,"other_keys_can_be_added_here":"do not compare clientDataJSON against a template. See https://goo.gl/yabPex"}`
  (and the `webauthn.create` variant) parses and a signed assertion over it verifies; with `"crossOrigin":true` it is
  still refused (`CrossOrigin`).
- `test_ran_google_password_manager_registration_verifies`: registration authenticatorData with flags
  `UP|UV|AT|BE|BS` (0x5D), counter 0 and AAGUID `ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4` (the fixture writes the AAGUID
  bytes; a new helper `registration_auth_data_with_aaguid` in the test file, `common/passkey.rs` unchanged), `fmt:
  "none"`, empty `attStmt` → accepted with `backup_eligible` and `backup_state` true and `sign_count == 0`.
- `test_ran_google_password_manager_assertion_with_zero_counter_verifies`: two consecutive assertions with
  counter 0 against a stored credential with counter 0 and BE=1 both verify; BS may flip 1→0→1; an assertion with BE=0
  against the stored BE=1 is still refused (`WebAuthnError::BackupEligibilityChanged`).

**`crates/remote/tests/android_assets_tests.rs`** (new; uses the `png` dev-dependency)

- `test_ran_assets_are_routed_as_png`: `route(GET|HEAD, path)` → `Route::Asset(id)` for the four paths;
  `POST`/`Other` → `MethodNotAllowed`; `allow_header(path) == "GET, HEAD"`; `/icon-192.png/`, `/Icon-192.png`,
  `/icon-192.PNG` → `NotFound`; `asset(id).content_type == "image/png"` and body equals the file on disk;
  `is_funnel_public(Route::Asset(id))`.
- `test_ran_assets_are_served_with_the_unchanged_csp`: through the server harness, tailnet and anonymous
  Funnel `GET` → `200`, `Content-Type: image/png`, the exact pinned CSP, `X-Content-Type-Options: nosniff`,
  `Cache-Control: no-store`; `HEAD` → same headers, empty body; `POST` → `405`.
- `test_ran_maskable_icon_keeps_the_mark_in_the_safe_zone`: decoded RGB 512×512; every pixel whose centre
  `(x+0.5, y+0.5)` is farther than 204.8 from (256, 256) is exactly `(0x00,0x47,0xBB)`; (266, 246) and (246, 266) are
  `(0xED,0xF1,0xFF)`.
- `test_ran_badge_is_white_on_transparent`: decoded RGBA 96×96; every pixel with alpha > 0 has RGB
  `(255,255,255)`; opaque (alpha ≥ 128) fraction in `[0.08, 0.14]`; alpha 0 at the four corners, (24, 24), (72, 72);
  alpha 255 at (52, 44) and (44, 52).
- `test_ran_any_icons_are_full_bleed_brand_renders`: `icon-192.png` and `icon-512.png` decode to opaque RGB of
  their size, four corner pixels `(0x00,0x47,0xBB)`, centre-right of the top-right quadrant (e.g. (0.55 n, 0.45 n))
  `(0xED,0xF1,0xFF)` ± 2 per channel.

Expected red/green: invariants s57–s66, `test_ran_push_host_*`, `test_ran_push_service_*` (needs `pub` +
`PushHost`), all asset tests and the amended assertions are **red** before implementation. Endpoint, sender,
subscription-JSON, WebAuthn and worker-request-shape tests are **coverage contracts expected green** from the start
(the wire already accepts Android, §0.1); the tester records them as such in
`AI/tester_contract_remote_android.md`, which is not a weakening: they pin behaviour the issue asks to keep.

### 11.3 Injectable points

None new: the existing `PushTransport` fake, the push harness, passkey fixtures, the server harness (`Via::Tailnet`,
Funnel with/without session) and the pure functions `route`, `allow_header`, `is_funnel_public`, `asset`,
`PushEndpoint::parse`, `PushService::of`, `parse_client_data`, `verify_registration`, `verify_assertion`,
`filter_addresses`, `serve_connection` suffice. JS is tested statically (the project has no JS runtime in CI).

---

## 12. Owner-approved amendments (O-2, 2026-10-09)

Each replaces an old value by a new **exact** value. None removes a check or widens a set beyond the listed values.

| # | File / test | Old needle / assertion | New needle / assertion |
|---|---|---|---|
| AM-1 | `tests/invariants/src/remote_brand_contract.rs` const `ASSET_FILES` (L342–L350), used by `test_rmc_s54_system_fonts_and_fixed_asset_set` | `[&str; 7]` = `index.html, app.js, style.css, sw.js, manifest.webmanifest, icon.svg, apple-touch-icon.png` | `[&str; 11]` = the same seven + `icon-192.png`, `icon-512.png`, `icon-maskable-512.png`, `badge-96.png` |
| AM-2 | same file, `test_rmc_s54_system_fonts_and_fixed_asset_set` (L2402–L2412) | message `"the asset directory holds exactly seven files"`; `assets_rs.matches("include_bytes!(\"../assets/").count()` `== 7`, message `"assets.rs embeds seven files"` | message `"the asset directory holds exactly eleven files"`; count `== 11`, message `"assets.rs embeds eleven files"` (the directory listing still equals `ASSET_FILES` exactly) |
| AM-3 | same file, module doc L20 and test doc L2376 | "the fixed seven-file asset set" | "the fixed eleven-file asset set (amended by ADR 2026-10-09 Android)" |
| AM-4 | same file, `test_rmc_s53_manifest_and_meta_colors` (L2312–L2361) | `assert_eq!(objects.len(), 2, "exactly two manifest icons")`; two entry triples (`icon.svg`/`any`/`image/svg+xml`, `apple-touch-icon.png`/`180x180`/`image/png`) | `assert_eq!(objects.len(), 5, "exactly five manifest icons")`; the two triples kept plus `["\"src\":\"icon-192.png\"", "\"sizes\":\"192x192\"", "\"type\":\"image/png\"", "\"purpose\":\"any\""]`, `["\"src\":\"icon-512.png\"", "\"sizes\":\"512x512\"", "\"type\":\"image/png\"", "\"purpose\":\"any\""]`, `["\"src\":\"icon-maskable-512.png\"", "\"sizes\":\"512x512\"", "\"type\":\"image/png\"", "\"purpose\":\"maskable\""]`; the pair list gains `"\"id\":\"/\""` |
| AM-5 | same file, const `BUTTON_LABELS` (L295–L305), used by `test_rmc_s50_ui_contract_ids_labels_and_attributes_are_kept` (L1986) | `("login-button", "Sign in with Face ID")` | `("login-button", "Sign in with your passkey")` |
| AM-6 | `tests/invariants/src/remote_push_contract.rs`, `test_rmc_s38_page_and_service_worker` (L672–L744), `app.js` needle list | `"home-screen app"` | `"On iPhone, notifications need the home-screen app (iOS 16.4 or later); this browser does not support them"` (the full `PUSH_UNSUPPORTED_TEXT`; strictly stronger, still mentions the iPhone home-screen app) |
| AM-7 (owner-approved 2026-10-09, PE-4) | `tests/invariants/src/remote_battery_contract.rs`, `test_rbs_s6_no_new_dependency` (L519–L526), dev-dependency allowlist | `["tokio", "tempfile", "proptest"]` | `["tokio", "tempfile", "proptest", "png"]` (exact; message unchanged; the `[dependencies]` set assertion untouched) |


**AM-7 gate (plan evaluator, PE-4).** RESOLVED 2026-10-09: the owner approved AM-7 in the main session, so the
"AM-7 approved" branch below is binding and the fallback is void. AM-1..AM-6 are owner-approved (2026-10-09). AM-7 was not covered by that
approval: the orchestrator asks the owner before Phase 2. Exactly one of the two branches is then binding:

- **AM-7 approved**: `crates/remote/Cargo.toml` `[dev-dependencies]` gains `png = "=0.18.1"`; the pixel tests of §11.2
  (`test_ran_maskable_icon_keeps_the_mark_in_the_safe_zone`, `test_ran_badge_is_white_on_transparent`,
  `test_ran_any_icons_are_full_bleed_brand_renders`) decode with `png` in `crates/remote/tests/android_assets_tests.rs`.
  The auditor confirms `git diff Cargo.lock` touches only the `soos-remote` dependency list and `cargo deny check` is
  green.
- **AM-7 declined or not answered (fallback)**: no dev-dependency is added and `test_rbs_s6_no_new_dependency` is not
  touched. The three pixel tests move, unchanged in their assertions, to `tests/invariants/src/remote_android_contract.rs`
  with a test-only zlib/DEFLATE decoder (stored, fixed and dynamic Huffman blocks; RFC 1950/1951) plus PNG
  un-filtering (filter types 0–4) in that module. The decoder must verify the zlib **Adler-32 trailer** of the
  concatenated `IDAT` stream and the decoded length `height × (1 + width × channels)` and fail the test on any
  mismatch, so a decoder bug can only turn a test red, never green. It must also be self-tested on
  `crates/remote/assets/apple-touch-icon.png` (Adler-32 match and corner pixel `(0x00,0x47,0xBB)`). The invariants crate
  stays dependency-free.

Checked and **not** amended (they keep passing unchanged; listed so the plan evaluator need not re-check):

- `remote_brand_contract::test_rmc_s52_icons_are_the_brand_mark` (L2222): checks `icon.svg`, the inline stars and the
  180×180 touch icon only; all kept. New PNG checks live in `test_ran_s4`.
- `remote_companion_contract::test_rmc_s8_assets_exist_load_nothing_remote_and_have_no_inline_script` (L934) and
  `test_rmc_s8_app_js_is_textcontent_only_and_carries_the_ui_constants` (L1015): existence lists and substring checks
  (`"display": "standalone"` etc.), not exact sets; the §5 manifest formatting keeps them green.
- `remote_push_contract::test_rmc_s38` `sw.js` forbidden list: no forbidden needle is introduced (`fetch(` is not in
  it; `https://` stays absent because the worker uses relative paths).
- `remote_push_contract::test_rmc_s39_push_is_documented`: `home-screen`, `16.4`, `web.push.apple.com` stay in §2d.
- `remote_camera_contract::test_rlc_s8_camera_is_documented`: `Face ID` stays in §2f inside "Face ID, fingerprint or
  screen lock".
- `remote_passkey_contract` (`.register(` count 1, `SERVICE_WORKER_PATH`): unchanged.
- `remote_battery_contract::test_rbs_s6_no_new_dependency`: **not** unaffected (PE-4): its dev-dependency half needs
  AM-7 or the fallback above.
- `remote_battery_contract` (sw.js has no `battery`), `remote_camera_contract` (sw.js has no `"camera"`/`soos-camera`):
  the new worker text must not introduce these words.
- `crates/push-protocol/tests/protocol_tests.rs` `PUSH_HOSTS` literal (L47): value unchanged.
- `crates/remote/tests/routes_tests.rs` `test_rmc_assets_table_content_types_and_bodies` (L125): fixed list, still
  passes; the four new ids are covered by `test_ran_assets_are_routed_as_png`.

`AI/VERIFICATION_MATRIX.md` rows RMC84–RMC86 (text, not tests) gain "(amended by RAN3–RAN5, ADR 2026-10-09)".

---

## 13. Verification Matrix Rows (traceability phase)

New block `## Component: remote-android (GitHub #349, feat/remote-android, walkthrough 193)`:

Under the AM-7 fallback (§12) the three pixel tests are cited as
`soos-invariants::remote_android_contract::test_ran_*` instead of `soos-remote::android_assets_tests::test_ran_*`; the
test names are the same in both branches.

| # | Criterion | Test Method |
|---|---|---|
| RAN1 | `sw.js` notifications carry `renotify: true`, `icon` `/icon-192.png`, `badge` `/badge-96.png` and the existing tags; the worker makes no request on push (the browser loads only the two same-origin images) and stores nothing; every push still shows a notification | `soos-invariants::remote_android_contract::test_ran_s1_service_worker_notification_options`, `soos-remote::android_assets_tests::test_ran_badge_is_white_on_transparent` |
| RAN2 | `pushsubscriptionchange`: one bounded (10 s) same-origin POST to `/api/push/subscribe` with the page's exact headers and credentials, same key, silent, no retry; the server checks are unchanged and refuse it without a Funnel session | `…::test_ran_s2_service_worker_resubscribes_once_with_the_page_request_shape`, `soos-remote::android_push_tests::{test_ran_worker_resubscribe_shape_is_accepted, test_ran_worker_resubscribe_without_funnel_session_is_refused}` |
| RAN3 | Manifest `id` `/` and exactly five icons (SVG, 180 touch icon, 192/512 `any`, 512 `maskable`) | `…::test_ran_s3_manifest_is_installable_on_android`, amended `remote_brand_contract::test_rmc_s53_manifest_and_meta_colors` |
| RAN4 | Icons generated by the documented deterministic commands; well-formed PNGs within byte bounds; maskable mark inside the 40 % safe-zone circle on `#0047BB`; badge white on transparent; `any` icons full-bleed | `…::test_ran_s4_android_icons_are_well_formed_pngs`, `soos-remote::android_assets_tests::{test_ran_maskable_icon_keeps_the_mark_in_the_safe_zone, test_ran_any_icons_are_full_bleed_brand_renders}` |
| RAN5 | Four new assets embedded and routed `GET`/`HEAD` only as `image/png`, Funnel-public, exact CSP unchanged; eleven-file asset set | `…::test_ran_s5_android_assets_are_embedded_and_routed`, `soos-remote::android_assets_tests::{test_ran_assets_are_routed_as_png, test_ran_assets_are_served_with_the_unchanged_csp}`, amended `test_rmc_s54_system_fonts_and_fixed_asset_set` |
| RAN6 | Platform-neutral user text (passkey phrase, push unsupported/blocked texts, login label, installer) | `…::{test_ran_s6_user_text_is_platform_neutral, test_ran_s10_installer_text_is_platform_neutral}`, amended `test_rmc_s50_*`, `test_rmc_s38_*` |
| RAN7 | A subscription without a readable `applicationServerKey` is kept; the worker wait is bounded and a failed registration shows `PUSH_WORKER_FAILED_TEXT` | `…::test_ran_s7_push_client_keeps_unknown_key_subscriptions_and_bounds_the_worker_wait` |
| RAN8 | `PushHost` single source of the allowlist; `PushService::of` exhaustive; labels "Apple", "Android / Chrome (Google)", "Firefox (Mozilla)" | `…::test_ran_s8_push_service_classification_is_exhaustive`, `soos-push-protocol::android_endpoint_tests::test_ran_push_host_is_the_single_source_of_the_allowlist`, `soos-remote::android_push_tests::test_ran_push_service_maps_each_host` |
| RAN9 | FCM `/fcm/send/`, `/wp/` and Mozilla `/wpush/v1/`, `/wpush/v2/` endpoints accepted end to end (parse, sender frame and filter, subscribe, store, dispatch, `410` removal); Chrome `toJSON()` shape parses | `soos-push-protocol::android_endpoint_tests::test_ran_endpoint_shapes_are_accepted`, `soos-push-sender::android_sender_tests::test_ran_sender_accepts_android_endpoints`, `soos-remote::android_push_tests::{test_ran_endpoints_subscribe_store_and_dispatch, test_ran_chrome_subscription_json_parses}` |
| RAN10 | Chrome clientDataJSON with extra members verifies (`crossOrigin: true` still refused); Google Password Manager registration (BE=1, BS=1, counter 0, real AAGUID) and repeated zero-counter assertions verify; BE change still refused | `soos-remote::android_webauthn_tests::{test_ran_chrome_client_data_with_extra_members_verifies, test_ran_google_password_manager_registration_verifies, test_ran_google_password_manager_assertion_with_zero_counter_verifies}` |
| RAN11 | `Docs/REMOTE_COMPANION.md` §2h Android section and neutral push text, §2e icon commands, §6 asset list; ADR 2026-10-09 recorded | `…::test_ran_s9_android_is_documented` |
| RAN12 | Hardware (owner): on an Android phone (Chrome; Samsung Internet or Firefox if available) — (1) over the tailnet with the Tailscale app, Chrome **Install app** adds the soos icon (maskable, not clipped) and opens standalone with the blue status bar; (2) enroll a passkey with `soos-remote enroll-code`, sign in over Funnel with fingerprint or screen lock; (3) **Enable notifications** (Android 13+ system prompt accepted), the device list shows "Android / Chrome (Google) device"; (4) **Send test notification** arrives with the soos icon and the white star badge in the status bar; two failed-password bursts within the coalescing window under the same tag both sound/vibrate (`renotify`); (5) a failed password at the PC with the phone screen off and Chrome set to Unrestricted battery arrives within about 1 min; (5b) with the Tailscale VPN off on the phone and Funnel off, a failed-password notification still appears (the icon may fall back to the browser default); (6) lock and unlock from the phone work; (7) the iPhone home-screen app still works as before (RMC21/RMC88 spot check) | Manual, owner; recorded in walkthrough 193 |

---

## 14. ADR Draft (to be added to `AI/DECISIONS.md` by the developer/traceability phase; text authoritative there)

* **[2026-10-09] Android Support for the `soos-remote` Phone Companion and Web Push (GitHub #349; owner decisions of
  2026-10-09: full Android parity, exact amendment of the iPhone-only test pins; branch `feat/remote-android`; spec
  `AI/architect_spec_remote_android.md`):** *Amends, without superseding, item (1) "Scope" of the 2026-10-06 ADR "Web
  Push Notifications for Failed-Password Alerts Through a Separate Sender Unit" (the receiving web app is the owner's
  phone web app: the iPhone/iPad home-screen app, iOS/iPadOS 16.4 or later, or Chrome, Samsung Internet or Firefox on
  Android, in a tab or installed), its service-worker sentence ("has no `fetch` handler and stores nothing" stays true;
  the worker may now issue exactly one same-origin `POST /api/push/subscribe` on `pushsubscriptionchange`), and item (3)
  "Presentation only" of the 2026-10-06 ADR "soos Brand Direction Applied to the `soos-remote` Web App" together with
  item (5) of the 2026-10-07 battery ADR (the fixed asset set grows from seven to eleven files and the manifest from two
  to five icons). Does not change the IPC ADR, RC-1 (`soos-remote` opens no network socket), the push allowlist, the
  CSRF rules, the CSP or any route other than four new static assets.* (1) **Wire unchanged**: FCM
  (`fcm.googleapis.com`, `/fcm/send/` and `/wp/` paths) and Mozilla (`updates.push.services.mozilla.com`, `/wpush/v1/`
  and `/wpush/v2/`) endpoints were already allowlisted; RFC 8030/8291/8292 and WebAuthn ES256 with `none` attestation
  cover Chrome Android and Google Password Manager (synced passkeys, BE=1/BS=1, counter 0). A `PushHost` enum in
  `soos-push-protocol` becomes the single source of `PUSH_HOSTS` and makes the device classification exhaustive (no
  fallback arm). (2) **Service worker**: every notification uses `renotify: true` (a second burst under the same tag
  alerts again), the same-origin raster `icon` `/icon-192.png` and the monochrome `badge` `/badge-96.png`; the worker
  still makes no request on push (the browser itself loads the two same-origin images when it displays the notification;
  a failed load still shows it). On `pushsubscriptionchange` the worker takes the browser's renewed subscription, or
  re-subscribes with the old subscription's own application server key, and posts it once with the page's exact request
  shape (`X-Soos-Action: push-subscribe`, JSON body, same-origin credentials), aborted after 10 s, never retried, its
  result ignored; the server's checks are unchanged, so it succeeds only where the page would (tailnet identity, or a
  valid Funnel web session) and is otherwise silently refused; the page re-sends the subscription on its next open.
  (3) **Install**: the manifest gains `id` `/` and PNG icons 192 and 512 (`any`) and 512 `maskable`, generated
  deterministically from `icon.svg` with the commands of `Docs/REMOTE_COMPANION.md` §2e; the maskable icon draws the
  mark at 408 px on brand blue `#0047BB` so its visible spikes stay inside the 40 % safe zone; the 180 px
  `apple-touch-icon.png` is kept for iOS. The four PNGs are embedded, served `GET`/`HEAD` only as `image/png` under the
  unchanged CSP. (4) **Wording**: user-visible text is platform-neutral ("passkey (Face ID, fingerprint or screen
  lock)"; no "iPhone settings"); the "push not supported" message names both cases. (5) **Robustness**: the page keeps
  a subscription whose `options.applicationServerKey` is unreadable instead of unsubscribing it on every load, and a
  failed or non-activating service worker ends the "Enable notifications" tap with an error after at most 10 s.
  **Accepted risks**: a passkey synced by Google Password Manager is as strong as the Google account (as iCloud Keychain
  for iPhone); Google or Mozilla see delivery metadata, never the content; vendor battery savers may delay pushes
  (documented: set the browser to Unrestricted); a renewed subscription is lost until the next app open when the PC
  refuses the worker's request (off-tailnet, expired Funnel session, four devices registered); over Funnel the worker's
  single request refreshes a still-valid session's idle timer (absolute lifetime unchanged). **Out of scope**: native
  Android app, FCM direct API keys, Edge desktop push (`*.notify.windows.com`), Android Shortcuts/Tasker integration
  (pointer only). Matrix RAN1–RAN12 (RAN12 owner check on an Android phone); walkthrough 193.

---

## 15. Documentation Drift / Open Points

1. **Walkthrough number**: the issue says 191; `main` already has 191 (live camera) and 192 (battery). This spec uses
   **193**. The issue body may be edited, no other impact.
2. **Matrix ids** (settled, PE-1): dedicated prefix `RAN` (rows RAN1–RAN12, static tests `test_ran_s1_*` …
   `test_ran_s10_*`, behaviour tests `test_ran_*`), consistent with #345 `RLC` and #346 `RBS`.
3. **Chrome WebAPK on a tailnet-only host**: whether Chrome mints a WebAPK or a plain standalone shortcut for a
   `*.ts.net` origin reachable only over the tailnet is not verifiable without hardware; both open standalone. RAN12
   (1) records which one the owner gets; no code depends on it.
4. **Chrome and `pushsubscriptionchange`**: Chrome rarely fires it; Firefox does. The page's `syncSubscription` on every
   open remains the main recovery path; the worker handler is best effort by design.
5. **`png` dev-dependency** (PE-4): accepted on supply-chain grounds (already locked at 0.18.1 through `image`; `png`,
   `fdeflate`, `flate2`, `crc32fast`, `miniz_oxide`, `bitflags`, `simd-adler32` are MIT / Apache-2.0 / Zlib, all allowed
   by `deny.toml`; no lock entry added). It needs owner amendment **AM-7** (§12) because `test_rbs_s6_no_new_dependency`
   pins the dev-dependency keys; without AM-7 the fallback of §12 applies.

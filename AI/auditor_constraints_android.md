# Audit Constraints — GitHub #349: Android Support for `soos-remote` and Web Push

- **Date**: 2026-10-09
- **Branch**: `feat/remote-android` (audited at `2cd5b0c`, diff against `origin/main` = `24d0d8c`)
- **Inputs**: `AI/architect_spec_remote_android.md` (round 1 + PE-1..PE-6), `AI/plan_evaluator_report.md` (APPROVED),
  `AI/tester_contract_remote_android.md`, test commits `a5e6d8c` (AM-1..AM-7), `7b7f08c` (new tests), `2cd5b0c`
  (type-only AM-4 compile fix).
- **Code about to change**: `crates/remote/assets/{sw.js,app.js,manifest.webmanifest,index.html}` + four new PNGs,
  `crates/remote/src/{assets.rs,routes.rs,push.rs}`, `crates/push-protocol/src/lib.rs`, `scripts/install_remote.sh`,
  `Docs/REMOTE_COMPANION.md`, `AI/DECISIONS.md`, `AI/ARCHITECTURE.md`, `AI/VERIFICATION_MATRIX.md`.
  `crates/remote/src/http.rs` must **not** change.

## Checklist results (baseline, before implementation)

| # | Check | Result |
|---|---|---|
| 1 | Panic paths: `grep -nE '\.unwrap\(\|\.expect\(\|panic!\|todo!\|unimplemented!\|unreachable!'` on the production part of `push-protocol/src/lib.rs`, `remote/src/{assets,routes,http,push}.rs` | 0 hits |
| 2 | Unsafe | 0 hits; `#![forbid(unsafe_code)]` present in `soos-push-protocol` (L10) and `soos-remote` (L36) |
| 3 | Output isolation | no `println!/eprintln!/dbg!` touched; no `console.*` in `app.js` or `sw.js` |
| 4 | Bounded I/O | server side unchanged (assets are `include_bytes!`, served by the existing generic path); client side: one new worker `fetch` and one page wait, both must be bounded (C-6, C-7, C-12) |
| 5 | Arithmetic | no new arithmetic in production Rust |
| 6 | Filesystem | no new file written at run time; PNGs are compile-time embedded |
| 7 | Secrets & privacy | notification text unchanged (`readNotification`); icon/badge URLs are constants; no new log line |
| 8 | Fail-closed | no PAM/daemon path touched; server CSRF/host/identity/Funnel-session gates unchanged (proved by `test_ran_worker_resubscribe_without_funnel_session_is_refused`) |
| 9 | Supply chain | `png = "=0.18.1"` under `[dev-dependencies]` only; `Cargo.lock` delta is exactly one line (`"png"` added to the `soos-remote` dependency list, no new `[[package]]`); `png 0.18.1` already locked through `image`; `cargo deny --locked check` (cargo-deny 0.20.2): `advisories ok, bans ok, licenses ok, sources ok` |
| 10 | CI/workflow | no `.github/` or `.githooks/` change; `scripts/install_remote.sh` gets comment/echo text only (C-17) |

Current red/green state verified: `cargo test --locked --all-features -p soos-invariants` → 536 passed, 14 failed
(exactly `test_ran_s1`..`s10` and the four AM-amended tests `test_rmc_s38/s50/s53/s54`), matching the tester contract.

## Amendment audit (no weakening beyond AM-1..AM-7)

`git diff origin/main -- tests/invariants/src/remote_{brand,push,battery}_contract.rs` contains only:
AM-1 (`ASSET_FILES` 7 → 11 exact names), AM-2 (count 7 → 11, messages), AM-3 (doc comments), AM-4 (`"id":"/"` added,
icon count 2 → 5 with three new exact member sets; `2cd5b0c` only retypes the array as `[&[&str]; 5]`, same needles,
same `any(... all(...))` assertion), AM-5 (login label), AM-6 (`"home-screen app"` → the full, strictly longer
`PUSH_UNSUPPORTED_TEXT`), AM-7 (`png` added to the exact dev-dependency allowlist; `[dependencies]` half untouched).
No assertion removed, no set widened beyond the listed values. `tests/common/*` unchanged. **Verdict: compliant.**

## Test strength review

The new tests would catch the important wrong implementations: a second `fetch(` in `sw.js`, a fetch in the push path,
extra headers or a missing `X-Soos-Action`, a hard-coded or fetched key, a retry loop (`while (`/`for (`/
`setInterval`), response inspection (`.then(`/`.status`/`.ok`/`.json()`), a manifest with extra/missing members, a CSP
change (two independent pins), a non-exhaustive `PushService::of` (`_ =>`/`.host()`/string arms refused), an
`unsubscribe()` outside the `"different"` branch, pixel-level icon regressions (decoded with `png`), a loosened server
check (403 `login_required`/`forbidden` regression guard against the real server). Gaps found (not weakenings; closed
by constraints below and checked by the candid reviewer): **G-1** `test_ran_s2` does not pin that the timer calls
`controller.abort()` nor that `clearTimeout` runs on rejection (C-6); **G-2** `test_ran_s1` does not pin that
`showNotification` is unconditional in the `push` listener (C-2); **G-3** the spec §4.3 snippet starts the 10 s timer
only after `registerServiceWorker()` resolves, so a hung `register()` would not be bounded (C-12); **G-4** the
`pushsubscriptionchange` null sentinel (no request when no subscription) is not pinned in the listener chain (C-5).

## Audit Constraints — Issue #349

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-1 | The `push` listener and `readNotification` issue no request and touch no storage: no `fetch(`, `caches`, `localStorage`, `indexedDB`, `importScripts`, no `fetch` listener anywhere in `sw.js`. The words `caches`, `https://`, `http://` must not appear **even in comments** (`test_rmc_s38` scans the raw file). | `sw.js::push listener`, `readNotification` | `test_ran_s1`, `test_rmc_s38` |
| C-2 | Every push shows exactly one notification: `event.waitUntil(self.registration.showNotification(...))` stays the unconditional body of the `push` listener (no early `return`, no `if` around it, no `try` that skips it). `readNotification` keeps its fallback title/body and always returns a non-empty `tag` (`soos-alerts` or `soos-test`), so `renotify: true` can never raise a `TypeError` (empty tag + renotify). | `sw.js::push listener`, `readNotification` | `test_ran_s1`, `test_rmc_s38`, candid review (G-2) |
| C-3 | Notification options add only `renotify: true`, `icon: ICON_PATH`, `badge: BADGE_PATH` to the existing `body`, `tag`, `lang`. `ICON_PATH`/`BADGE_PATH` are the constant same-origin paths `"/icon-192.png"`/`"/badge-96.png"` with no query string or fragment and no payload-derived part; no `image`, `actions`, `data`, `vibrate`, `requireInteraction`, `silent` or `timestamp` option is added; the notification text never contains anything beyond today's counts/classes (no password, endpoint, key or identity). | `sw.js::push listener` | `test_ran_s1`, candid review |
| C-4 | `sw.js` contains exactly one `fetch(`, inside `postSubscription`, to the constant `SUBSCRIBE_PATH` (`"/api/push/subscribe"`, equal to `PUSH_SUBSCRIBE_PATH`), method `POST`, headers exactly `{"X-Soos-Action": SUBSCRIBE_ACTION, "Content-Type": "application/json"}`, body `JSON.stringify(subscription.toJSON())`, `credentials: "same-origin"`, `cache: "no-store"`. No `"/api/push"` GET, no `.unsubscribe(`, no other route, no absolute URL. | `sw.js::postSubscription` | `test_ran_s2` |
| C-5 | Same key only: the renewed subscription is `event.newSubscription`, else `pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: <old subscription's options.applicationServerKey> })`; when that key is absent the chain resolves `null` and **no request is sent** (`subscription === null ? null : postSubscription(subscription)`). The worker never fetches, hard-codes or stores a key. | `sw.js::renewedSubscription`, `pushsubscriptionchange` listener | `test_ran_s2`, candid review (G-4) |
| C-6 | Bounded and silent: `const RESUBSCRIBE_TIMEOUT_MS = 10000;`; the timer is exactly `setTimeout(() => controller.abort(), RESUBSCRIBE_TIMEOUT_MS)` with `signal: controller.signal`, and `clearTimeout(timer)` runs on **both** outcomes (`.finally(...)`); the response is not read; the listener is `event.waitUntil(<chain>.catch(() => null))` with the catch terminal; no retry, loop, recursion, `setInterval`, re-dispatch, `showNotification` or storage in the re-subscription path. | `sw.js::postSubscription`, listener | `test_ran_s2`, grep `controller.abort()` and `.finally(` in `postSubscription` (G-1) |
| C-7 | No server-side loosening: `check_action_csrf`, `check_host`, `classify_request`, the Funnel session gate (`server.rs`), the push route rate gate, `MAX_PUSH_SUBSCRIPTIONS`, `SubscribeBody` (`deny_unknown_fields`), the store and the WebAuthn verifier are **not modified**. No new route other than the four GET/HEAD assets; no new `POST` route; `is_funnel_public` unchanged. | `crates/remote/src/{routes,server,websession,push,webauthn}.rs` | `test_ran_worker_resubscribe_*`, `git diff origin/main -- crates/remote/src/server.rs crates/remote/src/websession.rs crates/remote/src/webauthn.rs` empty; candid review |
| C-8 | CSP unchanged byte for byte; `crates/remote/src/http.rs` is not modified (no `worker-src`, `blob:`, `data:`, no new header). | `http.rs` | `test_ran_s5`, `http_tests.rs`, `server_tests.rs`, `git diff origin/main -- crates/remote/src/http.rs` empty |
| C-9 | `PushHost` (derive `Debug, Clone, Copy, PartialEq, Eq, Hash`, no serde) is the single source: `PUSH_HOSTS` derived from `PushHost::name` with the unchanged value and order; `parse` matches the authority with exact byte equality over `PushHost::ALL` (no case folding, no suffix/prefix match), keeping rule order and `EndpointError::HostNotAllowed`; `PushEndpoint` `Debug` stays redacted; `host()` signature unchanged. No `unwrap`/`expect`/indexing; `#![forbid(unsafe_code)]` kept; no new dependency of `soos-push-protocol`. | `push-protocol/src/lib.rs::{PushHost, PUSH_HOSTS, PushEndpoint::parse, push_host}` | `test_ran_s8`, `test_ran_push_host_is_the_single_source_of_the_allowlist`, `protocol_tests.rs` |
| C-10 | `PushService::of` is `pub`, a three-arm `match endpoint.push_host()` with no `_` arm, no string match, no `.host()`; serde names stay `apple`/`google`/`mozilla`. | `remote/src/push.rs::PushService::of` | `test_ran_s8`, `test_ran_push_service_maps_each_host`, `push_server_tests.rs` |
| C-11 | Four new `AssetId` variants each embedded with `include_bytes!("../assets/<file>")` and `content_type: "image/png"`; four exact string arms in `route()`'s `read_route` (`GET`/`HEAD` only, `405` otherwise, no prefix/case matching, no file-system read). | `assets.rs::asset`, `routes.rs::route` | `test_ran_s5`, `test_ran_assets_are_routed_as_png`, `test_ran_assets_are_served_with_the_unchanged_csp`, `test_rmc_s54` |
| C-12 | Bounded page wait covering the **whole** registration: `readyRegistration` resolves `null` at most `SERVICE_WORKER_READY_TIMEOUT_MS` (10000) after it is called, including when `navigator.serviceWorker.register()` itself never settles (start the timer before or around `registerServiceWorker()`, e.g. one `new Promise` whose `setTimeout(... resolve(null) ...)` is armed first; clear it on every early resolve). This tightens the spec §4.3 snippet (gap G-3) and keeps every `test_ran_s7` needle. `navigator.serviceWorker.ready` appears once, only there. | `app.js::readyRegistration` | `test_ran_s7`, candid review (G-3) |
| C-13 | `enableNotifications`: first `await` stays `Notification.requestPermission()`; then `await readyRegistration()`, `=== null` → `setText(pushFeedback, PUSH_WORKER_FAILED_TEXT)` and `return` **before** `pushManager.subscribe(`; the `finally` (re-enable button, `fetchPush()`) is unchanged so the button can never stay disabled. Text through `setText`/`textContent` only, no `innerHTML`. | `app.js::enableNotifications` | `test_ran_s7`, `test_rmc_s38` |
| C-14 | `serverKeyMatch` returns `"unknown"` (absent `options`, falsy `applicationServerKey`, non-string `publicKey`) **before** calling `fromBase64Url` or building any `Uint8Array`; only `"different"` reaches the single `.unsubscribe(` of `syncSubscription`; the existing outer `.catch(() => null)` stays (an invalid key that makes `atob` throw remains silent). | `app.js::serverKeyMatch`, `syncSubscription` | `test_ran_s7` |
| C-15 | PNGs are deterministic and metadata-free: generated only by the §6 commands documented in `Docs/REMOTE_COMPANION.md` §2e; chunks exactly `IHDR, IDAT+, IEND` (no `tEXt`, `iTXt`, `zTXt`, `tIME`, `iCCP`, `eXIf`: no path, date, user or host leaks); byte bounds 8/16/16/4 KiB; regenerating twice gives byte-identical files (developer checks `sha256sum` of two runs and records it in the walkthrough). | `crates/remote/assets/{icon-192,icon-512,icon-maskable-512,badge-96}.png` | `test_ran_s4`, pixel tests, `sha256sum` |
| C-16 | Supply chain: `png = "=0.18.1"` stays under `[dev-dependencies]` only (never `[dependencies]`); `Cargo.lock` delta stays the single `"png"` line in `soos-remote`; no other manifest or lock change; `cargo deny --locked check` green before push. | `crates/remote/Cargo.toml`, `Cargo.lock` | `test_rbs_s6_no_new_dependency`, `git diff origin/main -- Cargo.lock`, `cargo deny --locked check` |
| C-17 | `scripts/install_remote.sh`: only the six comment/`echo` text lines of spec §7.2 change; no command, template key, `sudo`, `/etc` write, permission or control flow change; `bash -n scripts/install_remote.sh` passes. | `scripts/install_remote.sh` | `test_ran_s10`, RMC-S6, RLC-S9, `git diff` review, `bash -n` |
| C-18 | No new log line, `tracing` field or `console.*` call; no new user-visible text reveals endpoints, keys, session or identity data; Docs/ADR say "the worker makes no request on push" (never "the push path makes no network access", PE-3) and document the accepted risks (Funnel idle-timer refresh, lost renewal until next open, GPM sync). | all changed files | grep `console\.` in assets = 0; `test_ran_s9`; candid review |
| C-19 | English only in code, comments, docs, ADR, walkthrough `AI/walkthroughs/193_remote_android.md` and commit messages; worker text must not introduce `battery`, `"camera"`, `soos-camera` (RBS/RLC contracts). | all changed files | invariants English/brand checks, `test_rbs_*`, `test_rlc_*`, candid review |
| C-20 | Zero test weakening: no file under `tests/`, `crates/*/tests/` or `tests/invariants/` is modified by the developer phase; the gate is `cargo fmt --all -- --check`, `cargo clippy --locked --all-features --all-targets -- -D warnings`, `cargo test --locked --all-features` for `soos-invariants`, `soos-remote`, `soos-push-protocol`, `soos-push-sender` all green. | developer phase | `git diff 2cd5b0c -- tests crates/*/tests` empty; CI commands |

### Pre-existing violations found (not introduced by this change)

- `app.js::enableNotifications` awaits `navigator.serviceWorker.ready` without a bound and `syncSubscription`
  unsubscribes subscriptions whose key is unreadable (both fixed by this change, spec §0.1; C-12, C-14).
- `PushService::of` falls back to `Mozilla` for any host (fixed by C-10).
- No panic, unsafe, log-leak or supply-chain violation found in the audited production code.

### Clearance: CLEARED

The spec, the tests and the amendments are sound; the gaps G-1..G-4 are closed by constraints C-2, C-5, C-6 and C-12,
which are satisfiable without touching any test and are checked by the candid reviewer.

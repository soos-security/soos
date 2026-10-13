# Auditor Constraints — soos-remote: browser capability detection and graceful degradation (GitHub #366, PR 1 of #355)

- **Date**: 2026-10-13
- **Branch**: `feat/remote-cross-browser` (GitHub-only issue; `Closes #366`, `Refs #355`)
- **Inputs audited**: `AI/architect_spec_remote_cross_browser.md` (round 2 APPROVED, §14 R-1 … R-8),
  `AI/tester_contract_remote_cross_browser.md`, ADR draft "[2026-10-13] Browser Support Policy for the
  `soos-remote` Web App", and the code about to change: `crates/remote/assets/app.js` and
  `crates/remote/assets/style.css`. `sw.js`, `index.html`, the manifest and every Rust source of every crate stay
  byte-identical.
- **Crates touched**: none in Rust production code. `soos-invariants` gains one test module.

## Audit results (checklist)

| # | Item | Result |
|---|---|---|
| 1 | Panic paths | No Rust production change. The test module uses `unwrap`/`expect`/indexing under the standard `#![allow(..., reason = ...)]` header. |
| 2 | `unsafe` | None added; no Rust production change. |
| 3 | Output isolation | Not a PAM change. Base `app.js`/`sw.js` have 0 `console` calls (grep); RXB-S5 keeps it that way. |
| 4 | Bounded I/O and deadlines | Page-side only. Existing bounds stay: worker wait 10 s, worker re-subscribe 10 s, camera part 512 KiB and buffer 1 MiB. New: the enable tap after a granted permission is bounded by `beforeDeadline` (timer cleared on both outcomes); the status poll is one 15 s interval cleared by `closeStream`. No unbounded loop or retry is added. |
| 5 | Arithmetic and numerics | `Math.max(0, deadline - Date.now())`: a past deadline fires at once, never a negative delay. |
| 6 | Filesystem | n/a (no file I/O in the page; static assets only). |
| 7 | Secrets and privacy | Base: 5 sites display a browser `error.message` (`app.js` login, enroll, push enable, camera stream, camera start; spec V-12). After the change only fixed strings reach `setText` (RXB-S5). No storage, no console, no push endpoint, passkey material or frame logged. |
| 8 | Fail-closed | Capability flags only disable or hide controls. No Rust change, so every server check (identity, CSRF, `Origin`, `Sec-Fetch-Site`, UV, session, push allowlist) is identical for every browser: no client flag can open a server path. |
| 9 | Supply chain | No dependency, no `Cargo.toml`/`Cargo.lock`/`deny.toml` change, no Node/npm, no CDN, no polyfill. |
| 10 | CI/workflows | Not touched. |

## Audit Constraints — GitHub #366

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-1 | `browserCapabilities()` is pure (no request, DOM write, storage, `new`, listener, permission request, `userAgent`) and is the only reader of `PublicKeyCredential` and `PushManager` | `app.js::browserCapabilities`, `pushSupported` | RXB-S1 |
| C-2 | Every `navigator.credentials`, `navigator.serviceWorker`, `.pushManager`, `Notification.`, `new EventSource(`, `.getReader(`, `createImageBitmap(` sits after a negated guard in an `if (` that exits; none at top level | `app.js` all top-level functions | RXB-S2 |
| C-3 | No User-Agent, `userAgentData`, `navigator.vendor`/`platform`, `appVersion` in `app.js`/`sw.js`; no `user-agent` in `crates/remote/src` | assets, server | RXB-S6 |
| C-4 | A browser exception message is never displayed: `.message` is read only in `failureText`, every displayable throw is `fixedError`, no `setText` argument is `.message`, `error` or `err`; no template literal | `app.js` | RXB-S5 |
| C-5 | No `console` call in `app.js` or `sw.js` (no endpoint, credential id, frame or error text logged) | assets | RXB-S5 |
| C-6 | Top-level constants are declared before their first top-level use (a load-time `ReferenceError` would stop lock and status as well) | `app.js` top level | RXB-S5 (TDZ check) |
| C-7 | The "Enable notifications" button is disabled only after a granted permission and re-enabled in `finally`; subscribe and the PC hand-off are bounded by one deadline of `SERVICE_WORKER_READY_TIMEOUT_MS`; the first `await` stays `Notification.requestPermission(`, and no comment containing the word "await" precedes it in the raw body | `app.js::enableNotifications`, `beforeDeadline` | RXB-S4, RAN-S7, `remote_push_contract.rs` test 48 |
| C-8 | A subscription refused with `unsupported_push_service` is unsubscribed locally (best effort, rejection swallowed) and never retried | `app.js::enableNotifications` | RXB-S4 |
| C-9 | Lock and status never depend on passkeys; passkey controls are disabled with `PASSKEY_UNSUPPORTED_TEXT`; the handlers return before any request (no challenge is issued to a browser that cannot answer it) | `app.js::requestLock`, `fetchStatus`, `openStream`, `requestLogin`, `requestUnlock`, `requestEnroll`, `startCameraView` | RXB-S3 |
| C-10 | The status poll fallback is a single interval, cleared by `closeStream` (hidden page, sign-out); it never coexists with an `EventSource` | `app.js::openStream`, `closeStream` | RXB-S9 |
| C-11 | Camera parser bounds unchanged (`CAMERA_MAX_PART_BYTES` 524288, `CAMERA_MAX_BUFFER_BYTES` 1048576, stream path pattern); no `createObjectURL`, `blob:`, image element | `app.js` camera | RXB-S10, RLC-S6 |
| C-12 | Server data reaches the new tile attribute only filtered to a `LABELS` key | `app.js::render` | code review (spec §3.7); RXB-S7 checks the hook |
| C-13 | CSS: no colour literal outside tokens; fallbacks reuse role tokens only; no `url(`, `@import`; the `:has()` fallbacks carry the same declarations | `style.css` | RXB-S7, brand RMC-S45/S47/S49 |
| C-14 | CSP, headers, CSRF, cookie, WebAuthn verifier, push allowlist and the 11-file asset set unchanged; no Rust production file in the diff | `crates/remote/src/**`, assets dir | RXB-S8, existing suites, `git diff --stat -- crates/*/src` empty |
| C-15 | No existing test modified | `tests/**`, `crates/*/tests/**` | `git diff --stat` shows only `tests/invariants/src/lib.rs` (registration) and the new module |

## Pre-existing violations found (not introduced by this change)

- `app.js` displays browser `DOMException` messages at 5 sites (`error && error.message ? error.message : …`). They are
  not a secret leak, but they break the fixed-string rule and can show engine-specific text. This change fixes
  them (C-4).

## Residual risks reviewed

- A second "Enable notifications" tap while the permission prompt is open can start a second identical flow; the
  subscribe call is idempotent on the PC (same endpoint upsert). Accepted (plan evaluator R-2).
- Without `EventSource`, the 15 s poll refreshes a Funnel session's idle timer while the page is open, as an open
  stream does. Accepted; the 8 h absolute lifetime is unchanged.
- Static checks cannot prove runtime behaviour in a given engine. The hardware rows RXB12–RXB20 and the future
  Playwright PR cover that.

## Clearance: CLEARED

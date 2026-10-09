# Walkthrough 193 — Android Support for the `soos-remote` Phone Companion and Web Push

- **Date**: 2026-10-09
- **Issue**: GitHub #349 (GitHub-only, no backlog id, like #339, #345 and #346; not registered in `BRANCH_TO_ISSUE`
  of `scripts/sync_issue.py`, the commits say `Refs #349` and the PR body says `Closes #349`). **Branch**:
  `feat/remote-android`, created from `origin/main` at `24d0d8c`. Nothing is pushed, deployed, installed or restarted
  by the agents; no agent runs `tailscale`.
- **ADR**: "[2026-10-09] Android Support for the `soos-remote` Phone Companion and Web Push" (`AI/DECISIONS.md`).
- **Documents**: spec `AI/architect_spec_remote_android.md` (round 1, amended by PE-1..PE-6); plan evaluation
  `AI/plan_evaluator_report.md`; tester contract `AI/tester_contract_remote_android.md`; auditor constraints
  `AI/auditor_constraints_android.md` (C-1–C-20, CLEARED).
- **Matrix criteria**: RAN1–RAN12 (RAN12 is the owner's Android hardware check, pending); RMC84–RMC86 text amended.
- **Walkthrough number**: the issue says 191, but 191 (live camera) and 192 (battery) already exist; 193 is used
  (spec §15).

## 1. Context & Objectives

The `soos-remote` phone web app and its Web Push alerts were built and tested for the owner's iPhone home-screen app.
Issue #349 asks for full Android parity (owner decision O-1): Chrome, Samsung Internet and Firefox, in a tab or
installed, with status, lock, unlock, alerts, push notifications, live camera and battery level.

Most of the wire already worked: FCM (`fcm.googleapis.com`) and Mozilla (`updates.push.services.mozilla.com`) were
already in the push allowlist, `SubscribeBody` accepts Chrome's `toJSON()` shape and the WebAuthn verifier accepts
Google Password Manager passkeys (BE=1/BS=1, counter 0). The gaps were presentation and robustness: no installable
raster icons, no monochrome badge, iPhone-only wording, a device classification that fell back to "Mozilla" for any
host, a page that unsubscribed subscriptions with an unreadable key on every load, and an unbounded wait for the
service worker.

## 2. Architect Design

- **D-1** `PushHost` enum in `soos-push-protocol` is the single source of the three hosts; `PUSH_HOSTS` is derived
  from it (value and order unchanged); `PushEndpoint` stores a `PushHost` and exposes `push_host()`, so
  `PushService::of` in `soos-remote` is a three-arm exhaustive `match`.
- **D-2** Maskable icon background is brand blue `#0047BB`: the visible mark (two pale spikes) stays inside the 40 %
  safe-zone circle (spike tips 204 px from the centre, limit 204.8 px of 512).
- **D-3** Badge is an alpha-only white silhouette of the pale spikes.
- **D-4** The worker makes exactly one network request, only on `pushsubscriptionchange`; the `push` path makes none.
- **D-5** No new server route other than four static assets, no server-check change.
- **D-6** One platform-neutral passkey phrase: "Face ID, fingerprint or screen lock".
- **D-7** Pixel properties tested in `crates/remote/tests` with the `png` dev-dependency (AM-7); the dependency-free
  invariants crate checks PNG structure only.
- **New assets**: `crates/remote/assets/{icon-192,icon-512,icon-maskable-512,badge-96}.png`, four `AssetId` variants,
  routed `GET`/`HEAD` only as `image/png`. Manifest gains `id` `/` and three PNG icons (five in total).
- **Constants**: `sw.js` `ICON_PATH = "/icon-192.png"`, `BADGE_PATH = "/badge-96.png"`,
  `RESUBSCRIBE_TIMEOUT_MS = 10000`; `app.js` `SERVICE_WORKER_READY_TIMEOUT_MS = 10000`, `PUSH_WORKER_FAILED_TEXT`,
  `PUSH_SERVICES` labels "Apple", "Android / Chrome (Google)", "Firefox (Mozilla)".
- **Invariants touched**: RC-1 (no network socket in `soos-remote`) unchanged; CSP unchanged byte for byte; CSRF,
  host, identity and Funnel-session gates unchanged.

## 3. Plan Evaluation

`VALIDATION_VERDICT: APPROVED` with one MAJOR and five MINOR items folded into the spec:

- **PE-4 (MAJOR)**: the `png` dev-dependency turns `test_rbs_s6_no_new_dependency` red; resolved by an explicit
  amendment gate AM-7 with a binding fallback (an in-crate DEFLATE decoder). The owner approved AM-7, so the
  `png = "=0.18.1"` branch is in force and the fallback is void.
- **PE-1** matrix prefix `RAN`; **PE-2** `ACTION_PUSH_SUBSCRIBE` lives in `crates/remote/src/lib.rs`; **PE-3** wording
  "the worker makes no request on push" (the browser itself loads the icon and badge) and RAN12 step 5b; **PE-5**
  out-of-scope bullet in `Docs/REMOTE_COMPANION.md` §9; **PE-6** the worker POST over Funnel refreshes a valid session's
  idle timer (accepted risk in the ADR).

### Owner-approved test amendments (2026-10-09, spec §12)

Each replaces an old value with a new exact value; none removes a check or widens a set beyond the listed values.
Committed in `a5e6d8c` (the type-only compile fix of AM-4 in `2cd5b0c`).

| # | Test | Amendment |
|---|---|---|
| AM-1 | `remote_brand_contract` const `ASSET_FILES` (`test_rmc_s54_system_fonts_and_fixed_asset_set`) | seven names → eleven exact names (+ the four PNGs) |
| AM-2 | `test_rmc_s54_system_fonts_and_fixed_asset_set` | "seven files" → "eleven files"; `include_bytes!` count 7 → 11 |
| AM-3 | `remote_brand_contract` module and test docs | "fixed seven-file asset set" → "fixed eleven-file asset set (amended by ADR 2026-10-09 Android)" |
| AM-4 | `test_rmc_s53_manifest_and_meta_colors` | two → five icons with three new exact member sets; `"id":"/"` required |
| AM-5 | `remote_brand_contract` const `BUTTON_LABELS` (`test_rmc_s50_*`) | login label "Sign in with Face ID" → "Sign in with your passkey" |
| AM-6 | `remote_push_contract::test_rmc_s38_page_and_service_worker` | `"home-screen app"` → the full, strictly longer `PUSH_UNSUPPORTED_TEXT` |
| AM-7 | `remote_battery_contract::test_rbs_s6_no_new_dependency` | dev-dependency allowlist gains `png` (exact; `[dependencies]` half untouched) |

## 4. Tester Contract

- 27 new tests (commit `7b7f08c`): 11 static in `tests/invariants/src/remote_android_contract.rs`
  (`test_ran_s1_*` … `test_ran_s10_*` and `test_ran_helper_js_lexer_self_test`) and 16 behaviour tests in
  `crates/push-protocol/tests/android_endpoint_tests.rs` (2), `crates/push-sender/tests/android_sender_tests.rs` (1),
  `crates/remote/tests/android_push_tests.rs` (5), `android_webauthn_tests.rs` (3) and `android_assets_tests.rs` (5).
- Mapping: RAN1 `test_ran_s1`, `test_ran_badge_is_white_on_transparent`; RAN2 `test_ran_s2`,
  `test_ran_worker_resubscribe_*`; RAN3 `test_ran_s3`, amended `test_rmc_s53`; RAN4 `test_ran_s4`, the maskable and
  full-bleed pixel tests; RAN5 `test_ran_s5`, `test_ran_assets_are_*`, amended `test_rmc_s54`; RAN6 `test_ran_s6`,
  `test_ran_s10`, amended `test_rmc_s50`/`test_rmc_s38`; RAN7 `test_ran_s7`; RAN8 `test_ran_s8`,
  `test_ran_push_host_is_the_single_source_of_the_allowlist`, `test_ran_push_service_maps_each_host`; RAN9 endpoint,
  sender, subscribe/store/dispatch and Chrome JSON tests; RAN10 the three WebAuthn tests; RAN11 `test_ran_s9`.
- Red evidence: `soos-invariants` 536 passed, 14 failed (exactly `test_ran_s1`..`s10` and the four AM-amended tests);
  the push-protocol and `android_push_tests` files failed to compile on the missing API only (E0432/E0599/E0624);
  the asset tests failed on missing files and `NotFound` routes. The A8 coverage tests (sender, WebAuthn) were green
  by design: they pin behaviour that already accepted Android.
- Blocker found and resolved: the AM-4 literal mixed array lengths (E0308); a type-only fix that keeps every needle
  and assertion (`[&[&str]; 5]`) was committed separately in `2cd5b0c` with orchestrator approval.
- Power check: a scratch reference implementation turned every contract green (`soos-invariants` 550 passed);
  `android_push_tests` and `android_assets_tests` were green 10/10 runs in a row.

## 5. Auditor Constraints

CLEARED with C-1–C-20 (gaps G-1..G-4 closed by C-2, C-5, C-6, C-12). How the main ones are met:

- C-1–C-3 (push path): the `push` listener issues no request and touches no storage;
  `event.waitUntil(self.registration.showNotification(...))` stays unconditional; options add only `renotify`,
  `icon: ICON_PATH`, `badge: BADGE_PATH`.
- C-4–C-6 (re-subscription): one `fetch(` in `sw.js`, inside `postSubscription`, to `SUBSCRIBE_PATH` with exactly
  the page's two headers, `credentials: "same-origin"`, `cache: "no-store"`; same key only (`null` → no request);
  `setTimeout(() => controller.abort(), RESUBSCRIBE_TIMEOUT_MS)` cleared in `.finally(...)`; the response is not
  read; terminal `.catch(() => null)`.
- C-7, C-8: no server-side check, route (other than the four assets) or `http.rs` change; CSP unchanged.
- C-9, C-10: `PushHost` (no serde, exact byte match over `PushHost::ALL`), `PUSH_HOSTS` value and order unchanged;
  `PushService::of` public, three arms, no `_`.
- C-11: four `include_bytes!` arms with `image/png`, four exact `GET`/`HEAD` route arms.
- **C-12 (developer deviation from the spec snippet)**: the spec §4.3 snippet started the 10 s timer only after
  `registerServiceWorker()` resolved, so a hung `register()` would not have been bounded. The auditor constraint wins:
  `readyRegistration` arms the timer first, around the whole registration, and resolves `null` at most 10 s after it
  is called. Every `test_ran_s7` needle is kept.
- C-13, C-14: `enableNotifications` returns with `PUSH_WORKER_FAILED_TEXT` before `pushManager.subscribe(`; the
  `finally` re-enables the button; `serverKeyMatch` returns `"unknown"` before decoding, and only `"different"`
  unsubscribes.
- C-15 (determinism): the four PNGs carry only `IHDR`, `IDAT`, `IEND`; two generation runs (rsvg-convert 2.63.2,
  ImageMagick 7.1.2-32) gave byte-identical files:

  | File | SHA-256 |
  |---|---|
  | `icon-192.png` | `acfd3c22d2ee3f3f1b73f346c456565ce50a4b1d91bf00e9972ebc346cd2c37c` |
  | `icon-512.png` | `1e31cee295371d889f3b9f1d75bab2ef7ff78f052d0ac72a9be6f43c2a6602e4` |
  | `icon-maskable-512.png` | `babe481cbed67a7e3c13dfc4890eba66f0df49c1970d0a23b48b197019eb79a2` |
  | `badge-96.png` | `d798e9b8981e596d9a3aeaf41fd26c331e6935e25b094adf30aefd5f26dfca52` |

- C-16 (supply chain): `png = "=0.18.1"` under `[dev-dependencies]` only; the `Cargo.lock` delta is the single
  `"png"` line in the `soos-remote` dependency list (no new package).
- C-17: `scripts/install_remote.sh` changes comment and `echo` text only.
- C-18, C-19: no new log line or `console.*`; English only; the worker text introduces no `battery`/`camera` word.
- C-20: no test file modified by the developer phase.

## 6. Implementation

- Commit `07e2a6d` (`feat(remote): support android phones in the companion and web push`):
  - `crates/push-protocol/src/lib.rs`: `PushHost`, derived `PUSH_HOSTS`, `PushEndpoint::push_host()`.
  - `crates/remote/src/push.rs`: `PushService::of` public and exhaustive.
  - `crates/remote/src/{assets,routes}.rs`: four embedded PNG assets and their routes.
  - `crates/remote/assets/sw.js`: `renotify`, icon, badge, `pushsubscriptionchange` handler.
  - `crates/remote/assets/app.js`: neutral texts, `PUSH_SERVICES` labels, key tri-state, bounded worker wait (C-12).
  - `crates/remote/assets/index.html`: login label; `manifest.webmanifest`: `id` and five icons; the four PNGs.
  - `crates/remote/Cargo.toml`: `png` dev-dependency (AM-7); `scripts/install_remote.sh`: neutral text.
  - `Docs/REMOTE_COMPANION.md`: new §2h, §2e icon commands, neutral wording, §6 asset list, §9 out-of-scope;
    `AI/DECISIONS.md`: the ADR.
- Traceability (this phase): `AI/VERIFICATION_MATRIX.md` component `remote-android` (RAN1–RAN12) and the RMC84–RMC86
  amendment note; `AI/ARCHITECTURE.md` §13 Android paragraph and the "Page design" row amendment;
  `Docs/REMOTE_COMPANION.md` §2e "Unchanged" bullet made accurate after the Android changes; this walkthrough.

## 7. Candid Review

Pending at the time of writing: the candid review (`./scripts/candid_review.sh`, report `AI/candid_review_report.md`
bound to the diff fingerprint) runs on the final diff, which includes this walkthrough and the RAN matrix rows. Its
verdict and findings are recorded in `AI/candid_review_report.md`.

## 8. Verification Results

All on 2026-10-09 at `07e2a6d` plus this traceability commit:

- `cargo test --locked --all-features -p soos-remote -p soos-push-protocol -p soos-push-sender --no-fail-fast`:
  381 passed, 0 failed (`android_endpoint_tests` 2, `android_sender_tests` 1, `android_push_tests` 5,
  `android_webauthn_tests` 3, `android_assets_tests` 5; every pre-existing suite green).
- `cargo test --locked --all-features -p soos-invariants`: 550 passed, 0 failed, including the 11 `remote_android_contract`
  tests, the amended `test_rmc_s38/s50/s53/s54`, `test_rbs_s6_no_new_dependency` and
  `test_matrix_claimed_rows_cite_only_existing_evidence`.
- `sha256sum crates/remote/assets/{icon-192,icon-512,icon-maskable-512,badge-96}.png`: matches the C-15 table.
- `python3 scripts/sync_issue.py --check`: OK (GitHub-only issue, no backlog sub-issue to tick).

## 9. Known Limitations / Follow-ups

- RAN12 (owner hardware check on an Android phone, `Docs/REMOTE_COMPANION.md` §2h item 10) is pending.
- Whether Chrome mints a WebAPK or a plain standalone shortcut for a tailnet-only `*.ts.net` origin is not verifiable
  without hardware; both open standalone (RAN12 step 1 records which).
- Chrome rarely fires `pushsubscriptionchange`; the page's re-sync on every open stays the main recovery path. A
  renewal is lost until the next open when the PC refuses the worker's request (off-tailnet, expired Funnel session,
  four devices registered).
- Vendor battery savers can delay pushes (documented: set the browser to Unrestricted). Firefox passkey and install
  behaviour depends on the Firefox and Android versions.
- Out of scope: native Android app, FCM direct API keys, Edge desktop push, Android Shortcuts/Tasker integration
  (pointer only).

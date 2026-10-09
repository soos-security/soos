# Plan Evaluation Report
- **Date**: 2026-10-09
- **Issue**: GitHub-only — feat(remote): Android support for the phone companion and Web Push (GitHub #349; no `AI/BACKLOG.md` entry)
- **Branch**: `feat/remote-android`
- **Base commit**: `24d0d8c` (`origin/main`, #348)
- **Spec evaluated**: `AI/architect_spec_remote_android.md` round 1, amended in place by this evaluation (items PE-1..PE-6)
- **Owner decisions in force**: full Android parity (O-1); amendments AM-1..AM-6 approved (O-2). AM-7 (new, PE-4) is **not** covered.

## 1. Coverage Matrix

| Acceptance line (#349) | Spec element | Status |
|---|---|---|
| A1 `renotify`, raster `icon`, monochrome `badge`, no fetch/cache, every push shows | §3.1, §6; `test_ran_s1`, `test_ran_badge_is_white_on_transparent`; RAN1 | Covered (wording corrected, PE-3) |
| A2 `pushsubscriptionchange`: same key, same CSRF + credentials, silent, bounded | §3.2, §3.3; `test_ran_s2`, `test_ran_worker_resubscribe_*` (2); RAN2 | Covered |
| A3 Manifest installable (`id`, 192/512 `any`, 512 `maskable`, deterministic, touch icon kept) | §5, §6; `test_ran_s3`, `test_ran_s4`, two pixel tests, AM-4; RAN3, RAN4 | Covered |
| A4 Four PNG assets embedded, `image/png`, GET/HEAD only, CSP unchanged | §2.3; `test_ran_s5`, `test_ran_assets_*` (2), AM-1/AM-2; RAN5 | Covered |
| A5 Platform-neutral text (passkey phrase, no "iPhone settings", two-case unsupported text) | §4.1, §7; `test_ran_s6`, `test_ran_s10`, AM-5, AM-6; RAN6 | Covered |
| A6 Unknown-key subscription kept; failed SW registration shows an error | §4.2, §4.3; `test_ran_s7`; RAN7 | Covered |
| A7 Exhaustive `PushService`; labels | §2.1, §2.2, §4.4; `test_ran_s8`, `test_ran_push_service_maps_each_host`, `test_ran_push_host_*`; RAN8 | Covered |
| A8 FCM `/fcm/send/`,`/wp/`, Mozilla `/wpush/v1/`,`/wpush/v2/` end to end; Chrome subscription JSON; Chrome clientDataJSON; GPM registration | §11.2 (push-protocol, push-sender, remote push + webauthn tests); RAN9, RAN10 | Covered (coverage contracts, green from start: acceptable, see §3 Pillar 6) |
| A9 Docs Android section, neutral push text, neutral installer | §7.1, §7.2; `test_ran_s9`, `test_ran_s10`; RAN11 | Covered (+ §9 out-of-scope bullet, PE-5) |
| A10 ADR, matrix rows incl. owner Android check, walkthrough | §13, §14; RAN11, RAN12; walkthrough **193** | Covered (number drift verified, see §2) |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Walkthroughs 191/192 taken | `AI/walkthroughs/` | `191_remote_live_camera.md`, `192_remote_battery_status.md`; next is 193 | Yes → **193** |
| Matrix precedent | `AI/VERIFICATION_MATRIX.md` L2177, L2200 | #345 → `RLC1..16`, #346 → `RBS1..12`, dedicated blocks; RMC stops at RMC88 | Settled: **RAN** (PE-1) |
| `PUSH_HOSTS` + path alphabet (`:` `=`) | `push-protocol/src/lib.rs` L24, L119–L126 | as cited | Yes |
| `PushEndpoint.host: &'static str`, `PushService::of` private with `_ => Mozilla` | `push-protocol` L109; `remote/src/push.rs` L997–L1004 | as cited | Yes |
| `check_action_csrf` rules | `routes.rs` L316–L345 | one `X-Soos-Action` == action; `Sec-Fetch-Site` absent/`same-origin`; `Origin` absent/`https://host[:443]` | Yes |
| Funnel gate before routing | `server.rs` L1190–L1205 | non-public route without session → `403 login_required`; `Touch::Refresh` on non-asset routes | Yes (refresh side effect noted, PE-6) |
| `PUSH_SUBSCRIBE_PATH` | `routes.rs` L90 | `/api/push/subscribe` | Yes |
| `ACTION_PUSH_SUBSCRIBE` "parsed from routes.rs" | `crates/remote/src/lib.rs` L425 | defined in **lib.rs**, only imported by routes.rs | **No** → PE-2 fixed |
| `sameServerKey` → unsubscribe on absent key; `await navigator.serviceWorker.ready` unbounded | `app.js` L934–L950, L1018–L1040, L1078 | as cited | Yes |
| `readNotification` tag never empty (`renotify` needs a tag) | `sw.js` L16 | defaults to `soos-alerts`, else `soos-test` | Yes |
| `png 0.18.1` in `Cargo.lock` | `Cargo.lock` L2749 (via `image` ← `arboard`/`eframe` ← `soos-gui`) | 0.18.1; deps `bitflags 2.13.2`, `crc32fast`, `fdeflate`, `flate2`, `miniz_oxide 0.8.9` — all locked | Yes |
| `png` license vs `deny.toml` | `cargo metadata`; `deny.toml` L26–L34 | `MIT OR Apache-2.0` (deps MIT / Apache-2.0 / Zlib) — all allowed; no new duplicate | Yes |
| "`test_rbs_s6_no_new_dependency` stays green" | `remote_battery_contract.rs` L519–L526 | dev-dependency keys must be in `["tokio","tempfile","proptest"]` → `png` turns it **red** | **No** → PE-4 |
| Icon generation deterministic, chunks `IHDR/IDAT/IEND`, sizes 2885/8121/6926/1098 B | Re-run in evaluator scratchpad (rsvg-convert + magick) | two runs byte-identical; IHDR 192²/512²/512² ct 2, 96² ct 6, no interlace; sizes identical; maskable (266,246) `#EDF1FF`, corner `#0047BB`; badge corner alpha 0, (52,44) white opaque | Yes |
| Existing tests not affected (s8, s38 forbidden list, s39, rlc_s8, passkey `.register(`, routes/server asset lists) | invariants + `crates/remote/tests` | non-exhaustive lists / substring checks; still green with the plan | Yes |
| CSP unchanged | `http.rs` | `img-src 'self'`, `connect-src 'self'`, `manifest-src 'self'`, worker via `script-src 'self'` | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: the worker's `pushsubscriptionchange` POST becomes a new way to register an endpoint
  without user authentication. The plan adds no route and changes no check; the request goes through `check_host`,
  `classify_request`, the Funnel session gate and `check_push_csrf` exactly like the page's (verified in code). A
  Funnel caller without session gets `403 login_required` before routing; a cross-site initiator gets `403 forbidden`;
  both are pinned by the new regression test. A same-origin SW fetch can only be issued by code served from the origin
  (CSP `script-src 'self'`, single `/sw.js` registration pinned by the passkey contract).
- Side effect: a worker POST with a still-valid Funnel session refreshes its idle timer without the owner looking at
  the page (one request per browser-initiated event, 8 h absolute cap unchanged).
- Result: PASS, with MINOR PE-6 (documented as accepted risk).

### Pillar 2 — PAM deadline & concurrency
- Not touched: no change in `crates/pam`, daemon or protocol. Client-side waits are bounded (`RESUBSCRIBE_TIMEOUT_MS`
  10 s with `AbortController`; `SERVICE_WORKER_READY_TIMEOUT_MS` 10 s with `clearTimeout`; no retry loop, statically
  checked). Failure scenario "worker hangs on a dead PC" → aborted at 10 s, swallowed by the terminal `.catch`.
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario: `PushService::of` mislabels a fourth host — removed by the exhaustive `PushHost` match (compile
  error on a new host). `renotify: true` with an empty tag would make `showNotification` throw and lose the
  notification: excluded, the tag is never empty. `PushHost::name` is `const fn`; `PUSH_HOSTS` value unchanged and
  pinned by `protocol_tests.rs` L47. No new `unwrap`/error path in production Rust.
- Result: PASS.

### Pillar 4 — Dependencies
- Failure scenario: the `png` dev-dependency is fine for the lock and licences but turns the existing contract test
  `test_rbs_s6_no_new_dependency` red; the plan asserted the opposite. Fixing it requires amending an existing test
  assertion that is outside the owner-approved AM-1..AM-6.
- Result: **FINDING (MAJOR, PE-4)** — resolved in the spec by an explicit AM-7 gate with a binding fallback (see §4).

### Pillar 5 — Data confidentiality
- Notification title/body still come from the existing server payload (counts and classes only); `readNotification`
  is unchanged; icon/badge URLs are constant paths with no identity or alert data. The worker stores nothing, logs
  nothing, never reads the response. No new server log line. `PushEndpoint` `Debug` stays redacted.
- Failure scenario: the ADR's "push path makes no network access" is false in practice — the browser fetches the
  icon and badge from the PC when it displays the notification. Not a leak (Funnel-public static assets, no
  credentials needed), but an inaccurate invariant statement; a failed fetch must not suppress the notification.
- Result: PASS, with MINOR PE-3 (wording corrected, RAN12 step 5b added).

### Pillar 6 — Test integrity
- Amendments AM-1..AM-6 each replace an exact value with an exact value (AM-6 is strictly stronger); none removes a
  check. Verified that no other existing assertion pins the changed text (`Face ID` in docs §2f stays; `home-screen`,
  `16.4`, `web.push.apple.com` stay in §2d).
- Power check: `test_ran_s7` fails against the current `sameServerKey` (unsubscribes on absent key) and the unbounded
  `serviceWorker.ready`; `test_ran_s8` fails against the `_ =>` arm; pixel tests fail on an `#EDF1FF` maskable
  background (corner/safe-zone ring) or an RGB-coloured badge; the regression test fails if `same-site` or a missing
  action header were ever accepted.
- Coverage tests green from the start (endpoint shapes, sender, Chrome JSON, WebAuthn, worker request shape) are
  acceptable because A8 is a tests-only acceptance line with no behaviour change; the tester must label them
  "coverage contract (green at red phase)" in `AI/tester_contract_remote_android.md`. Every behaviour change (A1–A7,
  A9) has at least one red test.
- Test `test_ran_s2` parsed `ACTION_PUSH_SUBSCRIBE` from the wrong file and would have panicked rather than fail
  meaningfully.
- Result: PASS after PE-2 fix.

## 4. Findings

- **[MAJOR] PE-4** `png` dev-dependency breaks `test_rbs_s6_no_new_dependency` (dev-dependency allowlist
  `["tokio","tempfile","proptest"]`, `remote_battery_contract.rs` L519–L526); spec §2.4/§12 claimed it stays green.
  — *Fixed in spec*: §2.4 corrected; §12 adds **AM-7** (exact new value `["tokio","tempfile","proptest","png"]`) marked
  **pending owner approval**, plus a binding fallback if declined/unanswered: no new dependency, the three pixel tests
  move unchanged to `tests/invariants/src/remote_android_contract.rs` with a test-only DEFLATE/PNG decoder that
  verifies the zlib Adler-32 trailer and decoded length (a decoder bug can only make a test red) and self-tests on
  `apple-touch-icon.png`. Supply chain itself is clean (locked 0.18.1, MIT OR Apache-2.0, no lock entry added).
  **Required before Phase 2: orchestrator asks the owner about AM-7 and records the branch taken.**
- **[MINOR] PE-1** Matrix prefix: settled to **`RAN`** (RAN1–RAN12; `test_ran_s1_*`…`test_ran_s10_*`; behaviour
  tests `test_ran_*`), consistent with #345 `RLC` / #346 `RBS`. — *Fixed in spec* (all ids renamed); binding.
- **[MINOR] PE-2** `ACTION_PUSH_SUBSCRIBE` lives in `crates/remote/src/lib.rs` L425, not `routes.rs`. — *Fixed in spec*
  (`test_ran_s2`, §9).
- **[MINOR] PE-3** "The push path makes no network access" is inaccurate: the browser loads `/icon-192.png` and
  `/badge-96.png` on display. — *Fixed in spec* (§3.1 note, RAN1 text, ADR text: "the worker makes no request on
  push"; RAN12 step 5b checks a notification still appears off-tailnet).
- **[MINOR] PE-5** `Docs/REMOTE_COMPANION.md` §9 must list the #349 out-of-scope items. — *Fixed in spec* (§7.1).
- **[MINOR] PE-6** Worker POST over Funnel refreshes a valid session's idle timer. — *Fixed in spec* (§3.3 note, ADR
  accepted risks).

Settled open questions: walkthrough **193** (191/192 verified taken); matrix prefix **RAN**; `png` acceptable on
licence/lock grounds but gated by AM-7; coverage tests green from the start acceptable when labelled.

Security checks requested by the orchestrator: `pushsubscriptionchange` loosens no CSRF/session/Funnel check (PASS);
the worker still never fetches on `push` and every push shows a notification (PASS, tag never empty); CSP unchanged
(PASS, pinned by `test_ran_s5` and the existing CSP tests); no sensitive data in notifications (PASS); icons
deterministic (PASS, reproduced byte-identically by the evaluator).

## 5. Verdict
VALIDATION_VERDICT: APPROVED

The single MAJOR finding (PE-4) is resolved in the spec with two fully specified branches; the orchestrator must
obtain the owner's AM-7 answer (or apply the fallback) before Phase 2. No other revision is required.

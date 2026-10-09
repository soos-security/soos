# Candid Review Report

- **Date**: 2026-10-09
- **Target Branch**: `feat/remote-android`
- **Base (merge-base)**: `24d0d8c`
- **Reviewed-Diff-Fingerprint**: `52583adfe84797adecd30f28aefdd3b79798225a29fc8ce4bd8a15b4498d3047`
- **Audited Files**: AI/ARCHITECTURE.md, AI/DECISIONS.md, AI/VERIFICATION_MATRIX.md,
  AI/architect_spec_remote_android.md, AI/auditor_constraints_android.md,
  AI/tester_contract_remote_android.md, AI/walkthroughs/193_remote_android.md, Cargo.lock,
  Docs/REMOTE_COMPANION.md, crates/push-protocol/src/lib.rs,
  crates/push-protocol/tests/android_endpoint_tests.rs,
  crates/push-sender/tests/android_sender_tests.rs, crates/remote/Cargo.toml,
  crates/remote/assets/{app.js, index.html, manifest.webmanifest, sw.js, badge-96.png,
  icon-192.png, icon-512.png, icon-maskable-512.png}, crates/remote/src/{assets.rs, push.rs,
  routes.rs}, crates/remote/tests/{android_assets_tests.rs, android_push_tests.rs,
  android_webauthn_tests.rs}, scripts/install_remote.sh, tests/invariants/src/{lib.rs,
  remote_android_contract.rs, remote_battery_contract.rs, remote_brand_contract.rs,
  remote_push_contract.rs}

## 1. Executive Summary

Android support for the `soos-remote` companion: four new embedded PNG assets and routes,
an installable manifest (`id`, 192/512/maskable icons), notification `icon`/`badge`/`renotify`,
a bounded single-shot `pushsubscriptionchange` re-subscription in the service worker,
platform-neutral passkey/push text, a bounded `readyRegistration()` wait, unknown-key
subscriptions kept instead of dropped, and an exhaustive `PushHost` enum replacing the
string match (which previously classified any non-Apple/Google host as Mozilla by fallthrough).
No PAM, daemon, policy or protocol-codec code is touched. Production code is small and
mechanically checked; `cargo test` (remote, push-protocol, push-sender, invariants),
`cargo clippy --all-targets -D warnings` and `cargo fmt --check` all pass locally.

## 2. Test Changes

Mechanical listing from the frozen patch:

- New test files: `crates/push-protocol/tests/android_endpoint_tests.rs`,
  `crates/push-sender/tests/android_sender_tests.rs`, `crates/remote/tests/android_*_tests.rs`,
  `tests/invariants/src/remote_android_contract.rs` (additive).
- Removed/changed assertion lines: only patch line 5352
  (`assert_eq!(objects.len(), 2, "exactly two manifest icons")` → `5`) in
  `remote_brand_contract.rs::test_rmc_s53`; the other `^-` hits are documentation/comment lines.
- Other amended assertions: `ASSET_FILES` 7 → 11 and embed count 7 → 11 (`test_rmc_s54`),
  `login-button` label (`test_rmc_s50`), `"id":"/"` added to the manifest needles (`test_rmc_s53`),
  `"home-screen app"` → full `PUSH_UNSUPPORTED_TEXT` (`test_rmc_s38`), `png` added to the
  dev-dependency allowlist (`test_rbs_s6`).
- Escape hatches: only a `tolerance` parameter in a PNG pixel-colour helper (`close()`), used
  for exact-palette matching with small encoder slack; not a weakening of an existing check.
- No `#[ignore]`, no `should_panic`, no inline `mod tests` change.

Justification: every amendment is listed as owner-approved AM-1..AM-7 in
`AI/architect_spec_remote_android.md` §12 and audited in `AI/auditor_constraints_android.md`.
Each replacement is exact and equal or stronger (5 exact icon entries now pinned instead of 2;
full text instead of a substring; the `[dependencies]` set assertion of `test_rbs_s6` untouched).
The `test_rmc_s38` forbidden list (`addEventListener("fetch"`, `caches`, storage, absolute URLs,
`eval`, `innerHTML`) is unchanged and still holds for the new `sw.js`. No weakening found.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tried: unknown push host reaching `PushService::of` → impossible, `PushEndpoint::parse` only
  stores a `PushHost` from `PushHost::ALL`; the match is now exhaustive, removing the old
  silent `_ => Mozilla` fallthrough. `PUSH_HOSTS` is derived from `PushHost::name`, so one
  source of truth. PASS.
- Tried: route/asset mismatch → four routes map to four `AssetId` variants, each with an
  `include_bytes!` and `image/png`; asset set test pins the directory to exactly eleven files. PASS.
- Tried: `readyRegistration()` never settling → timer armed before `register()`, every branch
  calls `settle`, double resolve harmless. PASS.
- Tried: `serverKeyMatch` returning "unknown" for a browser without `options.applicationServerKey`
  → subscription re-posted rather than dropped (previous behaviour silently unsubscribed
  Android Chrome every page open). Correct per spec. PASS.
- Tried: `pushsubscriptionchange` without `newSubscription` after a `soos-remote push reset` →
  the worker re-subscribes with the *old* key and posts it; the PC accepts it but deliveries
  will fail until the page detects "different" on next open. See MINOR finding 1.

### PAM Concurrency & Deadlines
- `crates/pam` untouched. The worker POST is bounded by a 10 s `AbortController` with timer
  cleared in `finally`; one attempt, no retry. PASS.

### Panic Safety & Fail-Closed
- No new `unwrap/expect/panic!/indexing` in production Rust. The service-worker POST goes
  through the existing `push-subscribe` CSRF path (`X-Soos-Action`, `Sec-Fetch-Site`, `Origin`
  same-origin) and the existing session/tailnet gate; a refusal is swallowed and grants
  nothing. No path to unlock or `PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening
- See section 2. New tests exercise exhaustive host classification, PNG dimensions/opacity/
  palette, routes and CSP headers, and sw.js shape; a wrong implementation (e.g. host fallthrough,
  missing route, transparent maskable icon) would fail them. All pass. PASS.

### Memory, Bounds & Secrets
- Assets are compile-time `include_bytes!` (bounded, ~19 KB total). The re-subscription body is
  the browser's `PushSubscription.toJSON()`, bounded server-side by the existing body limit and
  endpoint validation (`MAX_PUSH_ENDPOINT_BYTES`, host allowlist). No password, frame or
  embedding added to any payload or log. PASS.

### Supply Chain & Automation
- `png = "=0.18.1"` is a test-only dev-dependency already locked transitively through `image`;
  `Cargo.lock` gains only a dependency-list entry, no new package. No workflow changes.
  `install_remote.sh` changes are text only. PASS.

### English-Only Policy
- All code, comments, docs and user-facing strings in the diff are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/remote/assets/sw.js` `renewedSubscription()` — when the browser gives no
  `newSubscription`, the worker re-subscribes with the old subscription's
  `applicationServerKey` and posts it without comparing against the PC's current VAPID key. After
  a `soos-remote push reset` this registers a subscription that can never receive pushes until
  the page is reopened (where `serverKeyMatch` drops it). Not a security issue (the push service
  rejects the mismatched VAPID JWT; nothing is disclosed). Optional fix: fetch `/api/push` and use
  its `public_key`, or skip the post when it differs.
- **[SUGGESTION]** `crates/remote/assets/app.js` header comment: two reflowed lines exceed the
  file's usual wrap width; cosmetic only.

## 5. Final Verdict

**VERDICT: APPROVED**

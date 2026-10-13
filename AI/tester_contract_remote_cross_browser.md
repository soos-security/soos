# Tester Contract — soos-remote: browser capability detection and graceful degradation (GitHub #366, PR 1 of #355)

- **Date**: 2026-10-13
- **Branch**: `feat/remote-cross-browser` (GitHub-only issue, not registered in `scripts/sync_issue.py`; `Closes #366`)
- **Spec**: `AI/architect_spec_remote_cross_browser.md` (§8 and the normative §14 R-1 … R-8; plan evaluator round 2
  `VALIDATION_VERDICT: APPROVED`)

## Files

- `tests/invariants/src/remote_cross_browser_contract.rs` (new): 11 contract tests RXB-S1 … RXB-S11 and one helper
  self-test. It has its own JavaScript lexer (strings, template literals and regex literals kept; comments removed),
  a top-level function splitter, a call-argument parser, a top-level declaration-order (TDZ) check and a one-level
  CSS parser. The helpers of the other remote contracts stay private and untouched.
- `tests/invariants/src/lib.rs`: module registration (`#[cfg(all(test, unix))]`).

The project has no JavaScript runtime in CI, so every check is static. Browser behaviour is covered by the owner
hardware rows RXB12–RXB20 (and the later Playwright harness, PR 2 of #355).

## How red was established

`cargo test --locked -p soos-invariants --all-features remote_cross_browser` on the base assets (`1b6d211`):
`3 passed; 9 failed`. Every failure is an assertion or a missing specified item (no compile error):

| Test | Red evidence (first failure message) |
|---|---|
| `test_rxb_s1_single_pure_capability_probe` | `exactly one browserCapabilities()` — left: 0, right: 1 |
| `test_rxb_s2_browser_apis_are_called_behind_the_probe` | `getAssertion: if (!browserCapabilities().webauthn…) { return\|throw\|Promise.reject } must precede the first navigator.credentials` |
| `test_rxb_s3_passkey_controls_degrade_without_webauthn` | `const PASSKEY_UNSUPPORTED_TEXT = "…"; not found` |
| `test_rxb_s4_push_failures_map_to_distinct_fixed_texts` | `const PUSH_HOME_SCREEN_TEXT = "…"; not found` |
| `test_rxb_s5_user_messages_are_fixed_strings` | `function failureText not found` |
| `test_rxb_s7_css_fallbacks` | ``a `@supports not selector(:has(*))` block exists`` |
| `test_rxb_s9_status_without_event_source` | `assertion failed: code.contains("const STATUS_POLL_MS = 15000;")` |
| `test_rxb_s10_camera_degrades_without_streams` | `const CAMERA_UNSUPPORTED_TEXT = "…"; not found` |
| `test_rxb_s11_docs_adr_and_walkthrough` | `## 2i. Browser support` not found |

Green on the base tree, by design (spec §14 R-5, plan evaluator F-7): `test_rxb_helper_self_test` (lexer and parser
self-test), and the regression guards `test_rxb_s6_no_user_agent_logic` and `test_rxb_s8_no_new_network_surface`.
The tree already complies with them. The #355 line "the new invariants fail on the current tree" is carried by
the nine red tests.

Tester-phase correction before hand-off: the first S8 draft rejected `url(` in `app.js` and failed on the base tree
because the lower-cased `fromBase64Url(` contains `url(`. `url(`, `@import` and `@font-face` are now checked in
`style.css` only, where they would load a resource. The other S8 needles still cover the JavaScript files.

## Tester Contract — GitHub #366

| Test (`tests/invariants/src/remote_cross_browser_contract.rs::`) | Acceptance line / matrix ID |
|---|---|
| `test_rxb_helper_self_test` | helper soundness (lexer, splitter, TDZ check, argument parser, CSS parser, units) |
| `test_rxb_s1_single_pure_capability_probe` | "`app.js` has a single pure capability probe" — RXB1 |
| `test_rxb_s2_browser_apis_are_called_behind_the_probe` | "No WebAuthn, Push, Notification, service worker, `ReadableStream` or `createImageBitmap` call happens without passing it"; static "guarded API calls" — RXB2 |
| `test_rxb_s3_passkey_controls_degrade_without_webauthn` | "With `window.PublicKeyCredential` deleted, every passkey button is disabled with a specific fixed message and lock/status still work" — RXB3 |
| `test_rxb_s4_push_failures_map_to_distinct_fixed_texts` | "Push subscribe failures map to distinct fixed messages … `unsupported_push_service` (naming Edge desktop)"; "No path leaves the 'Enable notifications' button disabled for more than the existing 10 s bound" — RXB4 |
| `test_rxb_s5_user_messages_are_fixed_strings` | "messages are fixed strings", "no sensitive data … in the console"; declaration order (no load-time `ReferenceError`, round 2 N-2) — RXB5 |
| `test_rxb_s6_no_user_agent_logic` | "No security decision depends on `navigator.userAgent` or UA client hints … The server never reads `User-Agent`" — RXB6 (regression guard) |
| `test_rxb_s7_css_fallbacks` | "Every `:has()` rule has an `@supports not selector(:has(*))` fallback with the same visual state. `cqw`/`cqh` have an `@supports not (container-type: size)` fallback. `100dvh` is preceded by a `vh` value" — RXB7 |
| `test_rxb_s8_no_new_network_surface` | "CSP … and the asset set stay byte-identical"; "no external URL in the assets"; no CDN or polyfill — RXB8 (regression guard) |
| `test_rxb_s9_status_without_event_source` | SSE degradation (spec D-7) — RXB9 |
| `test_rxb_s10_camera_degrades_without_streams` | "Camera: if `ReadableStream`/`createImageBitmap` are missing, the camera button is hidden with an explanation. The parser bounds stay as they are" — RXB10 |
| `test_rxb_s11_docs_adr_and_walkthrough` | Docs "Browser support" section (iOS home screen, Edge desktop, Brave, private windows, DNS-over-HTTPS vs MagicDNS in Troubleshooting), ADR, matrix rows, walkthrough — RXB11 |

The "existing tests unchanged and green" acceptance line is carried by the existing suites (CSP, headers, CSRF, cookie,
WebAuthn verifier, push allowlist, asset set), which are left untouched.

Phase 4 hand-back correction (developer evidence; it increases precision and weakens nothing): the TDZ helper of
RXB-S5 scanned string literal contents and reported `source` and `camera` from `"/api/camera"`-style literals as
used before their declaration. It now blanks string, template and regex literal contents first, and a self-test
case (`"/api/later"` before `const later`) pins this. The true-positive self-test case (`const A = B;` before
`const B`) still fails as expected.

Phase 4 result: 12 passed, 0 failed.

### Migrated existing tests

None. Spec §9: no existing test is amended. Every existing pin was checked by the plan evaluator: RAN-S6 texts,
the RAN-S7 order and first-await rule, the `remote_push_contract.rs:719` raw-body first-await check, the counts of
`navigator.credentials.get`, `getAssertion(`, `allowCredentials` and `.register(`, the forbidden words, the brand
CSS parser and colour rules, and the battery body checks.

### Assumptions recorded by the tester

- Top-level functions in `app.js` start at column 0 (current style); the splitter relies on it, and the
  `functions.len() > 40` sanity check fails loudly if that changes.
- The guard rule accepts a combined condition (`if (cameraDecoding || !browserCapabilities().imageBitmap)`) as
  long as the guard is negated, sits inside the `if (` condition and the block exits (`return`, `throw`,
  `Promise.reject(`).
- The TDZ check looks at top-level text outside function declarations; a callback inside a top-level statement
  that names a later constant would be reported conservatively (none exists).

### Flakiness check

The tests read files only (no time, no I/O races). 10 runs in a row:
`for i in $(seq 10); do cargo test --locked -p soos-invariants --all-features remote_cross_browser -q || break; done`
gave the same 3 passed / 9 failed result each time on the base tree. The Phase 4 result is recorded in the walkthrough.

# Walkthrough 197 — soos-remote: Browser Capability Detection and Graceful Degradation

- **Date**: 2026-10-13
- **Issue**: GitHub #366 (PR 1 of GitHub #355), no backlog id — **Branch**: `feat/remote-cross-browser`
- **Matrix criteria**: RXB1–RXB11 (verified), RXB12–RXB20 (owner hardware, pending)
- **Registration**: GitHub-only issue, not registered in `scripts/sync_issue.py` (dev-workflow Phase 0 rule); the
  commit carries `Closes #366` and `Refs #355`.
- **Number**: 197 (196 is reserved by the parallel branch `feat/network-lock`).

## 1. Context & Objectives

The owner reported that the `soos-remote` web app "does not work on every browser" (#355, 2026-10-12). The audit
found no hard blocker on modern engines, but several silent failures:

- WebAuthn, the service worker and Push, streamed `fetch` and `createImageBitmap` were called without feature
  detection, so a missing API ended in a generic error or a stuck button;
- browser exception text was shown to the user;
- the `:has()` CSS rules had no fallback, and the portrait camera canvas had none for container units;
- known platform limits looked like generic errors: iOS push outside the home-screen app, Edge desktop's push host,
  Brave without Google push, private windows.

This PR (planned split item 1) adds capability detection, per-feature degradation with fixed messages, CSS
fallbacks, compatibility docs, Rust static invariants and matrix rows. It applies the owner decisions of
2026-10-13:

- **mandatory browsers**: Safari iOS/macOS, Chrome, Firefox and Samsung Internet; best effort with clean
  degradation everywhere else;
- **Edge desktop push** stays out of scope: a clear message, no WNS, no new host;
- **floor**: Baseline widely available, with no ESR 115 commitment;
- **messages**: per-feature only, no capability panel.

The Playwright harness (PR 2) is out of scope: no Node or npm.

## 2. Architect Design

Spec `AI/architect_spec_remote_cross_browser.md` (§0 D-1 … D-10, normative §14 R-1 … R-8):

- **Probe**: `browserCapabilities()` is pure and synchronous, recomputed on every call. It returns `webauthn`,
  `serviceWorker`, `push`, `notifications`, `standalone`, `homeScreen`, `eventSource`, `streams` and `imageBitmap`
  from feature detection only. The page reads no User-Agent at all, even for wording: iOS wording uses
  `"standalone" in navigator`, and the Edge wording comes from the server's existing `unsupported_push_service`.
- **Guards**: every sensitive API call follows a negated guard in an `if` that exits.
- **Fixed messages**: `fixedError(text)` and `failureText(error, names, fallback)`. Name tables:
  `PASSKEY_ERROR_TEXTS` (the `get()`/`create()` errors), `ENROLL_ERROR_TEXTS` (adds `InvalidStateError` →
  "already registered") and `PUSH_ERROR_TEXTS`.
- **Degradation per feature**:
  - passkeys: buttons disabled with `PASSKEY_UNSUPPORTED_TEXT`, handlers return before any request;
  - push: seven new fixed texts, button disabled only after a granted permission, `beforeDeadline` bounds subscribe
    and hand-off by `SERVICE_WORKER_READY_TIMEOUT_MS`, a subscription refused as `unsupported_push_service` is
    dropped;
  - status: a 15 s `GET /api/status` poll without `EventSource`;
  - camera: start hidden with `CAMERA_UNSUPPORTED_TEXT` without streams or decoding.
- **CSS**: the mixed `:has()` selector list is split. `@supports not selector(:has(*))` repeats each `:has()`
  state on hooks set by `app.js` (`data-state` on `.stat-tile`, `feedback-last` via `setLogoutHidden`).
  `@supports not (container-type: size)` sizes the portrait canvas (133.333 % x 75 % in the card, `100vh` x
  `100vw` in full screen).
- **Unchanged**: no Rust production change; `sw.js`, `index.html`, the manifest, the CSP, all headers, CSRF, the
  cookie, the WebAuthn verifier, the push allowlist and the 11-file asset set.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`. Round 1 was REVISION_REQUIRED, with three MAJOR findings:

- **F-1**: the `:has()` rule shared a selector list with `.card > .feedback:last-child`, so an engine without
  `:has()` would drop both;
- **F-2**: the button stayed disabled during a pending permission prompt, which broke the 10 s bound;
- **F-3**: unmapped push failures, such as the PC being unreachable, blamed the browser.

It also raised five MINOR findings: test precision, the shared-deadline wording, the geometry approximations, the
regression guards and the enrollment-only `InvalidStateError`.

Round 2 was **APPROVED** (R-1 … R-7), with two MINOR notes:

- **N-1**: the stale §3.4 prose was marked as superseded;
- **N-2**: declaration order, now pinned by a general top-level TDZ check in RXB-S5.

## 4. Tester Contract

`AI/tester_contract_remote_cross_browser.md`. The new `tests/invariants/src/remote_cross_browser_contract.rs`
holds the self-contained JS lexer and CSS parser, `test_rxb_helper_self_test` and RXB-S1 … RXB-S11.

Red on the base assets: `3 passed; 9 failed`, each on an assertion or a missing specified item (for example
`exactly one browserCapabilities()` left: 0; `function failureText not found`). RXB-S6 and RXB-S8 are regression
guards that were already green on the base tree. 10 consecutive runs gave the same result.

Tester-side corrections, both strengthening precision and neither weakening a test:

- the S8 `url(` needle now applies to `style.css` only, because `fromBase64Url(` lower-cased contains `url(`;
- in Phase 4, the TDZ helper now blanks literal contents, because `"/api/camera"` falsely named `camera`. A
  self-test case pins the fix.

**Migrated existing tests: none.**

## 5. Auditor Constraints

`AI/auditor_constraints_remote_cross_browser.md`, C-1 … C-15, **Clearance: CLEARED**. Each was met as follows:

- **C-1 / C-2** (the probe is pure and guards every API): RXB-S1 and RXB-S2 green.
- **C-3** (no User-Agent): RXB-S6.
- **C-4 … C-6** (fixed strings, no console, TDZ): RXB-S5.
- **C-7 / C-8** (push bound and drop): RXB-S4, with RAN-S7 and push test 48 still green.
- **C-9** (passkeys): RXB-S3.
- **C-10** (polling): RXB-S9.
- **C-11** (camera bounds): RXB-S10 and RLC-S6.
- **C-12**: the tile attribute is filtered to a `LABELS` key in `render`.
- **C-13** (CSS): RXB-S7 and the brand suite.
- **C-14 / C-15**: no Rust production file and no existing test changed. `git diff --stat` lists only assets,
  docs, `tests/invariants/src/lib.rs` (registration) and the new module.

The audit found one pre-existing issue, now fixed: five sites displayed the browser's `error.message`.

## 6. Implementation

- `crates/remote/assets/app.js`:
  - probe, `fixedError`, `failureText`, `beforeDeadline`, `pushUnsupportedText` and `setLogoutHidden`;
  - guards in `getAssertion`, `requestEnroll`, `registerServiceWorker`, `readyRegistration`, `syncSubscription`,
    `enableNotifications`, `disableNotifications`, `openStream`, `readCameraStream` and `showCameraFrame`;
  - passkey-button rules and fixed texts, the status poll fallback, camera degradation and the tile `data-state`
    hook.
- `crates/remote/assets/style.css`: the split `.feedback` rule and two `@supports` fallback blocks appended. Palette,
  tokens and contrast are unchanged.
- `Docs/REMOTE_COMPANION.md`: the new §2i "Browser support" (policy, per-feature table, compatibility matrix with
  verified or pending rows, iOS, Edge, Brave, private windows, secure DNS, owner checks), §7 rows, an §8 residual and
  an §9 pointer.
- `AI/DECISIONS.md`: ADR "[2026-10-13] Browser Support Policy for the `soos-remote` Web App".
- `AI/VERIFICATION_MATRIX.md`: component `remote-cross-browser`, rows RXB1–RXB20.

## 7. Candid Review

Not run in this session: the orchestrator runs Phase 5 (`./scripts/candid_subagent.sh --prepare`, then a fresh
reviewer) before any commit.

## 8. Verification Results

All commands were run in the worktree on 2026-10-13, uncommitted, on top of `1b6d211`:

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --workspace --all-targets --all-features` | pass: 3437 passed, 0 failed, 6 ignored (391 test binaries); `soos-invariants` 583 passed (571 before + the 12 `remote_cross_browser_contract` tests) |
| `cargo test --locked -p soos-invariants --all-features remote_cross_browser` | 12 passed (Red on the base assets: 3 passed, 9 failed) |
| `./scripts/candid_review.sh` (layer 1) | pass ("Candid Review PASSED") |

## 9. Known Limitations / Follow-ups

- **Static evidence only**: there is no automated run in real engines. Playwright (PR 2 of #355) needs an owner
  decision.
- **Owner hardware checks** (`Docs/REMOTE_COMPANION.md` §2i), all pending:
  - RXB12: iPhone tab and third-party iOS browser vs the home-screen app;
  - RXB13: Safari macOS;
  - RXB14: Chrome desktop and Android;
  - RXB15: Firefox desktop, private window and DNS-over-HTTPS;
  - RXB16: Firefox Android;
  - RXB17: Samsung Internet;
  - RXB18: Brave push setting;
  - RXB19: Edge desktop message;
  - RXB20: private windows.
- **Residuals** (§8 of the doc):
  - the flags are page hints only;
  - a broken push service is only detected at subscribe time (bounded 10 s);
  - the 15 s poll refreshes the Funnel idle timer;
  - the container-unit fallback is approximate (capped stage, safe areas, large mobile viewport);
  - pre-Baseline engines are best effort.
- **Accepted**: a second "Enable notifications" tap during the permission prompt can start an identical second
  subscribe, which is idempotent on the PC.

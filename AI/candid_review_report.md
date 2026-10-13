# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `feat/remote-cross-browser`
- **Base (merge-base)**: `bcd3cd2`
- **Reviewed-Diff-Fingerprint**: `1b959d252e5472a39648868e63dbbe45438688b3aa2588507faed45559a23f8c`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_cross_browser.md`,
  `AI/auditor_constraints_remote_cross_browser.md`, `AI/tester_contract_remote_cross_browser.md`,
  `AI/walkthroughs/197_remote_cross_browser.md`, `Docs/REMOTE_COMPANION.md`, `crates/remote/assets/app.js`,
  `crates/remote/assets/style.css`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/remote_cross_browser_contract.rs`

## 1. Executive Summary

The diff is PR 1 of #355 (GitHub #366): a single pure `browserCapabilities()` probe in `app.js`
(feature detection only, no User-Agent), guards before every WebAuthn, service worker, Push,
`Notification`, `EventSource`, stream-reader and `createImageBitmap` use, per-feature
degradation with fixed English strings, `fixedError()`/`failureText()` so a browser's own
exception text is never shown, a push "Enable notifications" tap bounded by
`SERVICE_WORKER_READY_TIMEOUT_MS` after the permission answer, a 15 s status poll when
`EventSource` is missing, CSS fallbacks for `:has()` and container query units, the "Browser
support" docs section (2i) with troubleshooting rows, an ADR, matrix rows and a new static
invariant module (RXB-S1 to RXB-S11).

No Rust production source, server route, CSP, header set, CSRF rule, cookie attribute, WebAuthn
verifier, push host allowlist, `sw.js`, `index.html` or asset file is in the diff. Capability
flags only disable or hide client controls, so no flag can open a server path. Re-run by the
reviewer: `cargo test -p soos-invariants` (591 passed), `cargo test -p soos-remote` (all
green), `node --check crates/remote/assets/app.js` (OK).

Only MINOR and SUGGESTION findings remain.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (adds one `#[cfg(all(test, unix))] mod
  remote_cross_browser_contract;` declaration with a doc comment, 8 added lines, nothing removed)
  and `tests/invariants/src/remote_cross_browser_contract.rs` (new file).
- Removed or changed assertions (`^-` with `assert`/`#[test]`/`proptest!`/`should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`): **none**.
- Inline `mod tests` changes: **none**.

No existing contract was modified. The text pins of earlier contracts (`PUSH_UNSUPPORTED_TEXT`,
`PUSH_BLOCKED_TEXT`, `PUSH_WORKER_FAILED_TEXT`, `PASSKEY_PROMPT_TEXT`, enable-tap ordering) are
kept, and the earlier suites stay green without edits. The changed
`PUSH_REASONS.unsupported_push_service` wording was not pinned by any existing test.

## 3. Deep Reasoning Audit

### Logic & Architecture

- *Probe purity*: `browserCapabilities()` only reads globals (no request, no DOM write, no
  storage) and is recomputed at every call site. `pushSupported()` now delegates to it. PASS.
- *Missing WebAuthn*: `showLogin` disables sign-in and shows `PASSKEY_UNSUPPORTED_TEXT`.
  `showApp` disables enrollment. `render` keeps unlock disabled. `renderCamera` disables start.
  All four handlers return early before `confirm`/`postJson`. Every `.finally` re-enables a
  button only through `!browserCapabilities().webauthn`. Lock, `fetchStatus`, `openStream` and
  `closeStream` never reference WebAuthn. PASS.
- *Push mapping*: `NotAllowedError` gives the blocked text, `AbortError`/`NotSupportedError` the
  no-push-service text, `InvalidStateError` the stale-subscription text and `SecurityError` the
  no-worker text. The server's `unsupported_push_service` names Edge on a computer and drops
  the local subscription on a best-effort basis. iOS in a tab (`"standalone" in navigator`
  without standalone) gets the home-screen text. The acceptance list of #355 is covered. PASS.
- *Enable-tap bound*: the button is disabled only after a granted permission, and the deadline
  is computed before `readyRegistration()`, whose own 10 s timer starts right after. `subscribe`
  and the PC hand-off are wrapped in `beforeDeadline(…, deadline, …)`, which clears its timer
  on both outcomes, and `finally` re-enables the button. Worst case is about 10 s. A late
  `subscribe` success after the deadline is re-sent by `syncSubscription` on the next
  `fetchPush`. PASS.
- *EventSource fallback*: `openStream` arms `setInterval(fetchStatus(true), 15000)` and
  `closeStream` clears it. `resume` still does an immediate `fetchStatus`. Stale rendering
  (45 s) exceeds the poll period. A lock in poll mode is still reported as unconfirmed after
  5 s: FINDING (MINOR, see §4).
- *Hooks for CSS fallbacks*: `statTile` exists in `index.html` (`.stat-tile` inside
  `#status-card`). `data-state` is restricted to known `LABELS` keys, otherwise `unknown`.
  `feedback-last` is toggled with the logout visibility. `#feedback` → `#logout` is the only
  `.card > .feedback` + hidden-last-child pair in the markup, so the fallback covers every case
  the `:has()` rule matched. PASS.
- *Scope*: Playwright (PR 2) is out of scope and documented as a follow-up. The matrix "OK"
  cells cite existing rows (RMC/RLC). New platforms are "Expected (RXB12–RXB20)" pending
  hardware, which complies with "no OK cell without evidence". PASS.

### PAM Concurrency & Deadlines

No change under `crates/pam`, the daemon or any Rust source. The client-side push bound (above)
is the only deadline touched. PASS (not applicable).

### Panic Safety & Fail-Closed

No Rust production change. On the client, every new early exit disables or hides a control, and
none issues a request. Server-side checks (identity headers, CSRF `X-Soos-Action`, `Origin`,
`Sec-Fetch-Site`, WebAuthn UV, session) are byte-identical: `crates/remote/src` is not in the
diff. Attempted scenario: spoof `PublicKeyCredential`/`PushManager` globals to "enable" a path.
The flags only change UI, and every request still meets the unchanged server checks. PASS.

### Test Integrity & Anti-Weakening

§2 shows no removed or weakened assertion. The new module runs on the real assets with its own
lexer (strings, template and regex literals kept intact). Each check can fail against a
plausible wrong implementation:
- an unguarded `navigator.credentials`/`pushManager`/`getReader`/`createImageBitmap` in any
  function, or at top level;
- a `.disabled =` assignment not tied to `browserCapabilities().webauthn`;
- a handler whose guard comes after `confirm`/`postJson`;
- `error.message` shown directly;
- `userAgent` in the page, worker or server;
- a missing `@supports` fallback or `vh` line;
- a CSP or asset-set drift.

A helper self-test covers the lexer. PASS.

### Memory, Bounds & Secrets

- Camera parser bounds (`CAMERA_MAX_BUFFER_BYTES`, `CAMERA_MAX_PART_BYTES`) are unchanged, and
  only their error objects became `fixedError`.
- No passkey material, push endpoint or frame is logged.
- Every message is set through `textContent` (`setText`).
- `failureText` never forwards browser text, and unknown error names fall back to fixed strings.

PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. No polyfill, CDN or
external URL (RXB-S8 asserts it). PASS (not applicable).

### English-Only Policy

Code, comments, user strings, docs, ADR, walkthrough and commit message are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/remote/assets/app.js:600-606` (with `requestLock`, `app.js:857-866`):
  without `EventSource`, status arrives only every 15 s (`STATUS_POLL_MS`). The 5 s
  `LOCK_CONFIRM_UI_MS` timer therefore almost always fires first and shows "The desktop did not
  confirm the lock (LockedHint unchanged)" for a lock that succeeded (unlock confirmation works
  the same way). This affects only pre-Baseline engines and is wording only (the server lock is
  unaffected). Suggested follow-up: in poll mode, call `fetchStatus` once shortly after a 202
  lock/unlock, or note the behavior in `Docs/REMOTE_COMPANION.md:913`.
- **[MINOR]** `crates/remote/assets/style.css:881-890`: the `@supports not (container-type: size)`
  portrait box is approximate. It is clipped when `max-height: 70vh` caps the card stage, and
  in full screen it ignores safe-area padding. This is already listed as a residual in
  `Docs/REMOTE_COMPANION.md` §8 and affects only engines below the agreed Baseline floor. No
  action required.
- **[SUGGESTION]** `crates/remote/assets/app.js:1163-1169`: when `serviceWorker` is exposed but
  `register()` rejects (for example a private-window policy), `readyRegistration()` collapses to
  `null` and shows the generic `PUSH_WORKER_FAILED_TEXT` instead of the private-window sentence.
  That text is pinned byte-exact by RAN-S6, so a future change would need a contract migration.
  The troubleshooting table already covers both sentences.
- **[SUGGESTION]** `crates/remote/assets/app.js:1349-1356`: the button stays enabled while the
  permission prompt is open (by design, to honor the 10 s bound). A double tap can issue two
  `requestPermission` calls. Browsers coalesce these, so the result is harmless.

## 5. Final Verdict

No CRITICAL or MAJOR finding. Security invariants, server surface and existing contracts are
unchanged, and the acceptance criteria of #366 (PR 1 scope of #355) are met.

**VERDICT: APPROVED**

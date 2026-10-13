# Plan Evaluation Report
- **Date**: 2026-10-13
- **Issue**: GitHub-only #366 (PR 1 of #355) — soos-remote browser capability detection and graceful degradation
- **Branch**: `feat/remote-cross-browser`
- **Base commit**: `1b6d211` (`git rev-parse --short origin/main`)
- **Plan**: `AI/architect_spec_remote_cross_browser.md` (revised: V-6, A-2, A-3 corrected; normative §14 R-1..R-7)
- **Round**: 2

Scope of this round: the §14 revisions and the corrected V-6, A-2 and A-3, re-checked against the code at
`1b6d211` and the existing pins. Per the orchestrator, the untracked draft
`tests/invariants/src/remote_cross_browser_contract.rs` is not counted as a plan defect. It is brought in line with
§14 in Phase 2.

## 0. Round 1 findings and their resolution

| Round 1 finding | Revision | Status |
|---|---|---|
| F-1 MAJOR `:has()` selector list also drops `.card > .feedback:last-child` | R-1: rule split; a list never mixes `:has()` with other selectors; RXB-S7 asserts this plus the `HAS_FALLBACKS` table; V-6 corrected | Resolved |
| F-2 MAJOR button disabled during an unbounded permission prompt | R-2: `disabled = true` and `deadline` move after `"granted"`; order and single occurrence asserted; A-2 withdrawn | Resolved |
| F-3 MAJOR browser-blaming fallback for network failures | R-3: `PUSH_UNREACHABLE_TEXT` for a rejected hand-off, neutral fallback "Notifications not enabled", `PUSH_NO_SERVICE_TEXT` only via `AbortError`/`NotSupportedError` | Resolved |
| F-4 MINOR static test power | R-4: negated guard with an exit, `setText(` argument parsing, `"status …"`-only `new Error`, `Math.max(0, deadline - Date.now())` pinned | Resolved |
| F-5 MINOR shared deadline gives a false "no push service" | R-3: subscribe deadline maps to `PUSH_SLOW_TEXT` | Resolved |
| F-6 MINOR container fallback approximations | R-7, A-3: named in the spec and in Docs §8 | Resolved |
| F-7 MINOR RXB-S6/S8 cannot be red | R-5: recorded as regression guards; red evidence carried by S1–S5, S7, S9–S11 | Resolved |
| F-8 MINOR `InvalidStateError` wording on `get()` | R-6: `ENROLL_ERROR_TEXTS` for enrollment only | Resolved (see N-2) |

## 1. Coverage Matrix

| Acceptance line (#355 / #366, Playwright excluded) | Spec element | Status |
|---|---|---|
| Docs "Browser support" section (matrix, iOS home screen, Edge desktop, Brave, private windows, DoH vs MagicDNS, OK cells cite evidence) | §10, RXB-S11, RXB12–RXB20 | Mapped |
| Single pure capability probe | D-1, §3.1, RXB-S1 | Mapped |
| No guarded-API call without the probe | D-2, RXB-S2 + R-4 | Mapped |
| `PublicKeyCredential` deleted ⇒ passkey buttons disabled with a fixed message; lock and status work | D-4, §3.3, RXB-S3 | Mapped |
| Distinct fixed push texts (iOS home screen, no push service / rejected, denied, private window / no SW, `unsupported_push_service` naming Edge) | §3.2, §3.4, R-3, RXB-S4 | Mapped |
| Button never disabled longer than the 10 s bound | R-2, RXB-S4 | Mapped |
| No UA-based decisions; server never reads `User-Agent` | D-3, RXB-S6 (guard, R-5) | Mapped |
| `:has()` fallbacks with the same visual state | R-1, §4, RXB-S7 | Mapped |
| `cqw`/`cqh` fallback; `vh` before `dvh` | §4, A-3, R-7, RXB-S7 | Mapped |
| Brand invariants 63–75 and WCAG AA unchanged | §4, §9 | Mapped |
| CSP, headers, CSRF, cookie, verifier, allowlist, asset set byte-identical | D-10, RXB-S8 | Mapped |
| New static invariants red on the current tree | §8, R-5 | Mapped |
| Matrix rows with the listed hardware checks | §11 | Mapped |
| ADR and walkthrough (197) | §2, §10 | Mapped |

## 2. Facts Verified Against Code (Round 2 deltas)

| Fact / revision | Code location | Actual | Match |
|---|---|---|---|
| R-1 split vs brand parser: one rule per selector list, split on `,`, colour checks only on colour properties | `remote_brand_contract.rs:565-617, 723-737` | `margin-bottom` is not a colour property; two plain rules parse; no `@`-nesting change | Yes |
| R-1 vs `BINDINGS` and other `.feedback` pins | `remote_brand_contract.rs:268-326, 1611-1640` | `.feedback` appears only as HTML id/element pins (`login-feedback`, `feedback`, …); no CSS selector binding on `.feedback` | Yes |
| R-2 vs RAN-S7 (`remote_android_contract.rs:1046-1077`): first `await` is `Notification.requestPermission(`, `await readyRegistration()` < `PUSH_WORKER_FAILED_TEXT` < `pushManager.subscribe(`, `=== null` between, `finally` kept, `PUSH_UNSUPPORTED_TEXT`/`PUSH_BLOCKED_TEXT` present | `app.js:1165-1202` | moving `disabled = true` and `deadline` after `"granted"` adds no `await` before the permission call and keeps the order | Yes |
| R-2 vs `remote_push_contract.rs:719-726` (raw-body first `await`, comments included) | idem | R-2 forbids any comment containing "await" before the permission call | Yes |
| R-2 bound: `readyRegistration` arms its timer at call time, after `deadline` is fixed | `app.js:1006-1035` | ready ends ≤ `deadline` + ε; subscribe and post get `max(0, deadline − now)`; unsubscribe not awaited | Yes |
| R-3: non-200 outcomes still resolve through `postJson` (`.json()` failures caught inside) | `app.js:242-259` | only a transport rejection reaches the new `.catch` → `PUSH_UNREACHABLE_TEXT`; refusals keep `reasonFor(…, PUSH_REASONS, …)` | Yes |
| R-6 `PASSKEY_REASONS.already_registered` | `app.js:150` | `"This passkey is already registered"`; top-level `const` at line 133, before every handler | Yes |
| R-6: `failureText` reads `names` with an own-property check | §3.2 | `Object.assign({}, …)` creates own properties | Yes |
| New texts vs RAN-S6 / RMC forbidden words (`Face ID` suffix, `Touch ID`, `iPhone settings`, `Passwords app`, `https://`, `conditional`, `blob:`) | `PUSH_UNREACHABLE_TEXT`, `PUSH_SLOW_TEXT` | none present | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Scenario: a capability flag or a client text opens a server path. No Rust change (D-10). Every server check is
  unchanged. Flags only hide or disable client controls.
- Result: PASS.

### Pillar 2 — Deadlines & concurrency
- Scenario: the permission prompt stays parked. The button is no longer disabled during the prompt (R-2), so no path
  exceeds `SERVICE_WORKER_READY_TIMEOUT_MS` + timer slack.
- Scenario: a second tap during the prompt. Two flows run, and the first `finally` re-enables the button while the
  second is still running. The browser returns the same subscription, so the PC receives at most two identical
  subscribe POSTs (the replacement path is already covered by `push_server_tests.rs`). Harmless. Each flow is
  bounded on its own.
- Result: PASS.

### Pillar 3 — Fail-closed / no stuck state
- Scenario: the PC sleeps during the hand-off. The `.catch` maps the rejection to `PUSH_UNREACHABLE_TEXT`, not the
  Brave text (R-3).
- Scenario: `subscribe` throws `TypeError`. The neutral fallback applies.
- Scenario: an engine without `:has()`. All last-child feedbacks keep their margin from the `:has()`-free rule, and
  `#feedback` gets `.feedback-last` (R-1).
- Scenario: the page fails at load (see N-2). `ENROLL_ERROR_TEXTS` is a top-level `const` evaluated at load. If it is
  placed above the `PASSKEY_ERROR_TEXTS` declaration, the script throws a `ReferenceError` (temporal dead zone) and
  the whole page, lock included, stops working. No planned test pins the declaration order. → MINOR N-2.
- Result: PASS with MINOR N-2.

### Pillar 4 — Dependencies
- No polyfill, CDN, npm or new asset. Result: PASS.

### Pillar 5 — Data confidentiality
- All texts are fixed. `.message` is read only inside `failureText`, and only for `fixed` errors. There is no
  `console`. Result: PASS.

### Pillar 6 — Test integrity & power
- No existing test is amended. Every listed pin was re-verified against R-1..R-7.
- With R-4 in place, a wrong implementation (guard computed but not enforced, absolute-epoch delay, a raw
  `error.message` display, a leftover user-text `new Error`) now fails a planned assertion.
- §3.4 still carries pre-revision prose: the subscribe wrapper with `PUSH_NO_SERVICE_TEXT`, a bare
  `beforeDeadline(postJson…)`, and "The permission prompt itself is user-paced (A-2)". The test table in §8 for
  RXB-S5 still names only `PASSKEY_ERROR_TEXTS`. §14 is declared normative and supersedes them, but a tester reading
  §3.4 alone could pin the stale shape. → MINOR N-1.
- Result: PASS with MINOR N-1.

## 4. Findings

- **[MINOR] N-1 — stale prose in §3.4 and §8 contradicts §14.** §3.4 still describes subscribe →
  `PUSH_NO_SERVICE_TEXT`, the unwrapped post and the user-paced prompt (A-2), and the §8 RXB-S5 row still names only
  `PASSKEY_ERROR_TEXTS`. §14 supersedes them. Phase 2 must pin the §14 shapes (`PUSH_SLOW_TEXT`,
  `.catch → PUSH_UNREACHABLE_TEXT`, the R-2 order, `ENROLL_ERROR_TEXTS`). Editing §3.4 inline or marking it "superseded by §14" is recommended.
- **[MINOR] N-2 — declaration order of `ENROLL_ERROR_TEXTS`.** It must follow both `PASSKEY_REASONS` and
  `PASSKEY_ERROR_TEXTS` in the file (top-level `const`, temporal dead zone at load). Otherwise the whole page,
  lock and status included, fails to start. RXB-S5 (or S3) should assert the declaration order.

No CRITICAL or MAJOR finding remains.

## 5. Verdict
The Round 1 MAJOR findings F-1, F-2 and F-3 are resolved by R-1, R-2 and R-3 without breaking any existing pin. The two
remaining MINOR items are for the tester and developer phases to handle.

VALIDATION_VERDICT: APPROVED
